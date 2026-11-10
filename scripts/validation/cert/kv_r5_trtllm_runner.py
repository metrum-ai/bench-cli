#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Standalone stable TRT-LLM 1.2.1 source-defined exporter certification."""
import json
import math
import re
import sys
from decimal import Decimal

import kv_r5_runtime as runtime
from metrics_oracle import SAMPLE, parse_labels

IMAGE = 'nvcr.io/nvidia/tensorrt-llm/release@sha256:33cd085b772947bd22b7273886539331420404e5d2a4a039945241945ff927b9'
CHILD = 'sha256:cb4d8af81c586a90235ae3739b6d4ddc5d8336f2174c8a1c6b573d2e13faf5d7'
REVISION = '376f7e1bd8ed543f75014309e3fd4b237e9b0e73'
MODEL = 'Qwen/Qwen3-30B-A3B-Instruct-2507-FP8'
MODEL_REVISION = '5a5a776300a41aaa681dd7ff0106608ef2bc90db'
METRICS_PATH = '/prometheus/metrics'
CASES = ('BC-KV-007',)
LIMITS = {'BC-KV-007': 3000}
UNAVAILABLE = {'value': None, 'status': 'unknown', 'reason': 'not exported by 1.2.1'}
CRITERIA = {'BC-KV-007': {'source_defined_families_complete': True, 'consistent_live_deltas': True,
                        'cached_tokens': UNAVAILABLE, 'kv_headroom_min_fraction': 0.25}}
SUCCESS = 'trtllm_request_success_total'
HISTOGRAMS = {
    'trtllm_e2e_request_latency_seconds': [.3,.5,.8,1,1.5,2,2.5,5,10,15,20,30,40,50,60,120,240,480,960,1920,7680],
    'trtllm_time_to_first_token_seconds': [.001,.005,.01,.02,.04,.06,.08,.1,.25,.5,.75,1,2.5,5,7.5,10,20,40,80,160,640,2560],
    'trtllm_time_per_output_token_seconds': [.01,.025,.05,.075,.1,.15,.2,.3,.4,.5,.75,1,2.5,5,7.5,10,20,40,80],
    'trtllm_request_queue_time_seconds': [.3,.5,.8,1,1.5,2,2.5,5,10,15,20,30,40,50,60,120,240,480,960,1920,7680],
}
SOURCE_FILES = {
    'metrics/collector.py': '68a904084f36e491d34f035efa0c8bded13330f59d0eb28821e0eb779d59c475',
    'metrics/enums.py': 'f762c5c9f5c37ef78b697af1deb63b3809901f3245a273b42ac3580102b1873e',
    'serve/openai_server.py': '592027a9baeec71d6bad39355334aa4015733c713f48dada71ec193f062966e0',
    '_torch/pyexecutor/_util.py': '71594a3308b0ea439c05af95ba8a19974f0c3271dfdee34b50a4254c42b385b2',
}


def validate(data):
    if (data['model'], data['model_revision'], data['served_model']) != (MODEL, MODEL_REVISION, 'model'):
        raise ValueError('stable supported Qwen model/revision mismatch')
    if data['metrics_port'] != data['port'] or data['metrics_internal_port'] != 8000:
        raise ValueError('TRT-LLM standalone endpoint mismatch')
    if [p['name'] for p in data['phases']] != ['primer', 'measure']:
        raise ValueError('held-out primer and measured phases required')
    if not all(r['warmup'] for r in data['phases'][0]['expected_requests']) or any(r['warmup'] for r in data['phases'][1]['expected_requests']):
        raise ValueError('primer and measured cohorts overlap')
    config = runtime.load(runtime.pin(data['inputs']['engine_config']))
    allowed = {'return_perf_metrics', 'kv_cache_config', 'max_batch_size', 'max_num_tokens',
               'max_seq_len', 'tensor_parallel_size', 'pipeline_parallel_size'}
    if set(config)-allowed:
        raise ValueError('unreviewed stable engine configuration key')
    if config.get('return_perf_metrics') is not True:
        raise ValueError('stable Prometheus collector must be enabled explicitly')
    kv = config.get('kv_cache_config', {})
    if set(kv) != {'free_gpu_memory_fraction'} or not .25 <= kv['free_gpu_memory_fraction'] <= .9:
        raise ValueError('freeze uncapped KV profiling memory fraction')
    if config.get('tensor_parallel_size', 1) != 1 or config.get('pipeline_parallel_size', 1) != 1:
        raise ValueError('single GPU topology required')
    if data.get('server_flags'):
        raise ValueError('TRT-LLM options must be frozen in the reviewed config file')


def server(data):
    return ['--mount', 'type=bind,src='+data['inputs']['engine_config']['path']+',dst=/engine-config.json,readonly',
            '--entrypoint', 'trtllm-serve', IMAGE, '/model', '--host', '0.0.0.0', '--port', '8000',
            '--backend', 'pytorch', '--config', '/engine-config.json']


def preflight(instance):
    code = ('import importlib.util,importlib.metadata,json,pathlib; '
            'root=pathlib.Path(importlib.util.find_spec("tensorrt_llm").origin).parent; '
            'print(json.dumps({"version":importlib.metadata.version("tensorrt_llm"),'
            '"files":{str(root/p):h for p,h in '+repr(SOURCE_FILES)+'.items()}}))')
    package = json.loads(instance.docker('exec', instance.name, 'python3', '-c', code).stdout)
    runtime.save(instance.out/'trtllm-package.json', package)
    if not re.fullmatch(r'1\.2\.1(?:\+[a-zA-Z0-9.]+)?', package['version']):
        raise ValueError('actual standalone TRT-LLM package is not stable 1.2.1')
    expected = package['files']
    if any(instance.data['source_files'].get(path) != digest for path, digest in expected.items()):
        raise ValueError('actual standalone collector/endpoint source is not pinned')


def headroom(log):
    """Conservative bound from the source's measured memory-profiling report."""
    pattern = (r'available KV cache memory when calculating max tokens: ([0-9]+\.[0-9]{2}) GiB, '
               r'fraction is set [0-9.]+, kv size is [0-9]+\. device total memory ([0-9]+\.[0-9]{2}) GiB')
    values = re.findall(pattern, log)
    if len(values) != 1:
        raise ValueError('one attributable runtime KV memory profile is required')
    available, total = map(float, values[0])
    # The source rounds to two decimals. Bound conservatively rather than
    # rounding a value just below the criterion into a pass.
    fraction = (available-.005)/(total+.005)
    if not math.isfinite(fraction) or fraction < .25:
        raise ValueError('measured KV headroom below 25%')
    return {'profiled_available_kv_gib': available, 'profiled_total_gpu_gib': total,
            'conservative_headroom_fraction': fraction,
            'scope': 'runtime peak-memory profiling; not physical occupancy'}


def after_phase(instance, phase, directory):
    if phase['name'] == 'primer':
        result = instance.docker('logs', instance.name)
        log = result.stdout+result.stderr
        (directory/'memory-profile.log').write_text(log)
        fit = headroom(log)
        inventory = runtime.load(instance.out/'gpu.json')['observed'].split(',')
        observed_gib = int(inventory[2].strip())/1024
        if not math.isclose(fit['profiled_total_gpu_gib'], observed_gib, rel_tol=.01):
            raise ValueError('memory profile does not match the bound GPU inventory')
        runtime.save(directory/'headroom.json', fit)


def snapshot(text):
    types, values = {}, {}
    if len(text.encode()) > 4*1024*1024:
        raise ValueError('oversized exporter page')
    names = {SUCCESS} | {parent+suffix for parent in HISTOGRAMS for suffix in ('_bucket','_count','_sum')}
    seen = set()
    for line in text.splitlines():
        if line.strip() == '# EOF':
            break
        if line.startswith('# TYPE '):
            _, _, name, kind = line.split()
            if name == SUCCESS or name in HISTOGRAMS:
                if name in types or name in seen:
                    raise ValueError('late/duplicate TYPE')
                types[name] = kind
        elif line and not line.startswith('#'):
            match = SAMPLE.fullmatch(line)
            if match is None:
                if line.startswith('trtllm_'):
                    raise ValueError('malformed collector sample')
                continue
            name = match[1]
            if name not in names:
                continue
            parent = name if name == SUCCESS else name.rsplit('_', 1)[0]
            if types.get(parent) != ('counter' if name == SUCCESS else 'histogram'):
                raise ValueError('missing/incorrect source family TYPE')
            labels, value = parse_labels(match[2]), float(match[3])
            unit = 'seconds' if name.endswith('_sum') else 'observations' if name != SUCCESS else 'requests'
            exact = Decimal(match[3])
            if not math.isfinite(value) or value < 0 or (unit != 'seconds' and (exact != exact.to_integral_value() or exact > 2**53)):
                raise ValueError('invalid cumulative sample')
            key = (name, labels)
            if key in values:
                raise ValueError('duplicate source series')
            if len(values) >= 256:
                raise ValueError('too many collector series')
            values[key] = value
            seen.add(parent)
    if set(HISTOGRAMS) | {SUCCESS} != seen:
        raise ValueError('required stable collector family missing')
    # Require the complete source-defined bucket layout for each label group.
    for parent, bounds in HISTOGRAMS.items():
        groups = {}
        for (name, labels), value in values.items():
            if name != parent+'_bucket':
                continue
            labels = dict(labels)
            if 'le' not in labels:
                raise ValueError('bucket boundary absent')
            bound = float(labels.pop('le'))
            groups.setdefault(tuple(sorted(labels.items())), {})[bound] = value
        if not groups:
            raise ValueError('histogram buckets missing')
        for labels, buckets in groups.items():
            if set(buckets) != set(bounds) | {math.inf}:
                raise ValueError('incomplete/source-mismatched histogram layout')
            ordered = [buckets[b] for b in sorted(buckets)]
            if ordered != sorted(ordered) or values.get((parent+'_count', labels)) != buckets[math.inf]:
                raise ValueError('histogram count/bucket inconsistency')
            if (parent+'_sum', labels) not in values:
                raise ValueError('histogram sum missing')
    return values


def compare(before, after, expected_requests):
    a, b = snapshot(before), snapshot(after)
    if a.keys() != b.keys() or any(b[key] < a[key] for key in a):
        raise ValueError('collector labels changed or counter reset')
    success = [(key, b[key]-a[key]) for key in a if key[0] == SUCCESS]
    # This narrow single-box gate accepts exactly one source finish-reason
    # series. No implicit replica/model/finish-reason summation rule.
    if len(success) != 1 or success[0][1] != expected_requests:
        raise ValueError('completion delta differs from independently captured responses')
    base_labels = tuple((k,v) for k,v in success[0][0][1] if k != 'finished_reason')
    delta = []
    for (name, labels), start in a.items():
        if name != SUCCESS and tuple((k,v) for k,v in labels if k != 'le') != base_labels:
            raise ValueError('histogram identity differs from completed-request identity')
        difference = b[(name, labels)]-start
        if name.endswith('_count') and difference != expected_requests:
            raise ValueError('histogram count differs from request count')
        delta.append({'metric': name, 'labels': dict(labels), 'start': start, 'end': b[(name,labels)],
                      'delta': difference, 'unit': 'seconds' if name.endswith('_sum') else 'requests' if name == SUCCESS else 'observations'})
    return delta


def evaluate(data, phases):
    for phase in phases:
        runtime.product_rows(phase)
    primer, measured = phases
    fit = runtime.load(primer['directory']/'headroom.json')
    # Recompute from retained raw source output, not a receipt assertion.
    if fit != headroom((primer['directory']/'memory-profile.log').read_text()):
        raise ValueError('headroom receipt differs from raw runtime profile')
    count = len(measured['plan']['expected_requests'])
    delta = compare(measured['before'], measured['after'], count)
    return {'series': delta, 'measured_requests': count, 'cached_tokens': UNAVAILABLE,
            'cache_reuse_attribution': 'unavailable', 'physical_tier': 'unknown', 'fit': fit,
            'model': MODEL, 'model_revision': MODEL_REVISION,
            'model_selection_reason': 'Qwen3.8 hybrid architecture unsupported; listed Qwen3MoE FP8 checkpoint with single-GPU headroom'}


if __name__ == '__main__':
    raise SystemExit(runtime.main(sys.modules[__name__]))
