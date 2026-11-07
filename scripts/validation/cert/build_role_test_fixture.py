# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic workflow/build metadata for unit tests; NEVER certification evidence."""
import io
import json
import tarfile
from pathlib import Path
from execution_evidence import digest
import pretag_artifact as pretag


def workflow_evidence(root, sha, version, binaries, run_id=19, *, variant=False):
    root = Path(root)
    root.mkdir()
    artifact_name = pretag.TOKENIZER_ARTIFACT if variant else pretag.ARTIFACT
    prefix = f'metrum-ai-bench-cli-v{version}-{pretag.TARGET}' + ('-tokenizer' if variant else '')
    archive = root / (prefix + '.tar.gz')
    with tarfile.open(archive, 'w:gz') as tar:
        for name, content in binaries.items():
            info = tarfile.TarInfo(prefix + '/bin/' + name)
            info.size = len(content)
            info.mode = 0o755
            tar.addfile(info, io.BytesIO(content))
        if variant:
            profile = json.dumps(pretag.variant_profile(sha, version)).encode()
            info = tarfile.TarInfo(prefix + '/build-profile.json')
            info.size = len(profile)
            info.mode = 0o644
            tar.addfile(info, io.BytesIO(profile))
    checksum = Path(str(archive) + '.sha256')
    checksum.write_text(digest(archive) + '  ' + archive.name + '\n')
    run = {'id': run_id, 'head_sha': sha, 'head_repository': {'full_name': pretag.REPOSITORY},
           'path': pretag.WORKFLOW, 'event': 'workflow_dispatch', 'status': 'completed',
           'conclusion': 'success', 'synthetic_unit_fixture': True}
    artifact = {'name': artifact_name, 'expired': False,
                'workflow_run': {'id': run_id, 'head_sha': sha}, 'synthetic_unit_fixture': True}
    for name, data in [('run.json', run), ('artifact.json', artifact)]:
        (root / name).write_text(json.dumps(data))
    (root / 'artifacts.json').write_text(json.dumps({'total_count': 1, 'artifacts': [artifact]}))
    (root / 'download.log').write_text('SYNTHETIC UNIT TRANSPORT; NO REAL WORKFLOW\n')
    return {'kind': 'pretag-workflow', 'run_id': run_id, 'artifact_name': artifact_name,
            'version': version, 'run': pretag.pin(root / 'run.json'),
            'artifact_metadata': pretag.pin(root / 'artifact.json'),
            'artifact_listing': pretag.pin(root / 'artifacts.json'),
            'archive': pretag.pin(archive), 'checksum': pretag.pin(checksum),
            'download_log': pretag.pin(root / 'download.log')}


def bind(manifest, root):
    root = Path(root)
    sha = manifest['candidate_sha']
    measured = manifest['candidate']
    origin = workflow_evidence(root / 'synthetic-workflow', sha, '1.8.1',
                              {'metrum-ai-bench-cli-llm': Path(measured['path']).read_bytes()})
    measured.update(candidate_sha=sha, role='measured', profile='release', features=['otlp'], origin=origin)
    helper = root / 'validation-only-fixture'
    helper.write_text('#!/bin/sh\n# Copyright (c) 2026 Metrum AI, Inc.\n# synthetic validation-only\nexit 0\n')
    helper.chmod(0o755)
    validation = {'path': str(helper), 'sha256': digest(helper), 'candidate_sha': sha,
                  'role': 'validation-only', 'profile': 'release', 'features': ['otlp', 'tokenizer'],
                  'origin': {'kind': 'local-cargo-validation'}}
    manifest['tokenizer_validation'] = validation
    manifest['measured_binaries'] = [measured]
    log = root / 'validation-only.log'
    log.write_text('SYNTHETIC UNIT FIXTURE; NOT BUILD EVIDENCE\n')
    receipt = {'_copyright': 'Copyright (c) 2026 Metrum AI, Inc.', 'candidate_sha': sha, 'version': '1.8.1',
        'builds': {'measured': {'origin': origin, 'binaries': {'metrum-ai-bench-cli-llm': measured}},
                   'validation-only': {'argv': ['cargo', 'build', '--locked', '--release', '--features', 'otlp,tokenizer'],
                    'log_sha256': digest(log), 'binaries': {'metrum-ai-bench-cli-llm': validation}}}}
    path = root / 'builds.json'
    path.write_text(json.dumps(receipt))
    manifest['build_receipt'] = pretag.pin(path)
    lane = root / 'role-lane.json'
    lane.write_text(json.dumps({'lanes': {'synthetic': {'strategic_binary': measured}}}))
    manifest['lane_manifest'] = pretag.pin(lane)
    return manifest
