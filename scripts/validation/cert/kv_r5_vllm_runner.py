#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""vLLM r5 APC, exclusive native CPU offload, sessions and pressure gates."""
import json
import math
import re
import statistics
import sys

import kv_r5_metrics as metrics
import kv_r5_runtime as runtime

IMAGE = 'vllm/vllm-openai@sha256:d825baf9ce4b1b6bfc5ecc7b79880c7e6748c65a72d697ce3d47607aafa84c6e'
CHILD = 'sha256:91ee972ab751fb7180bc71c175865e36b091686600dca783d52b2511399810b3'
REVISION = '4ba787ec9959e566d3b587a796dff07673324da5'
MODEL = 'Qwen/Qwen3.8-27B-FP8'
MODEL_REVISION = 'c3a462cc3d38cf351f3b26cbd763469eeb74ef81'
METRICS_PATH = '/metrics'
ALLOW_RESET = True
CASES = ('BC-KV-001', 'BC-KV-002', 'BC-KV-008', 'BC-KV-009')
LIMITS = dict(zip(CASES, (None, 7200, None, None)))
CRITERIA = {
    'BC-KV-001': {'paired_requests_min': 16, 'server_cached_tokens_positive': True,
                  'positive_same_series_queries_and_hits': True, 'cli_delta_equals_oracle': True},
    'BC-KV-002': {'completed_cpu_load_bytes_positive': True, 'gpu_prefix_hit_tokens': 0,
                  'rehit_ttft_less_than_cold': True, 'independent_target_gpu_eviction': True,
                  'exclusive_settled_window': True, 'total_seconds_max': 7200},
    'BC-KV-008': {'sessions_min': 4, 'turns_per_session_min': 3, 'exact_trace_order': True,
                  'dedicated_warmup_excluded': True, 'each_stage_starts_cold': True},
    'BC-KV-009': {'ordered_pressure_and_rehit': True, 'independent_eviction': True,
                  'same_series_accounting': True, 'physical_tier': 'unknown'},
}
QUERY = 'vllm:prefix_cache_queries_total'
HIT = 'vllm:prefix_cache_hits_total'
LOAD = 'vllm:kv_offload_load_bytes_total'
APC = {QUERY: ('counter', 'tokens'), HIT: ('counter', 'tokens')}
SOURCE_FILES = {
    'v1/metrics/loggers.py': 'a17291f044251f1586457b6bd1542f98660724dd07a426d61ce852186b95b9e1',
    'distributed/kv_transfer/kv_connector/v1/offloading/metrics.py': 'd01f41fcbbf9476188432e32449c9dd2d2a2c4ba6d32f06b42ba3246fe6e1c57',
    'distributed/kv_transfer/kv_connector/v1/offloading/worker.py': '489e59c3b968c0f6a01d54f821917c5481a81bbbc8687a24a684da00efbb70cc',
}


def validate(data):
    if (data['model'], data['model_revision'], data['served_model']) != (MODEL, MODEL_REVISION, MODEL):
        raise ValueError('vLLM r5 model pin mismatch')
    if data['metrics_port'] != data['port'] or data['metrics_internal_port'] != 8000:
        raise ValueError('vLLM metrics endpoint mismatch')
    flags = data['server_flags']
    if '--enable-prefix-caching' not in flags or any(runtime.server_option_name(a) == '--no-enable-prefix-caching' for a in flags):
        raise ValueError('APC must be explicitly enabled')
    if runtime.server_overrides(flags, {'--model', '--revision', '--served-model-name', '--host', '--port',
                                       '--tokenizer', '--config', '--hf-overrides', '--model-impl'}):
        raise ValueError('server identity override')
    seq_flags = [a for a in flags if a.startswith('--') and '--max-num-seqs'.startswith(runtime.server_option_name(a))]
    if 'H100' in data['gpu_class'] and (seq_flags != ['--max-num-seqs']
            or any(a.startswith('--max-num-seqs=') for a in flags)
            or flags[flags.index('--max-num-seqs')+1] != '256'):
        raise ValueError('H100 requires --max-num-seqs 256')
    names = [p['name'] for p in data['phases']]
    if data['case'] == 'BC-KV-008':
        for phase in data['phases']:
            requests = phase['expected_requests']
            if (phase.get('reset_prefix_cache') is not True or not any(r['warmup'] for r in requests)
                    or not any(not r['warmup'] for r in requests)):
                raise ValueError('each session stage requires reset, dedicated warmup and measured requests')
            first = next(i for i, r in enumerate(requests) if not r['warmup'])
            if any(r['warmup'] for r in requests[first:]):
                raise ValueError('session warmup must finish before measured requests')
    expected = {'BC-KV-001': ['cold', 'warm', 'unique'],
                'BC-KV-002': ['cold', 'populate', 'pressure', 'rehit'],
                'BC-KV-009': ['cold', 'populate', 'warm', 'pressure', 'rehit', 'control']}
    if data['case'] in expected and names != expected[data['case']]:
        raise ValueError('required phase order missing')
    if data['case'] != 'BC-KV-008' and any(p.get('reset_prefix_cache') for p in data['phases'][1:]):
        raise ValueError('reset across reuse/pressure epoch is forbidden')
    if data['case'] in ('BC-KV-002', 'BC-KV-009'):
        import kv_r5_eviction_observer as observer
        observer.validate(data)
    if data['case'] == 'BC-KV-002' and ('--kv-offloading-size' not in flags or '--kv-offloading-backend' not in flags):
        raise ValueError('native CPU offload configuration required')
    if data['case'] == 'BC-KV-002' and flags[flags.index('--kv-offloading-backend')+1] != 'native':
        raise ValueError('the native CPU offload gate requires the native backend')
    for phase in data['phases']:
        args = phase['argv']
        for option, value in (('--cache-metrics-profile', 'vllm4ba-apc'),
                              ('--cache-metrics-source-revision', REVISION),
                              ('--cache-metrics-url', 'http://127.0.0.1:'+str(data['port'])+'/metrics')):
            if args.count(option) != 1 or args[args.index(option)+1] != value:
                raise ValueError('source-matched product telemetry required')
        if '--cache-phase-required' not in args:
            raise ValueError('required product phase analysis absent')


def server(data):
    options, env = [], []
    if data['case'] in ('BC-KV-002', 'BC-KV-009'):
        import kv_r5_eviction_observer as observer
        options = observer.server_options()
    if data['case'] in ('BC-KV-002', 'BC-KV-008', 'BC-KV-009'):
        env = ['--env', 'VLLM_SERVER_DEV_MODE=1']
    return [*env, IMAGE, '--model', '/model', '--served-model-name', MODEL,
            '--host', '0.0.0.0', '--port', '8000', *options, *data['server_flags']]


def proxy_args(data, phase):
    if data['case'] != 'BC-KV-008':
        return []
    first = next(i for i, row in enumerate(phase['expected_requests']) if not row['warmup'])+1
    prior = sum(len(p['expected_requests']) for p in data['phases'][:data['phases'].index(phase)])
    return ['--session-first-measured', str(first), '--session-completed-before', str(prior+first-1),
            '--session-model', data['served_model']]


def preflight(instance):
    needed = dict(SOURCE_FILES)
    if instance.data['case'] in ('BC-KV-002', 'BC-KV-009'):
        import kv_r5_eviction_observer as observer
        needed.update(observer.SOURCE_FILES)
    code = ('import importlib.util,importlib.metadata,json,pathlib; '
            'root=pathlib.Path(importlib.util.find_spec("vllm").origin).parent; '
            'print(json.dumps({"version":importlib.metadata.version("vllm"),'
            '"files":{str(root/p):h for p,h in '+repr(needed)+'.items()}}))')
    package = json.loads(instance.docker('exec', instance.name, 'python3', '-c', code).stdout)
    runtime.save(instance.out/'vllm-package.json', package)
    if not re.fullmatch(r'0\.33\.0(?:\+[a-zA-Z0-9.]+)?', package['version']):
        raise ValueError('actual vLLM package is not stable 0.33.0')
    expected = package['files']
    if instance.data['case'] not in ('BC-KV-002', 'BC-KV-009'):
        needed = {'v1/metrics/loggers.py': SOURCE_FILES['v1/metrics/loggers.py']}
    for relative, digest in needed.items():
        paths = [p for p, h in expected.items() if p.endswith('/'+relative) and h == digest]
        if len(paths) != 1 or instance.data['source_files'].get(paths[0]) != digest:
            raise ValueError('actual import source slice is not pinned')


def before_phase(instance, phase, directory):
    if instance.data['case'] in ('BC-KV-002', 'BC-KV-009'):
        import kv_r5_eviction_observer as observer
        observer.before_phase(instance, phase, directory)


def after_phase(instance, phase, directory):
    if instance.data['case'] not in ('BC-KV-002', 'BC-KV-009'):
        return
    import kv_r5_eviction_observer as observer
    observer.after_phase(instance, phase, directory)


def percentile(values, p):
    values = sorted(values)
    index = (len(values)-1)*p
    lower = math.floor(index)
    return values[lower]+(values[math.ceil(index)]-values[lower])*(index-lower)


def cohort(rows, usage):
    groups = {'cold': [], 'partial': [], 'full': []}
    for row, server in zip(rows, usage):
        if row['warmup'] == 'true':
            continue
        cached = server.get('prompt_tokens_details', {}).get('cached_tokens')
        prompt = server['prompt_tokens']
        if type(cached) is not int or not 0 <= cached <= prompt or prompt <= 0:
            raise ValueError('server cached-token observation unavailable')
        evidence = json.loads(row['cache_evidence'])
        if (evidence.get('cached_tokens'), evidence.get('prompt_tokens'), evidence.get('cached_status'), evidence.get('prompt_status'),
            evidence.get('source'), evidence.get('units'), evidence.get('invalid_report_seen'), evidence.get('conflicting_reports')) != (
            cached, prompt, 'reported', 'reported', 'openai-chat-usage.v1:/prompt_tokens_details/cached_tokens', 'tokens', False, False):
            raise ValueError('product cached-token provenance disagrees with server')
        group = 'cold' if cached == 0 else 'full' if cached == prompt else 'partial'
        groups[group].append(float(row['ttft_s']))
    return {name: {'n': len(values), 'median_s': statistics.median(values) if values else None,
                   'p95_s': percentile(values, .95) if values else None} for name, values in groups.items()}


def accounting(phase, compare_product=True):
    specs = dict(APC)
    if phase['plan'].get('native_cpu_load'):
        specs[LOAD] = ('counter', 'bytes')
    rows = metrics.deltas(phase['before'], phase['after'], specs)
    query, hit = metrics.single(rows, QUERY), metrics.single(rows, HIT)
    if query <= 0 or hit > query:
        raise ValueError('invalid same-window hit/query accounting')
    labels = [r['labels'] for r in rows if r['metric'] in APC]
    if labels[0] != labels[1]:
        raise ValueError('hit/query worker identities differ')
    if not compare_product:
        return rows  # outer session bracket explicitly includes held-out warmup
    summary = runtime.load(phase['directory']/'stdout.json')
    reports = summary['config']['native_cache_phase_analysis']
    if len(reports) != 1:
        raise ValueError('one product phase required per external bracket')
    report = reports[0]['delta']
    if report['window_unknown'] is not None or report['continuity'] == 'unknown':
        raise ValueError('product counter phase is Unknown')
    for name in APC:
        independent = next(r for r in rows if r['metric'] == name)
        product = [r for r in report['series'] if r['metric'] == name]
        if len(product) != 1 or product[0]['unknown'] is not None:
            raise ValueError('product counter coverage incomplete')
        for key in ('start', 'end', 'delta'):
            if product[0][key] != independent[key]:
                raise ValueError('product phase differs from independent source snapshots')
    return rows


def evaluate(data, phases):
    observed = {}
    decoded = {}
    for phase in phases:
        rows, usage = runtime.product_rows(phase)
        decoded[phase['plan']['name']] = (rows, usage)
        observed[phase['plan']['name']] = {'cohorts': cohort(rows, usage),
            'series': accounting(phase, compare_product=data['case'] != 'BC-KV-008'),
            'counter_scope': 'including_warmup' if data['case'] == 'BC-KV-008' else 'measured'}
    by_name = {p['plan']['name']: p for p in phases}
    if data['case'] == 'BC-KV-001':
        plans = [p['plan']['expected_requests'] for p in phases]
        hashes = [[r['messages_sha256'] for r in plan if not r['warmup']] for plan in plans]
        if len(hashes[0]) < 16 or hashes[0] != hashes[1] or len(set(hashes[2])) != len(hashes[0]) or set(hashes[0]) & set(hashes[2]):
            raise ValueError('paired shared/unique workloads do not match')
        options = [r['options'] for plan in plans for r in plan if not r['warmup']]
        if any(o != options[0] for o in options):
            raise ValueError('incomparable generation settings')
        warm = observed['warm']
        if metrics.single(warm['series'], HIT) <= 0 or warm['cohorts']['partial']['n']+warm['cohorts']['full']['n'] < 1:
            raise ValueError('positive reuse not observed')
    elif data['case'] == 'BC-KV-008':
        import kv_r5_session_cold as cold
        completed_before = 0
        for phase in phases:
            if phase['plan'].get('reset_prefix_cache') is not True or not (phase['directory']/'reset.json').is_file():
                raise ValueError('each session stage requires its frozen cold reset')
            cold.reset_ack(phase['directory']/'reset.json')
            requests = phase['plan']['expected_requests']
            measured = [r for r in requests if not r['warmup']]
            warmup = [r for r in requests if r['warmup']]
            ids = {r['session_id'] for r in measured}
            if None in ids or len(ids) < 4 or not warmup or ids & {r['session_id'] for r in warmup}:
                raise ValueError('dedicated session warmup or measured sessions missing')
            rows, usage = decoded[phase['plan']['name']]
            first_measured = next(i for i, r in enumerate(requests) if not r['warmup'])
            if usage[first_measured].get('prompt_tokens_details', {}).get('cached_tokens') != 0:
                raise ValueError('stage cold marker contradicted by server')
            observed[phase['plan']['name']]['first_measured_counter_window'] = cold.verify(phase, data['served_model'], completed_before)
            completed_before += len(requests)
            for session in ids:
                turns = [r['turn'] for r in measured if r['session_id'] == session]
                if len(turns) < 3 or turns != list(range(turns[0], turns[0]+len(turns))):
                    raise ValueError('duplicate or missing session turn')
    else:
        prefix = data['target_prefix_sha256']
        target_phases = ['cold', 'populate', 'rehit']
        if data['case'] == 'BC-KV-009':
            target_phases.insert(2, 'warm')
        target_rows = []
        for name in target_phases:
            requests = [r for r in by_name[name]['plan']['expected_requests'] if not r['warmup']]
            if len(requests) != 1:
                raise ValueError('one frozen target A request required in each target phase')
            target_rows.append(requests[0])
        target = target_rows[0]
        if any((r['messages_sha256'], r['options']) != (target['messages_sha256'], target['options']) for r in target_rows):
            raise ValueError('target A wire identity/settings changed across eviction and rehit')
        pressure = by_name['pressure']['plan']['expected_requests']
        if any(r['messages_sha256'] == target['messages_sha256'] for r in pressure):
            raise ValueError('pressure sequence contains target A')
        if data['case'] == 'BC-KV-009':
            control = [r for r in by_name['control']['plan']['expected_requests'] if not r['warmup']]
            if len(control) != 1 or control[0]['messages_sha256'] == target['messages_sha256'] or control[0]['options'] != target['options']:
                raise ValueError('distinct comparable control B request required')
        import kv_r5_eviction_observer as observer
        proof = observer.verify(data, phases)
        if not runtime.SHA.fullmatch(prefix):
            raise ValueError('frozen tokenized target prefix identity required')
        rehit = observed['rehit']['series']
        if data['case'] == 'BC-KV-002':
            if metrics.single(rehit, LOAD) <= 0 or metrics.single(rehit, HIT) != 0:
                raise ValueError('native CPU exclusive-window byte/hit conditions failed')
            cold_rows, _ = decoded['cold']
            rehit_rows, _ = decoded['rehit']
            if len(cold_rows) != 1 or len(rehit_rows) != 1 or float(rehit_rows[0]['ttft_s']) >= float(cold_rows[0]['ttft_s']):
                raise ValueError('rehit TTFT is not strictly below cold TTFT')
        else:
            if metrics.single(observed['warm']['series'], HIT) <= 0:
                raise ValueError('warm reuse not established before pressure')
            if proof.get('pressure_geometry_verified') is not True or proof.get('rehit_miss_accounting_verified') is not True:
                raise ValueError('pressure geometry or independent rehit/miss evidence missing')
        observed['exclusive_window'] = proof
    return {'phases': observed, 'physical_cache_state': 'unknown', 'speedup_threshold': None}


if __name__ == '__main__':
    raise SystemExit(runtime.main(sys.modules[__name__]))
