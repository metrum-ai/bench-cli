#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""BC-KV-003/004 live candidate runner; missing evidence blocks certification."""
import argparse
import datetime as dt
import json
import os
import subprocess
import time
from pathlib import Path

import kv_external_common as common
import manifest_tool

REVISION = '297e54fd105e974cdf401e5b4d20253a22c16f90'
IMAGE = 'lmsysorg/sglang@sha256:f832b5c39559c9caa900b22ad37c76fcce66470676855c42052102c537812f1f'
CONFIG = 'sha256:f3a303d4028dc4597e312c96ada28fa19509f46559601a94e89dc630ac1ef152'
SOURCE = '745d697dcf71ab8aa148059bccba27a286658bd736ee4b8a08489d90797f19f2'
CASES = {'BC-KV-003', 'BC-KV-004'}
HIT = 'sglang:cached_tokens_total'
RATE = 'sglang:cache_hit_rate'
REQUESTS = 'sglang:num_requests_total'
SPECS = {HIT: 'counter', RATE: 'gauge', REQUESTS: 'counter'}


def validate_config(config):
    if config['image'] != IMAGE or config.get('source_revision') != REVISION:
        raise ValueError('SGLang requires exact reviewed image and source profile')
    if (config['model'], config['revision']) != ('Qwen/Qwen3.8-27B-FP8', 'c3a462cc3d38cf351f3b26cbd763469eeb74ef81'):
        raise ValueError('model substitution requires a separately reviewed configuration')
    flags = config['server_flags']
    forbidden = manifest_tool.FORBIDDEN | {'--model-path', '--model-name', '--tokenizer-path', '--served-model-name', '--disable-radix-cache', '--api-key', '--admin-api-key'}
    if '--enable-metrics' not in flags or any(v.split('=')[0] in forbidden for v in flags):
        raise ValueError('metrics required and identity/cache overrides forbidden')
    for key in ('owner', 'evidence_link'):
        if not isinstance(config.get(key), str) or not config[key].strip():
            raise ValueError('owner and evidence link required')
    case = config.get('kv_case')
    if case not in CASES or os.environ.get('CERT_CASE_ID', case) != case:
        raise ValueError('one exact selected SGLang case required')
    if case == 'BC-KV-004' and '--enable-hierarchical-cache' not in flags:
        raise ValueError('HiCache configuration missing; case remains pending')
    if case == 'BC-KV-003' and '--enable-hierarchical-cache' in flags:
        raise ValueError('radix baseline requires separate HiCache configuration')
    if type(config.get('proxy_port')) is not int or not 1024 <= config['proxy_port'] <= 65535 or config['proxy_port'] == config['port']:
        raise ValueError('distinct proxy port required')
    if type(config.get('settle_seconds')) is not int or not 1 <= config['settle_seconds'] <= 10:
        raise ValueError('bounded frozen settling interval required')
    _, seed = common.prompts(config, 'seed')
    _, repeated = common.prompts(config, 'repeat')
    _, unique = common.prompts(config, 'unique')
    if len(seed) < 2 or len(repeated) < 2 or set(seed) != set(repeated) or len(set(unique)) != len(unique) or set(unique) & set(seed):
        raise ValueError('controlled repeated and unique cohorts required')
    if case == 'BC-KV-004':
        common.prompts(config, 'pressure')
        _, rehit = common.prompts(config, 'rehit')
        if set(rehit) != set(seed):
            raise ValueError('HiCache rehit must use seed prefix')
    fit = common.validate_fit(config)
    if case == 'BC-KV-004' and (type(fit.get('kv_capacity_tokens')) is not int or fit['kv_capacity_tokens'] <= 0):
        raise ValueError('HiCache pressure needs measured KV token capacity')
    return fit


def observation(before, after, request_count, require_reuse):
    # A newly exposed tier has no valid start value. Leave it Unknown rather
    # than inventing zero. Reuse still requires a valid positive same-series delta.
    left = common.parse_page(before, SPECS)
    right = common.parse_page(after, SPECS)
    delta = common.deltas(before, after, {REQUESTS: 'counter'})
    for key in left.keys() & right.keys():
        if key[0] == HIT:
            if right[key] < left[key]:
                raise ValueError('cache counter reset')
            delta[key] = right[key] - left[key]
    # This runner requires one exporter worker. It never sums across workers,
    # replicas, or overlapping cache_source=total and detailed tier views.
    a = common.parse_page(after, SPECS)
    workers = {tuple((k, v) for k, v in labels if k != 'cache_source') for (name, labels) in a if name == HIT}
    if len(workers) != 1:
        raise ValueError('ambiguous multiple cache workers')
    request_rows = [v for (n, _), v in delta.items() if n == REQUESTS]
    if len(request_rows) != 1 or request_rows[0] != request_count:
        raise ValueError('exporter request population differs; exclusive window not established')
    hit_rows = [(dict(labels)['cache_source'], delta.get((name, labels))) for (name, labels) in left.keys() | right.keys() if name == HIT]
    if any(source not in ('device', 'host', 'storage', 'total') for source, _ in hit_rows):
        raise ValueError('unsupported cache source')
    if require_reuse and not any(value is not None and value > 0 for _, value in hit_rows):
        raise ValueError('positive repeated-prefix reuse missing')
    return {'cache_source_token_deltas': dict(hit_rows), 'hit_rates': [v for (n, _), v in a.items() if n == RATE],
            'occupancy': None, 'physical_tier': 'unknown', 'requests': request_count}


def runtime_source(container):
    script = "import hashlib,pathlib,sglang; p=pathlib.Path(sglang.__file__).parent/'srt/observability/metrics_collector.py'; print(hashlib.sha256(p.read_bytes()).hexdigest())"
    proc = subprocess.run(['docker', 'exec', container, 'python3', '-c', script], capture_output=True, text=True, check=True, timeout=30)
    if proc.stdout.strip() != SOURCE:
        raise ValueError('runtime SGLang metric source mismatch')
    return {'metric_source_sha256': proc.stdout.strip(), 'reference_revision': REVISION}


def start_server(args, config):
    fit = validate_config(config)
    # Bind actual memory/GPU identity before starting the engine.
    gpu = subprocess.run(['nvidia-smi', '--id=' + os.environ['CUDA_VISIBLE_DEVICES'],
                          '--query-gpu=uuid,name,memory.total', '--format=csv,noheader,nounits'],
                         check=True, text=True, capture_output=True, timeout=30).stdout.strip().split(',')
    if len(gpu) != 3 or gpu[0].strip() != fit['gpu_uuid'] or gpu[1].strip() != fit['gpu_class'] or int(gpu[2].strip()) * 1024**2 < fit['gpu_memory_bytes']:
        raise ValueError('actual GPU differs from measured fit')
    image = json.loads(subprocess.run(['docker', 'image', 'inspect', IMAGE], check=True, capture_output=True, text=True, timeout=30).stdout)[0]
    if image['Id'] != CONFIG or image.get('Architecture') != 'amd64' or IMAGE not in image.get('RepoDigests', []):
        raise ValueError('local image digest/config/platform differs')
    command = ['docker', 'run', '-d', '--pull=never', '--name', args.container, '--label', f'metrum.cert.owner={args.container_owner}',
               '--gpus', 'device=' + os.environ['CUDA_VISIBLE_DEVICES'], '--ipc=host', '--env', 'CUDA_VISIBLE_DEVICES=0',
               '-p', f'127.0.0.1:{config["port"]}:8000', '--entrypoint', 'python3']
    cache = os.environ.get('CERT_HF_HUB_CACHE')
    if not cache or not Path(cache).is_dir():
        raise ValueError('offline model hub cache required')
    command += ['--mount', f'type=bind,src={Path(cache).resolve()},dst=/root/.cache/huggingface/hub,readonly', '--env', 'HF_HUB_OFFLINE=1',
                IMAGE, '-m', 'sglang.launch_server', '--model-path', config['model'], '--revision', config['revision'],
                '--host', '0.0.0.0', '--port', '8000', *config['server_flags']]
    (args.out / 'evidence/kv-server-command.json').write_text(json.dumps(command) + '\n')
    result = subprocess.run(command, capture_output=True, text=True, check=True, timeout=60)
    (args.out / 'evidence/container-id.txt').write_text(result.stdout)


def find_capture(value):
    if isinstance(value, dict):
        if 'native_cache_metric_discovery' in value:
            return value['native_cache_metric_discovery']
        for child in value.values():
            found = find_capture(child)
            if found is not None:
                return found
    if isinstance(value, list):
        for child in value:
            found = find_capture(child)
            if found is not None:
                return found
    return None


def main():
    case = os.environ.get('CERT_CASE_ID')
    if case not in CASES:
        raise SystemExit('unknown CERT_CASE_ID for kv_sglang_runner.py')
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('out', 'candidate', 'baseline', 'manifest'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--container', required=True)
    parser.add_argument('--container-owner', required=True)
    parser.add_argument('--start-server', action='store_true')
    args = parser.parse_args()
    config = json.loads(args.manifest.read_text(), object_pairs_hook=manifest_tool.unique_object)['lanes']['KVS']
    fit = validate_config(config)
    if args.start_server:
        start_server(args, config)
        return 0
    args.model, args.port, args.proxy_port = config['model'], config['port'], config['proxy_port']
    evidence = args.out / 'evidence'
    observed, passed = {}, False
    try:
        binding = common.docker_json(args.container)
        if binding['image'] != CONFIG or binding['owner'] != args.container_owner or binding['restarts'] != 0:
            raise ValueError('owned image/epoch differs')
        observed['runtime'] = runtime_source(args.container)
        observed['fit'] = fit
        (evidence / 'kv-runtime-binding.json').write_text(json.dumps(binding) + '\n')
        common.run_cohort(args, config, 'seed', args.port, 'sglang0523', REVISION)
        phases = ['repeat', 'unique'] + (['pressure', 'rehit'] if case == 'BC-KV-004' else [])
        observed['phases'] = {}
        for cohort in phases:
            time.sleep(config['settle_seconds'])
            quiet = common.fetch(args.port)
            time.sleep(config['settle_seconds'])
            before = common.fetch(args.port)
            if any(common.deltas(quiet, before, SPECS).values()):
                raise ValueError('window was not quiescent')
            (evidence / f'kv-{cohort}-quiet.prom').write_text(quiet)
            (evidence / f'kv-{cohort}-before.prom').write_text(before)
            result = common.run_cohort(args, config, cohort, args.port, 'sglang0523', REVISION)
            time.sleep(config['settle_seconds'])
            after = common.fetch(args.port)
            (evidence / f'kv-{cohort}-after.prom').write_text(after)
            time.sleep(config['settle_seconds'])
            settled = common.fetch(args.port)
            (evidence / f'kv-{cohort}-settled.prom').write_text(settled)
            if any(common.deltas(after, settled, SPECS).values()):
                raise ValueError('asynchronous work did not settle')
            if common.docker_json(args.container) != binding:
                raise ValueError('container epoch changed')
            capture = find_capture(result['rows'])
            if (not capture or capture.get('profile') != 'sglang0523' or capture.get('reference_source_revision') != REVISION
                    or capture.get('failure') is not None or capture.get('missing_required_metrics') or not capture.get('samples')):
                raise ValueError('candidate profile evidence unavailable')
            if any(s.get('value') is None or s.get('unknown') is not None for s in capture['samples']):
                raise ValueError('candidate profile contains Unknown')
            rates = [s for s in capture['samples'] if s['metric'] == RATE]
            if not rates or any(s.get('unit') != 'ratio' or s.get('role') != 'reported_hit_rate' or s.get('expected_family_type') != 'gauge' for s in rates):
                raise ValueError('hit-rate semantics changed')
            if cohort == 'repeat' and not any(type(u.get('prompt_tokens_details', {}).get('cached_tokens')) is int
                    and u['prompt_tokens_details']['cached_tokens'] > 0 for u in result['usage']):
                raise ValueError('server response cached-token observation missing')
            if cohort == 'pressure':
                counts = [u.get('prompt_tokens') for u in result['usage']]
                if any(type(n) is not int or n <= 0 for n in counts) or sum(counts) <= fit['kv_capacity_tokens']:
                    raise ValueError('pressure did not exceed measured KV token capacity')
            observed['phases'][cohort] = observation(before, after, len(result['requests']), cohort == 'repeat')
        passed = True
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as exc:
        observed['error'] = str(exc)
        observed['coordination_status'] = 'PENDING'
    gpu = (evidence / 'gpu.csv').read_text().strip().split(',', 1)
    record = {'ID': case, 'result': 'PASS' if passed else 'FAIL', 'candidate_sha': os.environ['CERT_SHA'], 'attempt': 1,
              'start_utc': os.environ['CERT_START_UTC'], 'end_utc': dt.datetime.now(dt.timezone.utc).isoformat(),
              'owner': config['owner'], 'evidence_link': config['evidence_link'], 'gpu_class': gpu[0].strip(), 'driver': gpu[1].strip(),
              'versions': {'server_image': IMAGE, 'source_revision': REVISION, 'model': config['model'], 'model_revision': config['revision'],
                           'candidate_binary_sha256': manifest_tool.sha256_file(args.candidate),
                           'baseline_binary_sha256': manifest_tool.sha256_file(args.baseline), 'manifest_sha256': manifest_tool.sha256_file(args.manifest)},
              'observed_values': observed, 'thresholds': {'reuse_observation_positive': True, 'hit_rate_range': [0, 1],
                                                       'occupancy': 'unknown', 'physical_tier': 'unknown'},
              'evidence_paths': [str(p.relative_to(args.out)) for p in evidence.iterdir() if p.is_file()]}
    # common.sh finalizes only after owned-container removal and reconciliation.
    (evidence / f'provisional-{case}.json').write_text(json.dumps(record, allow_nan=False) + '\n')
    return 0 if passed else 1


if __name__ == '__main__':
    raise SystemExit(main())
