# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Bounded local exporter intake and independent same-series counter oracle.

A snapshot never proves physical residency. Worker labels are retained privately
in evidence; reported tiers are never combined or converted to physical bytes.
"""
import base64
import hashlib
import json
import math
import os
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

import g2_usage_checks
import g6_counter_runner
import g7_runner
import l1_runner
import metrics_oracle
import manifest_tool

MAX_PAGE = 4 * 1024 * 1024


def fetch(port, path='/metrics'):
    # No caller-supplied URLs, redirects, proxy credentials or authentication.
    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, *args, **kwargs):
            return None
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    with opener.open(f'http://127.0.0.1:{port}{path}', timeout=5) as response:
        raw = response.read(MAX_PAGE + 1)
        if len(raw) > MAX_PAGE:
            raise ValueError('exporter response exceeds limit')
        return raw.decode('utf-8')


def parse_page(text, specs):
    """Require explicit TYPE, exact names and unique full label identities."""
    result, types = {}, {}
    for line in text.splitlines():
        line = line.strip()
        if line.startswith('# TYPE '):
            parts = line.split()
            if len(parts) != 4:
                raise ValueError('malformed TYPE')
            name, kind = parts[2:]
            if name in types and types[name] != kind:
                raise ValueError('conflicting TYPE')
            types[name] = kind
            continue
        if not line or line.startswith('#'):
            continue
        match = metrics_oracle.SAMPLE.fullmatch(line)
        if not match:
            if any(line.startswith(n) for n in specs):
                raise ValueError('malformed selected metric')
            continue
        name = match[1]
        if name not in specs:
            continue
        expected = specs[name]
        observed = types.get(name, types.get(name.removesuffix('_total')))
        if observed != expected:
            raise ValueError('missing or incompatible TYPE')
        value = float(match[3])
        if not math.isfinite(value) or value < 0:
            raise ValueError('invalid metric value')
        if expected == 'counter' and (value >= 2**53 or not value.is_integer()):
            raise ValueError('invalid integer counter precision')
        if expected == 'gauge' and value > 1:
            raise ValueError('invalid hit-rate ratio')
        key = (name, metrics_oracle.parse_labels(match[2]))
        if key in result:
            raise ValueError('duplicate series identity')
        result[key] = value
        if len(result) > 256:
            raise ValueError('too many series')
    if any(not any(k[0] == n for k in result) for n in specs):
        raise ValueError('required metric missing')
    return result


def deltas(before, after, specs):
    a, b = parse_page(before, specs), parse_page(after, specs)
    if a.keys() != b.keys():
        raise ValueError('series or worker identity changed')
    result = {}
    for key in a:
        if specs[key[0]] == 'counter':
            if b[key] < a[key]:
                raise ValueError('counter reset')
            result[key] = b[key] - a[key]
    return result


def pinned(item):
    path = Path(item['path']).resolve(strict=True)
    if manifest_tool.sha256_file(path) != item['sha256']:
        raise ValueError('input hash mismatch')
    return path


def prompts(config, cohort):
    path = pinned(config['prompt_sets'][cohort])
    rows = [json.loads(s, object_pairs_hook=manifest_tool.unique_object) for s in path.read_text().splitlines() if s.strip()]
    if not rows or len(rows) > 256 or any(set(r) != {'prompt'} or not isinstance(r['prompt'], str) or not r['prompt'] for r in rows):
        raise ValueError('cohort requires bounded nonempty plain prompts')
    return path, [r['prompt'] for r in rows]


def validate_fit(config):
    fit = json.loads(pinned(config['fit']).read_text(), object_pairs_hook=manifest_tool.unique_object)
    if (fit.get('image') != config['image'] or fit.get('model') != config['model']
            or fit.get('revision') != config['revision'] or fit.get('server_flags') != config['server_flags']
            or fit.get('gpu_class') not in ('NVIDIA H100 80GB HBM3', 'NVIDIA H100 PCIe', 'NVIDIA RTX PRO 6000 Blackwell Server Edition')
            or fit.get('runtime_compatible') is not True):
        raise ValueError('fit must bind compatible image/model/flags and supported GPU')
    for key in ('gpu_memory_bytes', 'peak_gpu_memory_bytes', 'host_available_bytes', 'host_required_bytes', 'max_prompt_tokens'):
        if type(fit.get(key)) is not int or fit[key] <= 0:
            raise ValueError('fit requires measured positive memory and prompt bounds')
    if fit['gpu_memory_bytes'] <= fit['peak_gpu_memory_bytes'] or fit['host_available_bytes'] <= fit['host_required_bytes']:
        raise ValueError('fit has no measured memory headroom')
    if not isinstance(fit.get('gpu_uuid'), str) or not fit['gpu_uuid'].startswith('GPU-'):
        raise ValueError('fit requires actual GPU identity')
    if not isinstance(fit.get('driver_version'), str) or not fit['driver_version'].strip():
        raise ValueError('fit requires the demonstrated GPU driver')
    pinned(fit['evidence'])
    return fit


def verify_fit_host(fit):
    gpu = subprocess.run(['nvidia-smi', '--id=' + os.environ['CUDA_VISIBLE_DEVICES'],
                          '--query-gpu=uuid,name,memory.total,memory.free,driver_version', '--format=csv,noheader,nounits'],
                         check=True, text=True, capture_output=True, timeout=30).stdout.strip().split(',')
    if (len(gpu) != 5 or gpu[0].strip() != fit['gpu_uuid'] or gpu[1].strip() != fit['gpu_class']
            or int(gpu[2].strip()) * 1024**2 < fit['gpu_memory_bytes']
            or int(gpu[3].strip()) * 1024**2 < fit['peak_gpu_memory_bytes'] or gpu[4].strip() != fit['driver_version']):
        raise ValueError('actual GPU/driver/free memory differs from measured fit')
    memory = dict(line.split(':', 1) for line in Path('/proc/meminfo').read_text().splitlines())
    available = int(memory['MemAvailable'].split()[0]) * 1024
    if available <= fit['host_required_bytes']:
        raise ValueError('actual host memory has no demonstrated cache headroom')


def docker_json(container):
    proc = subprocess.run(['docker', 'inspect', container], check=True, capture_output=True, text=True, timeout=30)
    data = json.loads(proc.stdout)
    if len(data) != 1 or data[0]['State'].get('Running') is not True:
        raise ValueError('owned server not running')
    row = data[0]
    return {'id': row['Id'], 'image': row['Image'], 'started': row['State']['StartedAt'],
            'restarts': row['RestartCount'], 'owner': row['Config']['Labels'].get('metrum.cert.owner')}


def verify_trace(data, trace, expected, model):
    rows = [json.loads(s) for s in data.splitlines() if s.strip()]
    requests = [r for r in rows if '.request.' in r.get('schema_version', '') and r.get('phase') == 'measure']
    if len(requests) != len(expected) or any(r.get('error') is not None for r in requests):
        raise ValueError('incomplete successful request population')
    summaries = [r for r in rows if '.summary.' in r.get('schema_version', '')]
    if len(summaries) != 1 or summaries[0].get('attempted') != len(expected) or summaries[0].get('successes') != len(expected):
        raise ValueError('candidate summary coverage differs')
    groups = {}
    for e in (json.loads(s) for s in trace.splitlines() if s.strip()):
        groups.setdefault(e['request'], []).append(e)
    if len(groups) != len(expected):
        raise ValueError('trace request count differs')
    expected_hashes = sorted(hashlib.sha256(p.encode()).hexdigest() for p in expected)
    observed_hashes, usage = [], []
    for events in groups.values():
        sent = [e for e in events if e['kind'] == 'send']
        complete = [e for e in events if e['kind'] == 'complete']
        identity = [e for e in events if e['kind'] == 'request_identity']
        if len(sent) != 1 or len(complete) != 1 or len(identity) != 1:
            raise ValueError('trace lacks exact request boundaries')
        entry = identity[0]
        if (entry.get('model') != model or entry.get('streaming') is not True
                or entry.get('max_tokens') != 32 or entry.get('temperature') != 0
                or entry.get('seed') != 42 or entry.get('thinking') is not False):
            raise ValueError('trace request settings differ')
        observed_hashes.append(entry['prompt_sha256'])
        original = b''.join(base64.b64decode(e['bytes_b64'], validate=True) for e in events if e['kind'] == 'chunk')
        forwarded = b''.join(base64.b64decode(e['bytes_b64'], validate=True) for e in events if e['kind'] == 'forwarded_chunk')
        if original != forwarded:
            raise ValueError('proxy changed response bytes')
        blocks = [frame['usage'] for frame in g2_usage_checks.parse_sse(original) if isinstance(frame.get('usage'), dict)]
        if len(blocks) != 1:
            raise ValueError('exactly one server usage block required')
        usage.append(blocks[0])
    if sorted(observed_hashes) != expected_hashes:
        raise ValueError('trace prompt identities differ')
    return rows, requests, usage


def candidate_command(args, config, cohort, metrics_port, profile, revision):
    path, expected = prompts(config, cohort)
    label = f'kv-{cohort}'
    evidence = args.out / 'evidence'
    command = [str(args.candidate), '--url', f'http://localhost:{args.proxy_port}/v1/chat/completions',
               '--api-key', 'dummy', '--model', args.model, '--prompts', str(path), '--num-requests', str(len(expected)),
               '--concurrency', '1', '--warmup-requests', '0', '--streaming', '--max-tokens', '32',
               '--temperature', '0', '--seed', '42', '--request-timeout', '120', '--fail-on-error',
               '--extra-body-json', '{"chat_template_kwargs":{"enable_thinking":false}}',
               '--cache-metrics-url', f'http://127.0.0.1:{metrics_port}/metrics', '--cache-metrics-profile', profile,
               '--cache-metrics-source-revision', revision, '--data-log', str(evidence / f'{label}.jsonl')]
    return label, command, expected


def run_cohort(args, config, cohort, metrics_port, profile, revision):
    label, command, expected = candidate_command(args, config, cohort, metrics_port, profile, revision)
    evidence = args.out / 'evidence'
    trace_path = evidence / f'{label}-trace.jsonl'
    proxy_command = [sys.executable, str(Path(__file__).with_name('trace_proxy.py')), '--listen-port', str(args.proxy_port),
                     '--upstream-port', str(args.port), '--out', str(trace_path), '--record-request-identity']
    (evidence / f'{label}-command.json').write_text(json.dumps({'candidate': command, 'proxy': proxy_command}) + '\n')
    with (evidence / f'{label}-proxy-error.txt').open('wb') as err:
        proxy = subprocess.Popen(proxy_command, stderr=err, stdout=subprocess.DEVNULL)
        try:
            g6_counter_runner.wait_for_proxy(args.proxy_port, proxy)
            code = l1_runner.call(command, evidence / f'{label}-stdout.txt', evidence / f'{label}-stderr.txt', 600)
            if code:
                raise ValueError(f'candidate exited {code}')
            g7_runner.wait_for_trace(trace_path)
        finally:
            proxy.terminate()
            try:
                proxy.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proxy.kill()
                proxy.wait(timeout=10)
    trace = trace_path.read_text()
    data = (args.out / 'evidence' / f'{label}.jsonl').read_text()
    rows, requests, usage = verify_trace(data, trace, expected, args.model)
    return {'rows': rows, 'requests': requests, 'usage': usage}
