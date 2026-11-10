#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Single-box stable Dynamo KVBM/vLLM certification gate; no tier inference."""
import json
import math
import os
import re
import sys

import kv_r5_metrics as metrics
import kv_r5_runtime as runtime

IMAGE = 'nvcr.io/nvidia/ai-dynamo/vllm-runtime@sha256:fd7f975e6da114d745611176c67b2ebedd3f891bf80211dbf611e8d154d313a0'
CHILD = 'sha256:6b14a01c813c0a4ab8a5ac2c13a6403b639ac1ed08e816c16ad9eda0f93af75f'
REVISION = '28f9765d98ded77149a94d0defee6d54e97f6741'
MODEL = 'Qwen/Qwen3-VL-32B-Instruct-FP8'
MODEL_REVISION = '4bf2c2f39c37c0fede78bede4056e1f18cdf8109'
ARCHITECTURE = 'Qwen3VLForConditionalGeneration'
MODEL_REASON = 'DynamoConnector 1.6.2 lacks HMA; hybrid SSM unsupported'
VLLM_REVISION = 'db9527a46873454610df6dbedf79a36d6bf1a7f6'
VLLM_SOURCE = {
    'model_executor/models/registry.py': '7f1d56e2f4c24ff7a949b0d4e796d6af70fa85a2b97c224652949f8d993dcc22',
    'model_executor/models/qwen3_vl.py': '8081274b53fd41f0bf386e5c3f4c9438d00b2e4400c2acd98a53261ff3228465',
    'v1/worker/gpu_worker.py': '43bd01925cd84f7d8a1d4a2ec30e289014a485fe4492a5343cbc893e8ed2f518',
    'utils/mem_utils.py': '8b21755c8cae378b64d2d5c72c901135820072764218808191f266a65cf3aad5',
    'engine/arg_utils.py': '95be1adb96f55933f6a52260c8cb2181d04707401ca21d97c33108c5ab58f3ac',
    'config/multimodal.py': 'f783ec8290a5e40b5b095767510e0912b713725ca3296aafa268d4077066d9a3',
}
METRICS_PATH = '/metrics'
CASES = ('BC-KV-006',)
LIMITS = {'BC-KV-006': None}  # The reviewed per-run manifest supplies the cap.
CRITERIA = {'BC-KV-006': {'required_families_complete': True, 'positive_matched_token_delta': True,
                        'correct_block_token_ratio_units': True, 'physical_tier': 'unknown',
                        'text_only': True, 'kv_headroom_min_fraction': .25}}
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
    if (data['model'], data['model_revision'], data.get('model_selection_reason')) != (MODEL, MODEL_REVISION, MODEL_REASON):
        raise ValueError('approved full-attention Qwen model/revision/reason required')
    config_path = os.path.join(data['model_snapshot'], 'config.json')
    if runtime.manifest_tool.sha256_file(config_path) != '2329a2c9022d94906d5af9bafe93ca97cba0eedb5426fc42ca64f2bfcd65e949':
        raise ValueError('approved checkpoint config bytes required')
    config = runtime.load(config_path)
    if config.get('architectures') != [ARCHITECTURE] or config.get('model_type') != 'qwen3_vl':
        raise ValueError('staged model architecture mismatch')
    proof = runtime.load(runtime.pin(data['inputs']['kvbm_build_provenance']))
    if (proof.get('source_revision') != REVISION or proof.get('image') != IMAGE
            or not proof.get('independent_build_evidence') or not proof.get('extension_sha256')):
        raise ValueError('KVBM executable/source binding is not established by an image label')
    for item in proof['independent_build_evidence']:
        runtime.pin(item)
    flags = data['server_flags']
    forbidden = {'--model', '--revision', '--served-model-name', '--port', '--host', '--kv-transfer-config', '--tokenizer',
                 '--language-model-only', '--limit-mm-per-prompt', '--enable-mm-embeds', '--enable-prompt-embeds',
                 '--hf-overrides', '--model-impl', '--config', '--kv-cache-memory-bytes', '--num-gpu-blocks-override',
                 '--tensor-parallel-size', '-tp', '--pipeline-parallel-size', '-pp', '--data-parallel-size', '-dp'}
    if runtime.server_overrides(flags, forbidden):
        raise ValueError('identity/connector override forbidden')
    if '--enable-prefix-caching' not in flags:
        raise ValueError('prefix caching must be explicit')


def server(data):
    return ['--env', 'DYN_KVBM_CPU_CACHE_GB='+str(data['cpu_cache_gb']),
            '--env', 'DYN_KVBM_METRICS=true', '--env', 'DYN_KVBM_METRICS_PORT=6880',
            '--entrypoint', 'vllm', IMAGE, 'serve', '/model', '--served-model-name', data['served_model'],
            '--host', '0.0.0.0', '--port', '8000', '--kv-transfer-config', json.dumps(CONNECTOR),
            '--language-model-only', '--tensor-parallel-size', '1', '--pipeline-parallel-size', '1',
            '--data-parallel-size', '1',
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
    # Read the installed registry and import the actual registered class. This
    # is a runtime image gate, distinct from the offline source review.
    code = '''import importlib.util,json,pathlib
from vllm.model_executor.models import ModelRegistry
from vllm.model_executor.models.qwen3_vl import Qwen3VLForConditionalGeneration
root=pathlib.Path(importlib.util.find_spec('vllm').origin).parent
config=json.loads(pathlib.Path('/model/config.json').read_text())
print(json.dumps({'architectures':config['architectures'],
 'registered':'Qwen3VLForConditionalGeneration' in ModelRegistry.get_supported_archs(),
 'class_name':Qwen3VLForConditionalGeneration.__name__, 'root':str(root)}))'''
    capability = json.loads(instance.docker('exec', instance.name, 'python3', '-c', code).stdout)
    runtime.save(instance.out/'model-capability.json', capability)
    if capability['architectures'] != [ARCHITECTURE] or capability['registered'] is not True or capability['class_name'] != ARCHITECTURE:
        raise ValueError('installed vLLM cannot resolve the approved model architecture')
    for path, digest in VLLM_SOURCE.items():
        if instance.data['source_files'].get(capability['root']+'/'+path) != digest:
            raise ValueError('actual vLLM model/profiling/text-only source is not pinned')
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


def headroom(log, gpu_mib):
    values = re.findall(r'Available KV cache memory: ([0-9]+(?:\.[0-9]{1,2})?) GiB', log)
    if len(values) != 1 or type(gpu_mib) is not int or gpu_mib <= 0:
        raise ValueError('one attributable runtime KV memory profile and GPU total required')
    available = float(values[0])
    # v0.31.0 format_gib rounds to two decimals; nvidia-smi reports MiB.
    fraction = (available-.005)/((gpu_mib+.5)/1024)
    if not math.isfinite(fraction) or fraction < .25 or fraction > 1:
        raise ValueError('measured KV headroom below 25% or invalid')
    return {'profiled_available_kv_gib': available, 'gpu_total_mib': gpu_mib,
            'conservative_headroom_fraction': fraction,
            'scope': 'runtime memory profiling; not physical occupancy'}


def after_phase(instance, phase, directory):
    if phase['name'] == 'populate':
        result = instance.docker('logs', instance.name)
        log = result.stdout+result.stderr
        (directory/'memory-profile.log').write_text(log)
        gpu_mib = int(runtime.load(instance.out/'gpu.json')['observed'].split(',')[2].strip())
        runtime.save(directory/'headroom.json', headroom(log, gpu_mib))


def evaluate(data, phases):
    observed = {}
    for phase in phases:
        trace = [json.loads(line) for line in (phase['directory']/'trace.jsonl').read_text().splitlines()]
        if any(e.get('text_only') is not True for e in trace if e['kind'] == 'request_identity'):
            raise ValueError('text-only wire evidence required')
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
    directory = phases[0]['directory']
    gpu_mib = int(runtime.load(directory.parent/'gpu.json')['observed'].split(',')[2].strip())
    fit = headroom((directory/'memory-profile.log').read_text(), gpu_mib)
    if runtime.load(directory/'headroom.json') != fit:
        raise ValueError('KV headroom receipt differs from retained runtime evidence')
    return {'phases': observed, 'physical_tier': 'unknown', 'bytes_per_block': None,
            'model': MODEL, 'model_revision': MODEL_REVISION, 'model_selection_reason': MODEL_REASON,
            'architecture': ARCHITECTURE, 'text_only': True, 'fit': fit,
            'attribution': 'source-reported KVBM matches and transfer blocks only'}


if __name__ == '__main__':
    raise SystemExit(runtime.main(sys.modules[__name__]))
