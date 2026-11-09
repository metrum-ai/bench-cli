# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline OCI descriptor verification for the pinned r4 amd64 image.

Descriptor bytes must be independently staged before offline execution. A
Docker image ID is a config digest, not an OCI manifest digest; matching that
ID to the config descriptor binds the local image and container to the child
manifest whose digest and index membership are independently verified.
"""
import argparse
import hashlib
import json
import os
import re
import subprocess
from pathlib import Path

import r4_model_pins as pins

INDEX = pins.IMAGE.rsplit('@', 1)[1]
AMD64 = 'sha256:91ee972ab751fb7180bc71c175865e36b091686600dca783d52b2511399810b3'
CONFIG = 'sha256:ce429874fcb2f22e87abec76226e454b90390720c14fc3e5238d0ec564008a27'
SHA = re.compile(r'sha256:[0-9a-f]{64}\Z')


def unique(pairs):
    out = {}
    for key, value in pairs:
        if key in out:
            raise ValueError('duplicate OCI JSON key')
        out[key] = value
    return out


def descriptor(path, expected):
    raw = Path(path).read_bytes()
    if 'sha256:' + hashlib.sha256(raw).hexdigest() != expected:
        raise ValueError('OCI descriptor digest mismatch')
    value = json.loads(raw, object_pairs_hook=unique)
    if not isinstance(value, dict):
        raise ValueError('OCI descriptor must be an object')
    return value, len(raw)


def descriptor_chain(index_path, child_path, *, index_digest=INDEX, child_digest=AMD64):
    """Verify the offline index and uniquely selected platform descriptor."""
    index, _ = descriptor(index_path, index_digest)
    child, size = descriptor(child_path, child_digest)
    matches = [d for d in index.get('manifests', []) if isinstance(d, dict)
               and isinstance(d.get('platform'), dict) and d['platform'].get('os') == 'linux'
               and d['platform'].get('architecture') == 'amd64']
    if len(matches) != 1 or matches[0].get('digest') != child_digest or matches[0].get('size') != size:
        raise ValueError('OCI index does not uniquely bind the expected linux/amd64 child')
    config = child.get('config', {})
    config_digest = config.get('digest') if isinstance(config, dict) else None
    if not isinstance(config_digest, str) or not SHA.fullmatch(config_digest):
        raise ValueError('OCI child config digest missing')
    return config_digest


def verify(index_path, child_path, image, container=None, *, index_digest=INDEX, child_digest=AMD64):
    """Verify descriptor chain and actual Docker image/container config identity."""
    config_digest = descriptor_chain(index_path, child_path, index_digest=index_digest, child_digest=child_digest)
    if not isinstance(image, dict) or image.get('Id') != config_digest or image.get('Os') != 'linux' or image.get('Architecture') != 'amd64':
        raise ValueError('local image does not match the amd64 child config')
    if container is not None and (not isinstance(container, dict) or container.get('Image') != config_digest):
        raise ValueError('running container image does not match the amd64 child config')
    return {'image_index': index_digest, 'platform_manifest': child_digest,
            'platform': 'linux/amd64', 'image_config': config_digest,
            'container_verified': container is not None}


def runtime(image_ref, container, out):
    if image_ref != pins.IMAGE:
        raise ValueError('runtime image reference is not the r4 index pin')
    index = os.environ.get('CERT_R4_OCI_INDEX', '')
    child = os.environ.get('CERT_R4_OCI_AMD64', '')
    if not index or not child:
        raise ValueError('staged r4 index and amd64 OCI descriptors are required')
    def inspect(command):
        result = subprocess.run(command, capture_output=True, text=True, timeout=30, check=True)
        value = json.loads(result.stdout, object_pairs_hook=unique)
        if not isinstance(value, list) or len(value) != 1:
            raise ValueError('Docker inspection must return exactly one object')
        return value[0]
    image = inspect(['docker', 'image', 'inspect', image_ref])
    observed_container = inspect(['docker', 'inspect', container]) if container else None
    result = verify(index, child, image, observed_container)
    # Smoke requires an explicit device-request binding before readiness or
    # workload. Certification behavior and full gates are unchanged.
    if container:
        import result_writer
        if result_writer.smoke_origin(Path(out).parent):
            import stage0_device
            device = stage0_device.check_container_request(
                observed_container, os.environ.get('CUDA_VISIBLE_DEVICES'))
            visible = subprocess.run(['docker', 'exec', container, 'nvidia-smi',
                                      '--query-gpu=name,uuid,driver_version', '--format=csv,noheader,nounits'],
                                     capture_output=True, text=True, timeout=30, check=True)
            result['gpu_uuid'] = device
            result['gpu_visible'] = stage0_device.check_container_visible(visible.stdout, device)
            host_lines = (Path(out).parent / 'gpu.csv').read_text(encoding='utf-8').splitlines()
            if len(host_lines) != 1:
                raise ValueError('invalid host GPU identity')
            host_fields = [part.strip() for part in host_lines[0].split(',')]
            if host_fields != [result['gpu_visible']['name'], result['gpu_visible']['driver']]:
                raise ValueError('container GPU does not match the host GPU identity')
    Path(out).write_text(json.dumps(result, sort_keys=True, indent=2) + '\n', encoding='utf-8')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image', required=True)
    parser.add_argument('--container')
    parser.add_argument('--out', required=True)
    args = parser.parse_args()
    try:
        runtime(args.image, args.container, args.out)
    except (OSError, ValueError, TypeError, KeyError, subprocess.SubprocessError) as exc:
        parser.exit(1, f'r4 image provenance failed: {exc}\n')


if __name__ == '__main__':
    main()
