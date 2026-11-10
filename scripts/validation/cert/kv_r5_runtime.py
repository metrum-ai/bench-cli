#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Owned single-box r5 execution; no provisioning, downloads or source builds.

This entry consumes a separately reviewed/frozen manifest. It is not registered
as an r4 lane and never changes r4 selection or validator semantics.
"""
import argparse
import csv
import datetime as dt
import hashlib
import http.client
import json
import math
import os
import re
import secrets
import signal
import subprocess
import sys
import time
from pathlib import Path

import g6_checks
import l1_runner
import manifest_tool
import r4_image_provenance
import result_writer

HERE = Path(__file__).resolve().parent
SHA = re.compile('[0-9a-f]{64}\\Z')


def load(path):
    return json.loads(Path(path).read_text(), object_pairs_hook=manifest_tool.unique_object)


def save(path, value):
    with Path(path).open('x') as stream:
        json.dump(value, stream, indent=2, allow_nan=False)
        stream.write('\n')


def pin(item):
    path = Path(item['path'])
    if not path.is_absolute() or not SHA.fullmatch(item['sha256']) or manifest_tool.sha256_file(path) != item['sha256']:
        raise ValueError('frozen file hash mismatch')
    return path


def validate(data, family):
    if data['schema_version'] != 'kv-r5-execution.v1' or data['case'] not in family.CASES:
        raise ValueError('unsupported r5 case')
    if not manifest_tool.REV.fullmatch(data['candidate_sha']):
        raise ValueError('full candidate SHA required')
    import execution_policy
    execution_policy.validate_case_tools(data['tools'], 'KVV')
    for item in [data['candidate'], data['oci_index'], data['oci_child'], *data['inputs'].values(), *data['tools'].values(), *data['model_files']]:
        pin(item)
    model_root = Path(data['model_snapshot']).resolve()
    if not data['model_files'] or any(not pin(p).resolve().is_relative_to(model_root) for p in data['model_files']):
        raise ValueError('model snapshot file lock required')
    for tool in ('docker', 'nvidia-smi', 'python3'):
        if tool not in data['tools'] or not os.access(pin(data['tools'][tool]), os.X_OK):
            raise ValueError('pinned executable tool required')
    required_harness = ('kv_r5_runtime.py', 'kv_r5_metrics.py', 'kv_r5_proxy.py',
                        'trace_proxy.py', 'g6_checks.py', 'metrics_oracle.py',
                        'l1_runner.py', 'manifest_tool.py', 'result_writer.py',
                        'execution_evidence.py', 'r4_image_provenance.py')
    pinned_paths = {str(pin(item).resolve()) for item in data['inputs'].values()}
    if any(str(HERE/name) not in pinned_paths for name in required_harness):
        raise ValueError('r5 harness closure must be frozen')
    if not os.access(pin(data['candidate']), os.X_OK):
        raise ValueError('candidate is not executable')
    if not re.fullmatch('GPU-[0-9a-fA-F-]+', data['gpu_uuid']):
        raise ValueError('one observed GPU UUID required')
    ports = [data['port'], data['proxy_port'], data['metrics_port']]
    if any(type(p) is not int or not 1024 <= p <= 65535 for p in ports) or len(set(ports[:2])) != 2:
        raise ValueError('invalid loopback ports')
    if data['image'] != family.IMAGE or data['source_revision'] != family.REVISION:
        raise ValueError('image/source profile mismatch')
    r4_image_provenance.descriptor_chain(pin(data['oci_index']), pin(data['oci_child']),
                                        index_digest=family.IMAGE.split('@')[1], child_digest=family.CHILD)
    if type(data['timeout_seconds']) is not int or not 0 < data['timeout_seconds'] <= family.LIMITS[data['case']]:
        raise ValueError('case time envelope exceeded')
    if not data['source_files'] or not all(SHA.fullmatch(value) for value in data['source_files'].values()):
        raise ValueError('independently reviewed runtime source file pins required')
    if not data['phases'] or len({p['name'] for p in data['phases']}) != len(data['phases']):
        raise ValueError('unique ordered phases required')
    for phase in data['phases']:
        if not re.fullmatch('[a-z][a-z0-9_-]{0,40}', phase['name']):
            raise ValueError('unsafe phase name')
        if not isinstance(phase['argv'], list) or not all(isinstance(a, str) for a in phase['argv']):
            raise ValueError('frozen product argv required')
        if phase['argv'][0] != data['candidate']['path'] or not phase['expected_requests']:
            raise ValueError('phase must invoke the measured candidate and freeze wire order')
        required_options = {'--url': 'http://127.0.0.1:'+str(data['proxy_port'])+'/v1/chat/completions',
                            '--model': data['served_model'], '--csv': '{out}/requests.csv',
                            '--html': '{out}/report.html', '--ndjson': '{out}/events.ndjson', '--sweep': '1'}
        for key, value in required_options.items():
            if phase['argv'].count(key) != 1 or phase['argv'][phase['argv'].index(key)+1] != value:
                raise ValueError('product endpoint/output argv mismatch')
        if any(a.startswith(key+'=') for key in required_options for a in phase['argv']):
            raise ValueError('ambiguous product option')
        # Every external workload/config file in argv must also appear in inputs.
        known = {v['path'] for v in data['inputs'].values()} | {data['candidate']['path']}
        for arg in phase['argv']:
            if arg.startswith('/') and '{out}' not in arg and arg not in known:
                raise ValueError('unfrozen file argument')
    family.validate(data)


class Runtime:
    def __init__(self, data, out, family):
        self.data, self.out, self.family = data, Path(out), family
        self.owner = secrets.token_hex(16)
        self.name = 'cert-kv-' + secrets.token_hex(12)
        self.total_deadline = time.monotonic() + data['timeout_seconds']
        self.deadline = self.total_deadline - 120  # cleanup belongs inside the case envelope
        self.started = False
        self.proxy = None
        self.container = None

    def command(self, argv, timeout=30):
        remaining = self.deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError('case deadline reached')
        return subprocess.run(argv, check=True, capture_output=True, text=True,
                              timeout=min(timeout, remaining), env=self.env())

    def env(self):
        return {'PATH': os.defpath, 'HOME': str(self.out), 'LANG': 'C.UTF-8', 'HF_HUB_OFFLINE': '1'}

    def docker(self, *args, timeout=30):
        return self.command([self.data['tools']['docker']['path'], *args], timeout)

    def inspect(self):
        value = json.loads(self.docker('inspect', self.name).stdout)
        if len(value) != 1 or value[0]['Config']['Labels'].get('metrum.cert.owner') != self.owner:
            raise ValueError('container ownership mismatch')
        return value[0]

    def get(self, port, path):
        conn = http.client.HTTPConnection('127.0.0.1', port, timeout=min(5, max(.1, self.deadline-time.monotonic())))
        try:
            conn.request('GET', path)
            response = conn.getresponse()
            raw = response.read(4 * 1024 * 1024 + 1)
            if response.status != 200 or len(raw) > 4 * 1024 * 1024:
                raise ValueError('bounded loopback fetch failed')
            return raw.decode('utf-8')
        finally:
            conn.close()

    def start(self):
        d = self.data
        gpu = self.command([d['tools']['nvidia-smi']['path'], '--id='+d['gpu_uuid'],
                            '--query-gpu=uuid,name,memory.total,driver_version', '--format=csv,noheader,nounits']).stdout.strip()
        save(self.out/'gpu.json', {'observed': gpu})
        fields = [v.strip() for v in gpu.split(',')]
        if len(fields) != 4 or fields[0] != d['gpu_uuid'] or fields[1] != d['gpu_class']:
            raise ValueError('actual GPU binding mismatch')
        if not ('H100' in fields[1] or 'RTX PRO 6000 Blackwell' in fields[1]):
            raise ValueError('unsupported GPU class')
        if int(fields[2]) < d['minimum_gpu_mib']:
            raise ValueError('GPU fit envelope mismatch')
        self.driver = fields[3]
        image = json.loads(self.docker('image', 'inspect', d['image']).stdout)[0]
        receipt = r4_image_provenance.verify(pin(d['oci_index']), pin(d['oci_child']), image,
                    index_digest=self.family.IMAGE.split('@')[1], child_digest=self.family.CHILD)
        save(self.out/'image-preflight.json', receipt)
        save(self.out/'ownership.json', {'name': self.name, 'owner': self.owner})
        argv = ['run', '-d', '--pull=never', '--name', self.name,
                '--label', 'metrum.cert.owner='+self.owner, '--gpus', 'device='+d['gpu_uuid'],
                '--ipc=host', '--env', 'HF_HUB_OFFLINE=1', '--env', 'TRANSFORMERS_OFFLINE=1',
                '--mount', 'type=bind,src='+d['model_snapshot']+',dst=/model,readonly',
                '-p', '127.0.0.1:'+str(d['port'])+':8000']
        if d['metrics_internal_port'] != 8000:
            argv += ['-p', '127.0.0.1:'+str(d['metrics_port'])+':'+str(d['metrics_internal_port'])]
        argv += self.family.server(d)
        save(self.out/'server-argv.json', argv)
        # Arm cleanup before Docker is allowed to create anything.
        self.started = True
        self.docker(*argv, timeout=60)
        self.container = self.inspect()
        receipt = r4_image_provenance.verify(pin(d['oci_index']), pin(d['oci_child']), image, self.container,
                    index_digest=self.family.IMAGE.split('@')[1], child_digest=self.family.CHILD)
        save(self.out/'image-runtime.json', receipt)
        # Hash the actual importable source in this exact running image. Labels
        # and version strings alone cannot establish a source slice match.
        code = ('import hashlib,json; files='+repr(d['source_files'])+'; '
                'print(json.dumps({p:hashlib.sha256(open(p,"rb").read()).hexdigest() for p in files}))')
        actual = json.loads(self.docker('exec', self.name, 'python3', '-c', code).stdout)
        save(self.out/'runtime-source.json', actual)
        if actual != d['source_files']:
            raise ValueError('runtime source bytes differ from reviewed source')
        self.family.preflight(self)
        while True:
            try:
                models = json.loads(self.get(d['port'], '/v1/models'))
                if not any(x.get('id') == d['served_model'] for x in models.get('data', [])):
                    raise ValueError('served model mismatch')
                save(self.out/'models.json', models)
                break
            except (OSError, http.client.HTTPException):
                if not self.inspect()['State']['Running']:
                    raise ValueError('server exited during readiness')
                if time.monotonic() >= self.deadline:
                    raise TimeoutError('server readiness deadline reached')
                time.sleep(.2)

    def run(self):
        d = self.data
        phases = []
        for phase in d['phases']:
            directory = self.out/phase['name']
            directory.mkdir()
            with (directory/'proxy.stdout').open('x') as stdout, (directory/'proxy.stderr').open('x') as stderr:
                self.proxy = subprocess.Popen([d['tools']['python3']['path'], str(HERE/'kv_r5_proxy.py'),
                    '--listen-port', str(d['proxy_port']), '--upstream-port', str(d['port']),
                    '--out', str(directory/'trace.jsonl'), '--record-request-identity'],
                    stdout=stdout, stderr=stderr, env=self.env())
            try:
                import g6_counter_runner
                g6_counter_runner.wait_for_proxy(d['proxy_port'], self.proxy)
                if phase.get('reset_prefix_cache') is True:
                    if not getattr(self.family, 'ALLOW_RESET', False):
                        raise ValueError('reset is unsupported for this engine')
                    conn = http.client.HTTPConnection('127.0.0.1', d['port'], timeout=5)
                    try:
                        conn.request('POST', '/reset_prefix_cache')
                        response = conn.getresponse()
                        raw = response.read(1024)
                        if response.status != 200:
                            raise ValueError('prefix reset failed')
                        save(directory/'reset.json', {'status': response.status, 'body': raw.decode()})
                    finally:
                        conn.close()
                before = self.get(d['metrics_port'], self.family.METRICS_PATH)
                (directory/'before.prom').write_text(before)
                argv = [a.replace('{out}', str(directory)) for a in phase['argv']]
                save(directory/'argv.json', argv)
                code = l1_runner.call(argv, directory/'stdout.json', directory/'stderr.txt',
                    max(.1, self.deadline-time.monotonic()), env=self.env())
                if code != 0:
                    raise ValueError('candidate phase failed')
                if hasattr(self.family, 'after_phase'):
                    self.family.after_phase(self, phase, directory)
                after = self.get(d['metrics_port'], self.family.METRICS_PATH)
                (directory/'after.prom').write_text(after)
                current = self.inspect()
                if (current['Id'], current['State']['StartedAt'], current['RestartCount']) != (
                    self.container['Id'], self.container['State']['StartedAt'], self.container['RestartCount']):
                    raise ValueError('server epoch changed')
                phases.append({'plan': phase, 'directory': directory, 'before': before, 'after': after})
            finally:
                self.proxy.terminate()
                self.proxy.wait(timeout=10)
                self.proxy = None
        return self.family.evaluate(d, phases)

    def cleanup(self):
        # Cleanup has its own bounded time so an exhausted workload deadline
        # cannot prevent scoped removal. Never enumerate or stop other owners.
        tool = self.data['tools']['docker']['path']
        def call(*args):
            return subprocess.run([tool, *args], capture_output=True, text=True,
                                  timeout=min(30, max(.1, self.total_deadline-time.monotonic())), check=True)
        if self.proxy is not None:
            self.proxy.terminate()
            self.proxy.wait(timeout=10)
        if self.started:
            value = json.loads(call('inspect', self.name).stdout)[0]
            if value['Config']['Labels'].get('metrum.cert.owner') != self.owner:
                raise ValueError('cleanup ownership mismatch')
            (self.out/'server.log').write_text(call('logs', self.name).stdout)
            call('rm', '-f', self.name)
            remaining = call('ps', '-aq', '--filter', 'label=metrum.cert.owner='+self.owner).stdout.strip()
            if remaining:
                raise ValueError('owned container remains after cleanup')
        save(self.out/'cleanup.json', {'owned_containers_remaining': 0, 'owner': self.owner})


def wire(phase):
    trace = (phase['directory']/'trace.jsonl').read_text()
    events = [json.loads(line) for line in trace.splitlines()]
    identities = [e for e in events if e['kind'] == 'request_identity']
    expected = [{key: r[key] for key in ('messages_sha256', 'options')}
                for r in phase['plan']['expected_requests']]
    actual = [{'messages_sha256': e['messages_sha256'], 'options': e['options']} for e in identities]
    if actual != expected or len({e['request'] for e in identities}) != len(expected):
        raise ValueError('wire order or request settings differ from frozen workload')
    usage = g6_checks.server_usage(trace)
    if len(usage) != len(expected):
        raise ValueError('missing or duplicated responses')
    if any(e.get('status') != 200 for e in events if e['kind'] == 'headers'):
        raise ValueError('server request failed')
    return usage


def product_rows(phase):
    """Reconcile real product CSV rows with the frozen wire order and usage."""
    usage = wire(phase)
    with (phase['directory']/'requests.csv').open(newline='') as stream:
        rows = list(csv.DictReader(stream))
    if len(rows) != len(usage):
        raise ValueError('product request/trace count mismatch')
    if len({row['seq'] for row in rows}) != len(rows):
        raise ValueError('duplicate product request ID')
    rows.sort(key=lambda row: int(row['seq']))
    for row, request, server in zip(rows, phase['plan']['expected_requests'], usage):
        if row['success'] != 'true' or row.get('valid') == 'false' or row.get('error'):
            raise ValueError('unsuccessful product request')
        if row['warmup'] != str(request['warmup']).lower():
            raise ValueError('warmup boundary mismatch')
        if (row.get('session_id') or None, row.get('turn') or None) != (
            request['session_id'], None if request['turn'] is None else str(request['turn'])):
            raise ValueError('session/turn boundary mismatch')
        if int(row['input_tokens']) != server['prompt_tokens'] or int(row['output_tokens']) != server['completion_tokens']:
            raise ValueError('product/server usage mismatch')
        if row['ttft_source'] != 'stream' or not math.isfinite(float(row['ttft_s'])) or float(row['ttft_s']) < 0:
            raise ValueError('stream TTFT with provenance required')
    return rows, usage


def adapter_preflight(manifest, family, lane):
    """Generic launcher hook, deliberately separate from r4 manifest preflight."""
    import build_roles
    import harness_integrity
    if manifest.get('schema_version') != 'kv-r5-frozen.v1' or manifest.get('lane') != lane:
        raise ValueError('KV frozen envelope/lane mismatch')
    cases = manifest['cases']
    if len(cases) != 1 or next(iter(cases)) not in family.CASES:
        raise ValueError('one registered KV case required')
    for item in manifest['inputs']:
        pin(item)
    spec_path = pin(manifest['lane_manifest'])
    spec = load(spec_path)
    validate(spec, family)
    if spec['case'] != next(iter(cases)) or spec['candidate_sha'] != manifest['candidate_sha']:
        raise ValueError('KV selection/candidate binding changed')
    measured, _ = build_roles.load_receipt(pin(manifest['build_receipt']), manifest['candidate_sha'])
    matching = [item for item in measured if item['path'] == spec['candidate']['path']
                and item['sha256'] == spec['candidate']['sha256']]
    if len(matching) != 1 or manifest['candidate'] != matching[0]:
        raise ValueError('KV candidate must be a shipped measured workflow artifact')
    closure = harness_integrity.verify(HERE.parents[2], manifest['candidate_sha'])
    if closure != manifest['harness_closure'] or manifest.get('harness_repo') != str(HERE.parents[2]):
        raise ValueError('KV harness closure differs from candidate')
    entry = cases[spec['case']]
    expected = [str(HERE/'case_entry.sh'), '--case', spec['case'], '--lane', lane.lower(),
                '--manifest', str(spec_path), '--manifest-sha256', manifest['lane_manifest']['sha256'],
                '--out', entry['out']]
    if entry['command'] != expected or pin(entry['launcher']) != HERE/'case_entry.sh':
        raise ValueError('KV frozen command changed')
    if manifest['tools'] != spec['tools']:
        raise ValueError('KV tool pins changed')
    for value in spec['inputs'].values():
        result_writer.require_input_origin(entry['out'], Path(value['path']).parent)
    return cases


def freeze_adapter(args, family, lane):
    import build_roles
    import harness_integrity
    selection = load(args.selection)
    spec = load(args.lane_manifest)
    if (selection['candidate_sha'] != spec['candidate_sha'] or args.case != spec['case']
            or args.case not in selection['selected']):
        raise ValueError('KV case/candidate not selected')
    if selection.get('catalog_sha256') != manifest_tool.sha256_file(args.catalog):
        raise ValueError('KV selection/catalog digest mismatch')
    import execution_policy
    if execution_policy.load_manifest(args.tool_manifest) != spec['tools']:
        raise ValueError('KV frozen tool manifest differs from execution spec')
    validate(spec, family)
    measured, _ = build_roles.load_receipt(args.build_receipt, spec['candidate_sha'])
    matching = [item for item in measured if Path(item['path']).resolve() == args.candidate.resolve()
                and item['sha256'] == spec['candidate']['sha256']]
    if len(matching) != 1 or Path(spec['candidate']['path']).resolve() != args.candidate.resolve():
        raise ValueError('KV measured candidate/build receipt mismatch')
    def file_pin(path):
        path = Path(path).resolve(strict=True)
        return {'path': str(path), 'sha256': manifest_tool.sha256_file(path)}
    inputs = {}
    def collect(value):
        if isinstance(value, dict):
            if 'path' in value and 'sha256' in value:
                path = pin(value)
                inputs[str(path)] = file_pin(path)
            for child in value.values():
                collect(child)
        elif isinstance(value, list):
            for child in value:
                collect(child)
    collect(spec)
    collect(load(args.build_receipt))
    closure = harness_integrity.verify(HERE.parents[2], spec['candidate_sha'])
    collect(closure)
    for path in [args.selection, args.catalog, args.lane_manifest, args.build_receipt, args.tool_manifest,
                 args.build_receipt.parent/'validation-only.log', *args.input]:
        item = file_pin(path)
        inputs[item['path']] = item
    lane_pin = file_pin(args.lane_manifest)
    command = [str(HERE/'case_entry.sh'), '--case', args.case, '--lane', lane.lower(),
               '--manifest', lane_pin['path'], '--manifest-sha256', lane_pin['sha256'],
               '--out', str(args.run_out.resolve())]
    manifest = {'schema_version': 'kv-r5-frozen.v1', 'lane': lane,
        'candidate_sha': spec['candidate_sha'], 'candidate': matching[0],
        'build_receipt': file_pin(args.build_receipt), 'lane_manifest': lane_pin,
        'tools': spec['tools'], 'inputs': list(inputs.values()), 'harness_closure': closure,
        'harness_repo': str(HERE.parents[2]),
        'cases': {args.case: {'launcher': file_pin(HERE/'case_entry.sh'), 'command': command,
                             'out': str(args.run_out.resolve())}}}
    adapter_preflight(manifest, family, lane)
    save(args.out, manifest)
    return manifest


def adapter_main(argv, family):
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument('--case', required=True, choices=family.CASES)
    parser.add_argument('--manifest', required=True, type=Path)
    args, _ = parser.parse_known_args(argv)
    if load(args.manifest)['case'] != args.case:
        raise ValueError('KV command case does not match the frozen execution spec')
    # Remove only the dispatch-owned --case pair; retain all runtime arguments.
    index = argv.index('--case')
    return main(family, argv[:index]+argv[index+2:])


def main(family, argv=None):
    def interrupted(_signum, _frame):
        raise InterruptedError('execution interrupted')
    signal.signal(signal.SIGTERM, interrupted)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', required=True, type=Path)
    parser.add_argument('--manifest-sha256', required=True)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--preflight', action='store_true')
    args = parser.parse_args(argv)
    runtime = None
    try:
        result_writer.require_python()
        if manifest_tool.sha256_file(args.manifest) != args.manifest_sha256:
            raise ValueError('execution manifest hash mismatch')
        data = load(args.manifest)
        validate(data, family)
        if args.preflight:
            return 0
        args.out.mkdir(exist_ok=False)
        evidence = args.out/'evidence'
        evidence.mkdir()
        save(evidence/'manifest.json', data)
        runtime = Runtime(data, evidence, family)
        started = dt.datetime.now(dt.timezone.utc).isoformat()
        passed, observed = False, {}
        try:
            runtime.start()
            observed = runtime.run()
            # Detect mutable input/manifest changes before accepting evidence.
            validate(data, family)
            if manifest_tool.sha256_file(args.manifest) != args.manifest_sha256:
                raise ValueError('manifest changed during execution')
            passed = True
        except (Exception, KeyboardInterrupt) as exc:
            observed = {'error': type(exc).__name__+': '+str(exc), 'certification_state': 'incomplete_or_failed'}
        finally:
            try:
                runtime.cleanup()
            except Exception as exc:
                passed = False
                observed['cleanup_error'] = type(exc).__name__+': '+str(exc)
        record = {'ID': data['case'], 'result': 'PASS' if passed else 'FAIL',
            'candidate_sha': data['candidate_sha'], 'attempt': 1, 'start_utc': started,
            'end_utc': dt.datetime.now(dt.timezone.utc).isoformat(), 'owner': data['owner'],
            'evidence_link': data['evidence_link'], 'gpu_class': data['gpu_class'],
            'driver': getattr(runtime, 'driver', 'unavailable'),
            'versions': {'image': data['image'], 'source_revision': data['source_revision'],
                         'candidate_binary_sha256': data['candidate']['sha256'],
                         'execution_manifest_sha256': args.manifest_sha256},
            'observed_values': observed, 'thresholds': family.CRITERIA[data['case']],
            'evidence_paths': [str(p.relative_to(args.out)) for p in evidence.rglob('*') if p.is_file()]}
        result_writer.write_record(args.out, record)
        return 0 if passed else 1
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as exc:
        parser.exit(1, 'r5 execution refused: '+str(exc)+'\n')
