# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic build metadata for unit tests only; never certification evidence."""
import json
from pathlib import Path
from execution_evidence import digest


def bind(manifest, root):
    root = Path(root)
    sha = manifest['candidate_sha']
    manifest['candidate'].update(candidate_sha=sha, role='measured', profile='release', features=['otlp'])
    helper = root / 'validation-only-fixture'
    helper.write_text('#!/bin/sh\n# Copyright (c) 2026 Metrum AI, Inc.\n# synthetic validation-only\nexit 0\n')
    helper.chmod(0o755)
    validation = {'path': str(helper), 'sha256': digest(helper), 'candidate_sha': sha,
                  'role': 'validation-only', 'profile': 'release', 'features': ['otlp', 'tokenizer']}
    manifest['tokenizer_validation'] = validation
    manifest['measured_binaries'] = [manifest['candidate']]
    receipt = {'_copyright': 'Copyright (c) 2026 Metrum AI, Inc.', 'candidate_sha': sha, 'builds': {}}
    for role, features, item in [('measured', 'otlp', manifest['candidate']),
                                 ('validation-only', 'otlp,tokenizer', validation)]:
        log = root / (role + '.log')
        log.write_text('SYNTHETIC UNIT FIXTURE; NOT BUILD EVIDENCE\n')
        receipt['builds'][role] = {'argv': ['cargo', 'zigbuild', '--locked', '--release', '--features', features],
                                  'log_sha256': digest(log), 'binaries': {'metrum-ai-bench-cli-llm': item}}
    path = root / 'builds.json'
    path.write_text(json.dumps(receipt))
    manifest['build_receipt'] = {'path': str(path), 'sha256': digest(path)}
    lane = root / 'role-lane.json'
    lane.write_text(json.dumps({'lanes': {'synthetic': {'strategic_binary': manifest['candidate']}}}))
    manifest['lane_manifest'] = {'path': str(lane), 'sha256': digest(lane)}
    return manifest
