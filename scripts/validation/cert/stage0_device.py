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
TOKEN = re.compile(r'[0-9]+\Z')


def probe(tools, *, device, expected_name, deadline=None, clock=time.monotonic):
    if (not isinstance(device, str) or not (TOKEN.fullmatch(device) or UUID.fullmatch(device))
            or not isinstance(expected_name, str) or not expected_name.strip()):
        raise ValueError('exactly one GPU and a reviewed GPU name are required')
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
            or (UUID.fullmatch(device) and fields[1].lower() != device.lower())):
        raise ValueError('GPU identity differs from the approved box')
    # Detect a tool swap while probing; it is not a host attestation.
    if execution_policy.file_hash(Path(tool)) != checked['nvidia-smi']['sha256']:
        raise ValueError('GPU identity tool changed')
    remaining()
    return {'name': fields[0], 'uuid': fields[1], 'driver': fields[2]}
