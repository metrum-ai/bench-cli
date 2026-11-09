#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Pre/post workload GPU identity probe using a content-pinned native tool."""
import math
import re
import subprocess
import time
from pathlib import Path

import execution_policy

UUID = re.compile(r'GPU-[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\Z')


def check_container_request(container, device):
    """Verify Docker requested exactly the same physical UUID for the server."""
    if not isinstance(device, str) or not UUID.fullmatch(device) or not isinstance(container, dict):
        raise ValueError('invalid container GPU binding')
    host = container.get('HostConfig')
    requests = host.get('DeviceRequests') if isinstance(host, dict) else None
    if not isinstance(requests, list) or len(requests) != 1 or not isinstance(requests[0], dict):
        raise ValueError('missing exclusive container GPU request')
    request = requests[0]
    if (request.get('DeviceIDs') != [device] or request.get('Count') != 0
            or request.get('Driver') not in ('', 'nvidia')
            or request.get('Capabilities') != [['gpu']]):
        raise ValueError('container GPU request differs from the probed GPU')
    return device


def check_container_visible(stdout, device):
    """Validate an in-container GPU query; this relies on the trusted host/runtime."""
    if not isinstance(stdout, str) or len(stdout) > 4096 or not isinstance(device, str) or not UUID.fullmatch(device):
        raise ValueError('invalid container-visible GPU output')
    lines = stdout.splitlines()
    if len(lines) != 1:
        raise ValueError('container must see exactly one GPU')
    fields = [part.strip() for part in lines[0].split(',')]
    if (len(fields) != 3 or not fields[0] or not UUID.fullmatch(fields[1])
            or fields[1].lower() != device.lower() or not fields[2]):
        raise ValueError('container-visible GPU identity differs from the probed GPU')
    return {'name': fields[0], 'uuid': device, 'driver': fields[2]}


def probe(tools, *, device, expected_name, deadline=None, clock=time.monotonic):
    if (not isinstance(device, str) or not UUID.fullmatch(device)
            or not isinstance(expected_name, str) or not expected_name.strip()):
        raise ValueError('one full GPU UUID and a reviewed GPU name are required')
    checked = execution_policy.validate_tools(tools)
    if 'nvidia-smi' not in checked:
        raise ValueError('pinned GPU identity tool missing')
    tool = checked['nvidia-smi']['path']
    def remaining():
        if deadline is None:
            return 15.0
        now = clock()
        if (type(deadline) not in (int, float) or not math.isfinite(deadline)
                or type(now) not in (int, float) or not math.isfinite(now) or now >= deadline):
            raise ValueError('GPU identity probe deadline elapsed')
        return min(15.0, deadline - now)
    timeout = remaining()
    result = subprocess.run([tool, '--id=' + device, '--query-gpu=name,uuid,driver_version',
                             '--format=csv,noheader,nounits'], stdin=subprocess.DEVNULL,
                            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                            env={'PATH': '/nonexistent', 'HOME': '/nonexistent', 'LC_ALL': 'C'},
                            timeout=timeout, check=True)
    if not isinstance(result.stdout, bytes) or len(result.stdout) > 4096:
        raise ValueError('invalid GPU identity output')
    try: lines = result.stdout.decode('utf-8').splitlines()
    except UnicodeDecodeError as exc: raise ValueError('invalid GPU identity output') from exc
    if len(lines) != 1:
        raise ValueError('GPU identity must identify exactly one device')
    fields = [field.strip() for field in lines[0].split(',')]
    if (len(fields) != 3 or fields[0] != expected_name or not UUID.fullmatch(fields[1]) or not fields[2]
            or fields[1].lower() != device.lower()):
        raise ValueError('GPU identity differs from the approved box')
    # Detect a tool swap while probing; it is not a host attestation.
    if execution_policy.file_hash(Path(tool)) != checked['nvidia-smi']['sha256']:
        raise ValueError('GPU identity tool changed')
    remaining()
    return {'name': fields[0], 'uuid': fields[1], 'driver': fields[2]}
