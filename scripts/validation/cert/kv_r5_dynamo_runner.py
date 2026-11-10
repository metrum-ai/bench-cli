#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Single-box stable Dynamo KVBM/vLLM certification gate; no tier inference."""
import json
import os
import re
import sys

import kv_r5_metrics as metrics
import kv_r5_runtime as runtime

IMAGE = 'nvcr.io/nvidia/ai-dynamo/vllm-runtime@sha256:fd7f975e6da114d745611176c67b2ebedd3f891bf80211dbf611e8d154d313a0'
CHILD = 'sha256:6b14a01c813c0a4ab8a5ac2c13a6403b639ac1ed08e816c16ad9eda0f93af75f'
REVISION = '28f9765d98ded77149a94d0defee6d54e97f6741'
METRICS_PATH = '/metrics'
CASES = ('BC-KV-006',)
LIMITS = {'BC-KV-006': 3600}
CRITERIA = {'BC-KV-006': {'required_families_complete': True, 'positive_matched_token_delta': True,
                        'correct_block_token_ratio_units': True, 'physical_tier': 'unknown'}}
SPECS = {'kvbm_offload_blocks_d2h': ('counter', 'blocks'),
         'kvbm_onboard_blocks_h2d': ('counter', 'blocks'),
         'kvbm_matched_tokens': ('counter', 'tokens'),
         'kvbm_host_cache_hit_rate': ('gauge', 'ratio'),
         'kvbm_disk_cache_hit_rate': ('gauge', 'ratio'),
         'kvbm_object_cache_hit_rate': ('gauge', 'ratio')}
CONNECTOR = {'kv_connector': 'DynamoConnector', 'kv_role': 'kv_both',
             'kv_connector_module_path': 'kvbm.vllm_integration.connector'}


def validate(data):
    if data['metrics_internal_port'] != 6880 or data['metrics_port'] in (data['port'], data['proxy_port']):
        raise ValueError('dedicated KVBM endpoint required')
    if [p['name'] for p in data['phases']] != ['populate', 'reuse', 'control']:
        raise ValueError('deterministic population/reuse/control phases required')
    if type(data['cpu_cache_gb']) is not int or not 0 < data['cpu_cache_gb'] <= 256:
        raise ValueError('frozen bounded CPU cache required')
    if not data['model'].startswith('Qwen/') or not runtime.manifest_tool.REV.fullmatch(data['model_revision']):
        raise ValueError('official Qwen model and full revision required')
    if data['model'] == 'Qwen/Qwen3-8B' and data.get('last_resort_notice') is not True:
        raise ValueError('last-resort model requires explicit notice')
    if data['model'] != 'Qwen/Qwen3.8-27B-FP8' and not data.get('model_selection_reason'):
        raise ValueError('primary-model fallback needs supported-model rationale')
    proof = runtime.load(runtime.pin(data['inputs']['kvbm_build_provenance']))
    if (proof.get('source_revision') != REVISION or proof.get('image') != IMAGE
            or not proof.get('independent_build_evidence') or not proof.get('extension_sha256')):
        raise ValueError('KVBM executable/source binding is not established by an image label')
    for item in proof['independent_build_evidence']:
        runtime.pin(item)
    flags = data['server_flags']
    forbidden = {'--model', '--revision', '--served-model-name', '--port', '--host', '--kv-transfer-config', '--tokenizer'}
    if any(a.split('=')[0] in forbidden for a in flags):
        raise ValueError('identity/connector override forbidden')
    if '--enable-prefix-caching' not in flags:
        raise ValueError('prefix caching must be explicit')


def server(data):
    return ['--env', 'DYN_KVBM_CPU_CACHE_GB='+str(data['cpu_cache_gb']),
            '--env', 'DYN_KVBM_METRICS=true', '--env', 'DYN_KVBM_METRICS_PORT=6880',
            '--entrypoint', 'vllm', IMAGE, 'serve', '/model', '--served-model-name', data['served_model'],
            '--host', '0.0.0.0', '--port', '8000', '--kv-transfer-config', json.dumps(CONNECTOR),
            *data['server_flags']]


def preflight(instance):
    # The wheel is optional in the official Dockerfile. Do not pip-install a
    # substitute into the frozen image or infer availability from its tag.
    code = '''import hashlib,importlib.metadata,importlib.util,json,pathlib
import kvbm
from kvbm.vllm_integration.connector import DynamoConnector
root=pathlib.Path(kvbm.__file__).parent
files={str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in root.rglob('*.so')}
print(json.dumps({'version':importlib.metadata.version('kvbm'),'backend_version':importlib.metadata.version('vllm'),'extensions':files,
 'connector_source':importlib.util.find_spec('kvbm.vllm_integration.connector').origin,
 'connector_class':DynamoConnector.__name__}))'''
    actual = json.loads(instance.docker('exec', instance.name, 'python3', '-c', code).stdout)
    runtime.save(instance.out/'kvbm-package.json', actual)
    if not re.fullmatch(r'0\.31\.0(?:\+[a-zA-Z0-9.]+)?', actual['backend_version']):
        raise ValueError('Dynamo runtime backend is not stable vLLM 0.31.0')
    proof = runtime.load(runtime.pin(instance.data['inputs']['kvbm_build_provenance']))
    if not actual['extensions'] or actual['extensions'] != proof['extension_sha256']:
        raise ValueError('installed KVBM extension differs from reviewed build evidence')
    if actual['connector_class'] != 'DynamoConnector' or actual['connector_source'] not in instance.data['source_files']:
        raise ValueError('actual KVBM connector source is not frozen')
    source = actual['connector_source']
    expected = 'bb30525efaccd9fb29876922e2b36820829599c8a5d365bd3fc4f06bdc92dd20'
    if instance.data['source_files'][source] != expected:
        raise ValueError('connector registration differs from reviewed 1.6.2 source')
    implementation = source.removesuffix('__init__.py')+'dynamo_connector.py'
    if instance.data['source_files'].get(implementation) != '8c421cab3663a4c0a6cb5e04039689e8cccf08f65788fbcd55b1464578821b61':
        raise ValueError('DynamoConnector implementation is not source-pinned')
    ram = os.sysconf('SC_PAGE_SIZE')*os.sysconf('SC_PHYS_PAGES')
    if ram < (instance.data['cpu_cache_gb']+16)*1024**3:
        raise ValueError('host memory cannot support the frozen CPU cache envelope')


def evaluate(data, phases):
    observed = {}
    for phase in phases:
        rows, usage = runtime.product_rows(phase)
        delta = metrics.deltas(phase['before'], phase['after'], SPECS)
        observed[phase['plan']['name']] = {'series': delta, 'requests': len(rows),
                                         'server_prompt_tokens': sum(r['prompt_tokens'] for r in usage)}
    if metrics.single(observed['reuse']['series'], 'kvbm_matched_tokens') <= 0:
        raise ValueError('real KVBM match/reuse not observed')
    populate, reuse, control = [p['plan']['expected_requests'] for p in phases]
    hashes = lambda plan: [r['messages_sha256'] for r in plan if not r['warmup']]
    if hashes(populate) != hashes(reuse) or not hashes(populate) or set(hashes(populate)) & set(hashes(control)):
        raise ValueError('repeated-prefix and unique control wire identities differ from plan')
    return {'phases': observed, 'physical_tier': 'unknown', 'bytes_per_block': None,
            'attribution': 'source-reported KVBM matches and transfer blocks only'}


if __name__ == '__main__':
    raise SystemExit(runtime.main(sys.modules[__name__]))
