# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Download and bind exact non-publishing pretag workflow package evidence."""
import hashlib
import json
import re
import subprocess
import tarfile
from pathlib import Path
from execution_evidence import digest

REPOSITORY = 'metrum-ai/bench-cli'
TARGET = 'x86_64-unknown-linux-gnu'
ARTIFACT = 'pretag-release-' + TARGET
WORKFLOW = '.github/workflows/pretag-certification.yml'


def pin(path):
    return {'path': str(path.resolve()), 'sha256': digest(path)}


def validate_metadata(run, artifact, sha, run_id, artifact_name=ARTIFACT):
    if (run.get('id') != run_id or run.get('head_sha') != sha
            or run.get('head_repository', {}).get('full_name') != REPOSITORY
            or run.get('path') != WORKFLOW or run.get('event') != 'workflow_dispatch'
            or run.get('status') != 'completed' or run.get('conclusion') != 'success'):
        raise ValueError('measured build requires successful frozen-candidate pretag workflow')
    if (artifact.get('name') != artifact_name or artifact.get('expired') is not False
            or artifact.get('workflow_run', {}).get('id') != run_id
            or artifact.get('workflow_run', {}).get('head_sha') != sha):
        raise ValueError('pretag artifact does not belong to frozen workflow run')


def verified_file(item):
    path = Path(item['path']).resolve(strict=True)
    if digest(path) != item['sha256']:
        raise ValueError('pretag evidence hash mismatch')
    return path


def verify(origin, sha):
    artifact_name = ARTIFACT
    if origin.get('kind') != 'pretag-workflow' or origin.get('artifact_name') != artifact_name:
        raise ValueError('local measured builds are not certification evidence')
    version = origin['version']
    if not re.fullmatch(r'\d+\.\d+\.\d+', version):
        raise ValueError('pretag version must be a stable release')
    run = json.loads(verified_file(origin['run']).read_text())
    artifact = json.loads(verified_file(origin['artifact_metadata']).read_text())
    listing = json.loads(verified_file(origin['artifact_listing']).read_text())
    if listing.get('total_count') != len(listing.get('artifacts', [])) or [a for a in listing['artifacts'] if a['name'] == artifact_name] != [artifact]:
        raise ValueError('pretag artifact listing is incomplete or ambiguous')
    validate_metadata(run, artifact, sha, origin['run_id'], artifact_name)
    verified_file(origin['download_log'])
    archive = verified_file(origin['archive'])
    checksum = verified_file(origin['checksum'])
    root = f'metrum-ai-bench-cli-v{version}-{TARGET}'
    expected = root + '.tar.gz'
    fields = checksum.read_text().split()
    if archive.name != expected or fields != [digest(archive), expected]:
        raise ValueError('pretag archive does not match workflow-recorded checksum')
    binaries = {}
    with tarfile.open(archive, 'r:gz') as tar:
        seen = set()
        for member in tar:
            # Never extract links, special files, traversal or ambiguous duplicates.
            parts = Path(member.name).parts
            canonical = '/'.join(parts)
            if member.name.startswith('/') or '..' in parts or canonical in seen:
                raise ValueError('unsafe or duplicate pretag archive member')
            seen.add(canonical)
            if len(parts) != 3 or parts[:2] != (root, 'bin'):
                continue
            if not member.isfile() or not member.mode & 0o111:
                raise ValueError('pretag bin member must be a regular executable')
            if not parts[2].startswith('metrum-ai-bench-cli'):
                continue  # Go fixture server is not a measured Cargo CLI.
            stream = tar.extractfile(member)
            binaries[parts[2]] = hashlib.file_digest(stream, 'sha256').hexdigest()
    if 'metrum-ai-bench-cli-llm' not in binaries:
        raise ValueError('pretag package lacks measured LLM CLI')
    return binaries


def download(sha, version, run_id, out):
    if type(run_id) is not int or run_id <= 0:
        raise ValueError('workflow run id must be positive')
    if not re.fullmatch(r'[a-f0-9]{40}', sha) or not re.fullmatch(r'(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)', version):
        raise ValueError('pretag download requires full source SHA and stable version')
    artifact_name = ARTIFACT
    def api(endpoint):
        return json.loads(subprocess.check_output(['gh', 'api', '--hostname', 'github.com',
                          f'repos/{REPOSITORY}/actions/{endpoint}'], text=True))
    out.mkdir(exist_ok=False)
    run = api(f'runs/{run_id}')
    (out / 'run.json').write_text(json.dumps(run, indent=2, sort_keys=True) + '\n')
    listing = api(f'runs/{run_id}/artifacts?per_page=100')
    (out / 'artifacts.json').write_text(json.dumps(listing, indent=2, sort_keys=True) + '\n')
    if listing['total_count'] > 100 or listing['total_count'] != len(listing['artifacts']):
        raise ValueError('artifact list incomplete; refusing ambiguous download')
    matches = [a for a in listing['artifacts'] if a['name'] == artifact_name]
    if len(matches) != 1:
        raise ValueError('expected exactly one nonexpired pretag release artifact')
    artifact = matches[0]
    validate_metadata(run, artifact, sha, run_id, artifact_name)
    for name, data in [('run.json', run), ('artifact.json', artifact)]:
        (out / name).write_text(json.dumps(data, indent=2, sort_keys=True) + '\n')
    with (out / 'download.log').open('xb') as log:
        subprocess.run(['gh', 'run', 'download', str(run_id), '--repo', 'https://github.com/' + REPOSITORY,
                        '--name', artifact_name, '--dir', str(out / 'download')],
                       stdout=log, stderr=subprocess.STDOUT, check=True)
    root = f'metrum-ai-bench-cli-v{version}-{TARGET}'
    archive = out / 'download' / (root + '.tar.gz')
    origin = {'kind': 'pretag-workflow', 'run_id': run_id, 'artifact_name': artifact_name, 'version': version,
              'run': pin(out / 'run.json'), 'artifact_metadata': pin(out / 'artifact.json'),
              'artifact_listing': pin(out / 'artifacts.json'),
              'archive': pin(archive), 'checksum': pin(Path(str(archive) + '.sha256')),
              'download_log': pin(out / 'download.log')}
    members = verify(origin, sha)
    destination = out / 'bin'
    destination.mkdir()
    with tarfile.open(archive, 'r:gz') as tar:
        for name, expected_hash in members.items():
            path = destination / name
            with tar.extractfile(f'{root}/bin/{name}') as source, path.open('xb') as target:
                while chunk := source.read(1024 * 1024):
                    target.write(chunk)
            path.chmod(0o755)
            if digest(path) != expected_hash:
                raise ValueError('extracted measured binary hash mismatch')
    return origin, {name: destination / name for name in members}
