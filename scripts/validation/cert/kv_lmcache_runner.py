#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""BC-KV-005: exact MP LMCache/vLLM connector with independent evidence."""
import argparse
import datetime as dt
import importlib
import json
import os
import subprocess
import time
from pathlib import Path

import kv_external_common as common
import manifest_tool

REVISION = '236f3eec32a2d330662a1795d90a01a1e0782c49'
IMAGE = 'lmcache/vllm-openai@sha256:3be0a977f2aa4bf1acd29d5ae1e18716c6660c4733e1528caaafd758cb452cbb'
CONFIG = 'sha256:ef175de2e521ec563ec90af395db4e4c5f8103424a2aa2d085c868e13e1cfaa7'
SOURCES = {
    'v1/mp_observability/subscribers/metrics/lookup.py': '7c06623bf9588bb6f4f915a72bb94749c54c4b15ce7b8b34951de6b31086b173',
    'v1/mp_observability/subscribers/metrics/l1.py': 'f02c8c90e72b1b7f66b17b849fa59e8c0aa086f3a2124094415f80a5eb436ca4',
    'v1/mp_observability/subscribers/metrics/engine.py': '189253a08ed3cc97dcc3f6f01757daf8d5d79d443eca0fe61da91bfef3db4078',
    'v1/mp_observability/subscribers/metrics/mp_transfer.py': 'd40fd2775bfad3b6480b82d3da2b58ddca32ae4a8c51c76440e62d49b3bcecf8',
    'v1/mp_observability/otel_init.py': '342babfc21a065a91fa8c9c743d8ea16f41cdb0256d0f1b10cb0b181cc8e5b96',
    'integration/vllm/lmcache_mp_connector.py': 'bbb37a924bda5a881bd965f976e98c2792d3c32685483fa10a3c567d1ff240c1',
}
CONNECTOR = {'kv_connector': 'LMCacheMPConnector', 'kv_role': 'kv_both',
             'kv_connector_module_path': 'lmcache.integration.vllm.lmcache_mp_connector',
             'kv_connector_extra_config': {'lmcache.mp.host': 'tcp://127.0.0.1', 'lmcache.mp.port': 5555}}
QUERY = 'lmcache_mp_lookup_requested_tokens_total'
HIT = 'lmcache_mp_lookup_hit_tokens_total'
STORED = 'lmcache_mp_l1_write_chunks_total'
LOADED = 'lmcache_mp_num_chunks_loaded_total'
REQUESTS = 'vllm:request_success_total'
CASES = {'BC-KV-005'}
SPECS = {QUERY: 'counter', HIT: 'counter', STORED: 'counter', LOADED: 'counter'}


def validate_config(config):
    if config['image'] != IMAGE or config.get('source_revision') != REVISION or config.get('connector_mode') != 'mp':
        raise ValueError('reviewed LMCache image/source and MP mode required; legacy is not an alias')
    if (config['model'], config['revision']) != ('Qwen/Qwen3.8-27B-FP8', 'c3a462cc3d38cf351f3b26cbd763469eeb74ef81'):
        raise ValueError('the source-supported primary hybrid model is required')
    if config.get('kv_case') != 'BC-KV-005' or os.environ.get('CERT_CASE_ID', 'BC-KV-005') != 'BC-KV-005':
        raise ValueError('one selected LMCache case required')
    for key in ('owner', 'evidence_link'):
        if not isinstance(config.get(key), str) or not config[key].strip():
            raise ValueError('owner and evidence link required')
    flags = config['server_flags']
    forbidden = manifest_tool.FORBIDDEN | {'--served-model-name', '--api-key', '--no-enable-prefix-caching', '--disable-hybrid-kv-cache-manager', '--kv-transfer-config', '--mamba-cache-mode'}
    if '--enable-prefix-caching' not in flags or any(v.split('=')[0] in forbidden for v in flags):
        raise ValueError('MP hybrid recipe requires native prefix caching and HMA; align is fixed by the runner')
    if config.get('kv_connector') != CONNECTOR:
        raise ValueError('exact canonical MP connector required')
    if config.get('server_env') != {'LMCACHE_TRACK_USAGE': 'false'}:
        raise ValueError('freeze disabled usage telemetry without legacy environment settings')
    mp = config.get('mp_server')
    if (not isinstance(mp, dict) or set(mp) != {'chunk_size', 'l1_size_gb', 'separate_object_groups', 'eviction_policy'}
            or mp['chunk_size'] != 784 or mp['separate_object_groups'] is not True or mp['eviction_policy'] != 'LRU'
            or type(mp['l1_size_gb']) is not int or not 1 <= mp['l1_size_gb'] <= 128):
        raise ValueError('freeze the Qwen3.8 MP recipe: 784-token chunks and separate object groups')
    ports = [config.get(k) for k in ('port', 'proxy_port', 'metrics_port')]
    if any(type(p) is not int or not 1024 <= p <= 65535 for p in ports) or len(set(ports)) != 3:
        raise ValueError('distinct inference/proxy/MP-metrics ports required')
    if type(config.get('settle_seconds')) is not int or not 1 <= config['settle_seconds'] <= 10:
        raise ValueError('bounded settling interval required')
    _, seed = common.prompts(config, 'seed')
    _, reuse = common.prompts(config, 'reuse')
    _, unique = common.prompts(config, 'unique')
    _, prime_seed = common.prompts(config, 'prime-seed')
    _, prime_reuse = common.prompts(config, 'prime-reuse')
    if (len(seed) < 2 or len(reuse) < 2 or set(seed) != set(reuse) or len(set(unique)) != len(unique)
            or set(unique) & set(seed) or set(prime_seed) != set(prime_reuse)
            or set(prime_seed) & (set(seed) | set(unique))):
        raise ValueError('freeze disjoint priming, store/reuse and unique prompt cohorts')
    fit = common.validate_fit(config)
    if any(fit.get(k) != config[k] for k in ('server_env', 'kv_connector', 'connector_mode', 'mp_server')):
        raise ValueError('fit must bind the actual MP connector and server configuration')
    return fit


def observation(before, after, request_count, engine_before, engine_after):
    delta = common.deltas(before, after, SPECS)
    rows = {}
    for name in SPECS:
        values = [(labels, value) for (metric, labels), value in delta.items() if metric == name]
        if len(values) != 1:
            raise ValueError('ambiguous MP series; no worker, salt or object-group summation permitted')
        rows[name] = values[0]
    if rows[QUERY][0] != rows[HIT][0]:
        raise ValueError('MP lookup token counter labels do not match')
    query, hit, stored, loaded = [rows[name][1] for name in (QUERY, HIT, STORED, LOADED)]
    if hit > query:
        raise ValueError('hit-token delta exceeds requested-token delta')
    requests = common.deltas(engine_before, engine_after, {REQUESTS: 'counter'})
    workers = {tuple((k, v) for k, v in labels if k != 'finished_reason') for _, labels in requests}
    if len(workers) != 1 or sum(requests.values()) != request_count:
        raise ValueError('request population differs or multiple vLLM workers present')
    return {'requested_tokens': query, 'hit_tokens': hit, 'stored_chunks': stored, 'loaded_chunks': loaded,
            'requests': request_count, 'resident_occupancy': None, 'physical_tier': 'unknown'}


def runtime_source(container):
    script = """import hashlib,json,pathlib
import lmcache,lmcache._version as lv,vllm._version as vv
from lmcache.integration.vllm.lmcache_mp_connector import LMCacheMPConnector
from vllm.distributed.kv_transfer.kv_connector.v1.base import SupportsHMA
assert issubclass(LMCacheMPConnector,SupportsHMA)
p=pathlib.Path(lmcache.__file__).parent
files=SOURCES_PLACEHOLDER
pid=pathlib.Path('/tmp/kv-mp.pid').read_text().strip()
stat=pathlib.Path('/proc/'+pid+'/stat').read_text().split()
print(json.dumps({'lmcache_version':lv.__version__,'lmcache_commit':lv.__commit_id__,
'vllm_version':vv.__version__,'vllm_commit':vv.__commit_id__,
'sources':{f:hashlib.sha256((p/f).read_bytes()).hexdigest() for f in files},
'mp_pid':pid,'mp_start_ticks':stat[21]}))
""".replace('SOURCES_PLACEHOLDER', repr(list(SOURCES)))
    proc = subprocess.run(['docker', 'exec', container, 'python3', '-c', script], check=True, capture_output=True, text=True, timeout=60)
    observed = json.loads(proc.stdout)
    if (observed.get('lmcache_version') != '0.5.7' or observed.get('lmcache_commit') != 'g236f3eec'
            or observed.get('vllm_version') != '0.33.0' or observed.get('vllm_commit') != 'g4ba787ec9'
            or observed.get('sources') != SOURCES or not observed.get('mp_pid') or not observed.get('mp_start_ticks')):
        raise ValueError('runtime MP package/source/HMA connector mismatch')
    return observed


def reset_native(args, cohort):
    # Only vLLM's native prefix cache is reset, after requests have drained.
    # The separate MP process and its counters/cache remain in the same epoch.
    import http.client
    conn = http.client.HTTPConnection('127.0.0.1', args.port, timeout=10)
    try:
        conn.request('POST', '/reset_prefix_cache')
        response = conn.getresponse()
        body = response.read(4096)
        if response.status != 200:
            raise ValueError('native prefix reset failed')
        (args.out / 'evidence' / f'kv-{cohort}-native-reset.json').write_text(json.dumps(
            {'status': response.status, 'body': body.decode(), 'mp_cache_reset': False}) + '\n')
    finally:
        conn.close()


def start_server(args, config):
    fit = validate_config(config)
    actual_fit = common.verify_fit_host(fit)
    (args.out / 'evidence/kv-fit-runtime.json').write_text(json.dumps(actual_fit) + '\n')
    image = json.loads(subprocess.run(['docker', 'image', 'inspect', IMAGE], check=True, capture_output=True, text=True, timeout=30).stdout)[0]
    if image['Id'] != CONFIG or image.get('Architecture') != 'amd64' or image.get('Os') != 'linux' or IMAGE not in image.get('RepoDigests', []):
        raise ValueError('local image digest/config/platform differs')
    (args.out / 'evidence/kv-image-preflight.json').write_text(json.dumps(
        {'reference': IMAGE, 'config': image['Id'], 'platform': 'linux/amd64', 'repo_digests': image['RepoDigests']}) + '\n')
    command = ['docker', 'run', '-d', '--pull=never', '--name', args.container, '--label', f'metrum.cert.owner={args.container_owner}',
               '--gpus', 'device=' + os.environ['CUDA_VISIBLE_DEVICES'], '--ipc=host', '--env', 'CUDA_VISIBLE_DEVICES=0',
               '--env', 'VLLM_NO_USAGE_STATS=1', '--env', 'LMCACHE_TRACK_USAGE=false',
               '-p', f'127.0.0.1:{config["port"]}:8000', '-p', f'127.0.0.1:{config["metrics_port"]}:9090',
               '--entrypoint', '/bin/bash']
    cache = os.environ.get('CERT_HF_HUB_CACHE')
    if not cache or not Path(cache).is_dir():
        raise ValueError('offline model hub cache required')
    command += ['--mount', f'type=bind,src={Path(cache).resolve()},dst=/root/.cache/huggingface/hub,readonly', '--env', 'HF_HUB_OFFLINE=1']
    # Static shell program; all engine/model options remain positional argv.
    # Both processes live in the one owner-labelled container, so removing that
    # container reaps the MP service as well as vLLM. No RPC port is published.
    program = ('set -euo pipefail; '
               'lmcache server --host 127.0.0.1 --port 5555 --chunk-size 784 --separate-object-groups '
               '--l1-size-gb "$1" --eviction-policy LRU --prometheus-port 9090 & '
               'printf "%s\\n" "$!" > /tmp/kv-mp.pid; shift; exec vllm serve "$@"')
    command += [IMAGE, '-c', program, 'kv-mp', str(config['mp_server']['l1_size_gb']), config['model'],
                '--revision', config['revision'], '--host', '0.0.0.0', '--port', '8000', '--mamba-cache-mode', 'align',
                '--kv-transfer-config', json.dumps(CONNECTOR, sort_keys=True), *config['server_flags']]
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
    if os.environ.get('CERT_CASE_ID') != 'BC-KV-005':
        raise SystemExit('unknown CERT_CASE_ID for kv_lmcache_runner.py')
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('out', 'candidate', 'baseline', 'manifest'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--container', required=True)
    parser.add_argument('--container-owner', required=True)
    parser.add_argument('--start-server', action='store_true')
    args = parser.parse_args()
    config = json.loads(args.manifest.read_text(), object_pairs_hook=manifest_tool.unique_object)['lanes']['KVL']
    fit = validate_config(config)
    if args.start_server:
        start_server(args, config)
        return 0
    args.model, args.port, args.proxy_port = config['model'], config['port'], config['proxy_port']
    metrics_port = config['metrics_port']
    evidence = args.out / 'evidence'
    observed, passed = {}, False
    try:
        models = json.loads((evidence / 'models.json').read_text())
        if [row.get('id') for row in models.get('data', [])] != [config['model']]:
            raise ValueError('actual served model differs')
        binding = common.docker_json(args.container)
        if binding['image'] != CONFIG or binding['owner'] != args.container_owner or binding['restarts'] != 0:
            raise ValueError('owned image/epoch differs')
        source_binding = runtime_source(args.container)
        observed['runtime'] = source_binding
        observed['fit'] = fit
        (evidence / 'kv-runtime-binding.json').write_text(json.dumps(binding) + '\n')
        for prime in ['prime-seed', 'prime-reuse']:
            reset_native(args, prime)
            result = common.run_cohort(args, config, prime, metrics_port, 'lmcache057-mp', REVISION)
            common.check_usage_bound(result['usage'], fit['max_prompt_tokens'])
        observed['priming_excluded'] = ['prime-seed', 'prime-reuse']
        observed['phases'] = {}
        for cohort in ['seed', 'reuse', 'unique']:
            reset_native(args, cohort)
            time.sleep(config['settle_seconds'])
            quiet = common.fetch(metrics_port)
            engine_quiet = common.fetch(args.port)
            time.sleep(config['settle_seconds'])
            before = common.fetch(metrics_port)
            engine_before = common.fetch(args.port)
            if any(common.deltas(quiet, before, SPECS).values()) or any(common.deltas(engine_quiet, engine_before, {REQUESTS: 'counter'}).values()):
                raise ValueError('window was not quiescent')
            (evidence / f'kv-{cohort}-quiet.prom').write_text(quiet)
            (evidence / f'kv-{cohort}-before.prom').write_text(before)
            (evidence / f'kv-{cohort}-engine-quiet.prom').write_text(engine_quiet)
            (evidence / f'kv-{cohort}-engine-before.prom').write_text(engine_before)
            result = common.run_cohort(args, config, cohort, metrics_port, 'lmcache057-mp', REVISION)
            common.check_usage_bound(result['usage'], fit['max_prompt_tokens'])
            time.sleep(config['settle_seconds'])
            after = common.fetch(metrics_port)
            engine_after = common.fetch(args.port)
            (evidence / f'kv-{cohort}-engine-after.prom').write_text(engine_after)
            (evidence / f'kv-{cohort}-after.prom').write_text(after)
            time.sleep(config['settle_seconds'])
            settled = common.fetch(metrics_port)
            engine_settled = common.fetch(args.port)
            (evidence / f'kv-{cohort}-settled.prom').write_text(settled)
            (evidence / f'kv-{cohort}-engine-settled.prom').write_text(engine_settled)
            if any(common.deltas(after, settled, SPECS).values()) or any(common.deltas(engine_after, engine_settled, {REQUESTS: 'counter'}).values()):
                raise ValueError('asynchronous counters did not settle')
            if common.docker_json(args.container) != binding or runtime_source(args.container) != source_binding:
                raise ValueError('container epoch changed')
            capture = find_capture(result['rows'])
            if (not capture or capture.get('profile') != 'lmcache057_mp' or capture.get('reference_source_revision') != REVISION
                    or capture.get('failure') is not None or capture.get('missing_required_metrics')):
                raise ValueError('candidate source-matched profile unavailable')
            samples = [s for s in capture.get('samples', []) if s.get('metric') in SPECS]
            if {s['metric'] for s in samples} != {QUERY, HIT, STORED, LOADED} or any(s.get('value') is None or s.get('unknown') is not None
                    or s.get('unit') != ('tokens' if s['metric'] in (QUERY, HIT) else 'chunks') or s.get('expected_family_type') != 'counter' for s in samples):
                raise ValueError('complete token-counter coverage required')
            phase = observation(before, after, len(result['requests']), engine_before, engine_after)
            if cohort == 'seed' and phase['stored_chunks'] <= 0:
                raise ValueError('positive recorded store missing')
            if cohort == 'reuse' and (phase['hit_tokens'] <= 0 or phase['requested_tokens'] <= 0 or phase['loaded_chunks'] <= 0):
                raise ValueError('positive recorded connector reuse missing')
            # Source logger totals are independent of response usage. Responses
            # establish request completion; no physical transfer is inferred.
            if any(type(u.get('prompt_tokens')) is not int or u['prompt_tokens'] <= 0 for u in result['usage']):
                raise ValueError('independent response usage missing')
            observed['phases'][cohort] = phase
        passed = True
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as exc:
        observed['error'] = str(exc)
        observed['coordination_status'] = 'PENDING'
    gpu = (evidence / 'gpu.csv').read_text().strip().split(',', 1)
    record = {'ID': 'BC-KV-005', 'result': 'PASS' if passed else 'FAIL', 'candidate_sha': os.environ['CERT_SHA'], 'attempt': 1,
              'start_utc': os.environ['CERT_START_UTC'], 'end_utc': dt.datetime.now(dt.timezone.utc).isoformat(),
              'owner': config['owner'], 'evidence_link': config['evidence_link'], 'gpu_class': gpu[0].strip(), 'driver': gpu[1].strip(),
              'versions': {'server_image': IMAGE, 'source_revision': REVISION, 'connector_mode': 'mp',
                           'model': config['model'], 'model_revision': config['revision'],
                           'candidate_binary_sha256': manifest_tool.sha256_file(args.candidate),
                           'baseline_binary_sha256': manifest_tool.sha256_file(args.baseline), 'manifest_sha256': manifest_tool.sha256_file(args.manifest)},
              'observed_values': observed, 'thresholds': {'positive_store_chunk_delta': True, 'positive_reuse_delta': True,
                                                       'complete_lookup_token_coverage': True, 'positive_loaded_chunks': True, 'physical_tier': 'unknown'},
              'evidence_paths': [str(p.relative_to(args.out)) for p in evidence.iterdir() if p.is_file()]}
    (evidence / 'provisional-BC-KV-005.json').write_text(json.dumps(record, allow_nan=False) + '\n')
    return 0 if passed else 1


if __name__ == '__main__':
    raise SystemExit(main())
