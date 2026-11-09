#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""One bounded smoke attempt; not a complete Stage-0 controller.

The caller must first validate an independently approved smoke plan and arm an
independent provider backstop. This helper does not classify a case, verify
milestones, establish a box barrier or authorize any provider action.
"""
import json
import os
import time
from pathlib import Path

import case_launcher
import execution_policy
import result_writer
import stage0_cgroup
import stage0_cleanup
import stage0_controller
import stage0_device
import stage0_docker
import stage0_process
import stage0_settlement

HERE = Path(__file__).resolve().parent


def validate_dispatch(manifest, case):
    """Restrict the successful-exit settlement assumption to the known G1 dispatch."""
    try:
        if case != 'BC-GPU-001' or manifest['lane'] != 'G1' or manifest['harness_repo'] != str(HERE.parents[2]):
            raise ValueError('unsupported smoke dispatch')
        entry = manifest['cases'][case]
        launcher = str(HERE / 'case_entry.sh')
        command = [launcher, '--case', case, '--lane', 'g1',
                   '--candidate', manifest['candidate']['path'], '--baseline', manifest['baseline']['path'],
                   '--manifest', manifest['lane_manifest']['path'], '--out', entry['out']]
        if entry['launcher']['path'] != launcher or entry['command'] != command:
            raise ValueError('unsupported smoke dispatch')
    except (KeyError, TypeError) as exc:
        raise ValueError('unsupported smoke dispatch') from exc


def _receipt(path, value):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'w', encoding='utf-8') as stream:
        json.dump(value, stream, sort_keys=True)
        stream.write('\n'); stream.flush(); os.fsync(stream.fileno())
    dfd = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
    try: os.fsync(dfd)
    finally: os.close(dfd)


def run(item, *, candidate, expectations_sha256, smoke_root, cgroup_root, expected_gpu, forwarded,
        clock=time.monotonic, pause=time.sleep):
    """Launch the real frozen case with a bounded cgroup and reconcile its resources.

    Caller must stop the box on *any* exception, timeout, cleanup ambiguity or
    disallowed terminal outcome. Reconciliation finding a surviving process or
    container is a failure even if it is subsequently removed.
    """
    started = clock()
    if (not isinstance(forwarded, dict) or set(forwarded) - execution_policy.FORWARDED
            or any(not isinstance(k, str) or not isinstance(v, str) for k, v in forwarded.items())
            or not forwarded.get('CUDA_VISIBLE_DEVICES')):
        raise ValueError('explicit limited GPU execution environment required')
    root = Path(smoke_root)
    if not root.is_absolute() or root.is_symlink() or root.resolve(strict=True) != root:
        raise ValueError('canonical smoke root required')
    origin = result_writer.smoke_origin(root)
    if origin is None or origin[0] != root or origin[1]['expectations_sha256'] != expectations_sha256:
        raise ValueError('smoke root is not bound to approved expectations')
    manifest = stage0_controller.pinned_manifest(item, candidate)
    case = item['id']
    output = Path(manifest['cases'][case]['out'])
    if (not output.is_absolute() or output.resolve() != output or output == root
            or not output.is_relative_to(root) or output.exists() or not output.parent.is_dir()):
        raise ValueError('fresh smoke attempt output required')
    control = output.with_name(output.name + '-stage0-control')
    lifecycle = output.with_name(output.name + '-lifecycle')
    if control.exists() or control.is_symlink() or lifecycle.exists() or lifecycle.is_symlink():
        raise ValueError('attempt control or lifecycle already exists')
    cap, grace = item['cap_seconds'], item['grace_seconds']
    if type(cap) is not int or type(grace) is not int or not 0 < grace < cap:
        raise ValueError('invalid case time cap')
    deadline = started + cap
    validate_dispatch(manifest, case)
    case_launcher.preflight(manifest)
    device = stage0_device.probe(manifest['tools'], device=forwarded['CUDA_VISIBLE_DEVICES'],
                                 expected_name=expected_gpu, deadline=deadline, clock=clock)
    control.mkdir(mode=0o700)
    parent_fd = os.open(control.parent, os.O_RDONLY | os.O_DIRECTORY)
    try: os.fsync(parent_fd)
    finally: os.close(parent_fd)
    home = control / 'home'; home.mkdir(mode=0o700)
    env = dict(forwarded)
    env.update({'PATH': '/nonexistent', 'HOME': str(home), 'PYTHONNOUSERSITE': '1',
                'PYTHONDONTWRITEBYTECODE': '1'})
    python = manifest['tools']['python3']['path']
    group = stage0_cgroup.create(cgroup_root)
    pending, outcome = None, None
    try:
        _receipt(control / 'cgroup.json', {'parent': str(Path(cgroup_root)), 'name': group.name})
        _receipt(control / 'gpu-before.json', device)
        # Reserve the declared cleanup grace after the launcher has stopped.
        process_cap = deadline - grace - clock()
        signal_grace = min(5.0, process_cap / 2)
        if signal_grace <= 0:
            raise ValueError('case cap exhausted before launch')
        command = [python, '-I', '-B', str(HERE / 'stage0_cgroup_exec.py'), str(group.join_fd), '--',
                   python, '-B', str(HERE / 'case_launcher.py'), '--manifest', item['manifest']['path']]
        with (control / 'stdout.log').open('xb') as stdout, (control / 'stderr.log').open('xb') as stderr:
            outcome = stage0_process.run_bounded(command, cwd=HERE.parents[2], env=env, stdout=stdout, stderr=stderr,
                cap_seconds=process_cap, grace_seconds=signal_grace, pass_fds=(group.join_fd,))
    except BaseException as exc:
        pending = exc
    errors, group_empty = [], False
    try:
        leaked = group.populated()
        if leaked:
            group.kill()
        while group.populated() and clock() < deadline:
            pause(min(0.1, max(0.0, deadline - clock())))
        if group.populated():
            raise ValueError('owned cgroup remains populated')
        group.close_if_empty()
        group_empty = True
        if leaked:
            errors.append('owned process survived launcher')
    except BaseException as exc:
        errors.append(type(exc).__name__)
    # Never infer settled creation from exit or absent inventory alone. Only
    # the exact GPU-001 dispatch with successful return, no owned processes,
    # and an explicit matched create/inspect receipt can close creation. This
    # remains conservative for every failed or interrupted launcher.
    creation_closed = False
    if (group_empty and pending is None and isinstance(outcome, stage0_process.ProcessOutcome)
            and outcome.timed_out is False and type(outcome.returncode) is int and outcome.returncode == 0):
        try:
            creation_closed = stage0_settlement.successful_gpu001_create(
                output, gpu_name=expected_gpu, gpu_uuid=forwarded['CUDA_VISIBLE_DEVICES']) is True
        except BaseException as exc:
            errors.append(type(exc).__name__)
    if not creation_closed:
        errors.append('container creation settlement is unproven')
    roles = ['server']
    if case in {'BC-PERF-001', 'BC-PERF-002', 'BC-TELEM-001'}:
        roles.append('auxiliary')
    for role in roles:
        try:
            owner = stage0_cleanup.read_owner(output, role)
            docker_home = control / ('docker-' + role); docker_home.mkdir(mode=0o700)
            adapter = stage0_docker.Docker(manifest['tools']['docker'], docker_home, owner=owner,
                                           deadline=deadline, clock=clock)
            if stage0_cleanup.reconcile(owner, adapter, creation_closed=creation_closed) != 'clean':
                errors.append('owned container survived launcher')
            if clock() > deadline:
                errors.append('case cleanup exceeded its cap')
        except BaseException as exc:
            errors.append(type(exc).__name__)
    try:
        if result_writer.smoke_origin(root) != origin:
            raise ValueError('smoke root changed during attempt')
        stage0_controller.pinned_manifest(item, candidate)
        after = stage0_device.probe(manifest['tools'], device=forwarded['CUDA_VISIBLE_DEVICES'],
                                    expected_name=expected_gpu, deadline=deadline, clock=clock)
        if after != device:
            raise ValueError('GPU identity changed during attempt')
        _receipt(control / 'gpu-after.json', after)
    except BaseException as exc:
        errors.append(type(exc).__name__)
    if errors:
        raise RuntimeError('owned-resource cleanup was not verified: ' + ', '.join(errors)) from pending
    if pending is not None:
        raise pending
    return outcome
