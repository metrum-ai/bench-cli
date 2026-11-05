#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Prospective r3: two real 8B servers on one NVIDIA GPU; no mock fallback."""
import argparse
import datetime as dt
import json
import os
import secrets
import signal
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

import g5_endpoints_checks
import g6_counter_runner
import manifest_tool
import result_writer

ROOT = Path(__file__).resolve().parents[3]


def now():
    return dt.datetime.now(dt.timezone.utc).isoformat().replace('+00:00', 'Z')


def configuration(config):
    ports = [config.get('port'), config.get('proxy_port'), config.get('second_port'), config.get('second_proxy_port')]
    if any(not isinstance(p, int) or isinstance(p, bool) or not 1024 <= p <= 65535 for p in ports) or len(set(ports)) != 4:
        raise ValueError('four distinct pinned server/proxy ports are required')
    flags = config.get('server_flags', [])
    values = [flag.split('=', 1)[1] if flag.startswith('--gpu-memory-utilization=') else flags[i + 1]
              for i, flag in enumerate(flags) if flag.startswith('--gpu-memory-utilization=') or
              (flag == '--gpu-memory-utilization' and i + 1 < len(flags))]
    if values != ['0.45']:
        raise ValueError('both servers require exactly one --gpu-memory-utilization 0.45')
    return ports


def rows_so_far(path):
    try:
        lines = path.read_text(encoding='utf-8').splitlines()
        return [json.loads(line) for line in lines if line.strip()]
    except (OSError, json.JSONDecodeError):
        return []


def run(args, config):
    evidence = args.out / 'evidence'
    port, proxy_port, second_port, second_proxy = configuration(config)
    memory = subprocess.run(['nvidia-smi', '--id=' + os.environ['CUDA_VISIBLE_DEVICES'], '--query-gpu=memory.total',
                             '--format=csv,noheader,nounits'], capture_output=True, text=True, timeout=30, check=True)
    total_mib = int(memory.stdout.strip())
    if total_mib * 1024 * 1024 < 48_000_000_000:
        raise ValueError('GPU reports less than 48GB; no mock replacement')
    (evidence / 'g5-endpoints-memory.txt').write_text(memory.stdout, encoding='utf-8')
    first = (evidence / 'container-id.txt').read_text(encoding='utf-8').strip()
    if not first or any(c not in '0123456789abcdef' for c in first):
        raise ValueError('first real server identity missing')
    first_models = json.loads((evidence / 'models.json').read_text(encoding='utf-8'))
    if not any(row.get('id') == args.model for row in first_models.get('data', [])):
        raise ValueError('first server did not serve the pinned model')
    name = 'cert-g5-second-' + secrets.token_hex(12)
    docker = ['docker', 'run', '-d', '--name', name, '--gpus', 'device=' + os.environ['CUDA_VISIBLE_DEVICES'],
              '--ipc=host', '--env', 'CUDA_VISIBLE_DEVICES=0']
    if os.environ.get('HF_TOKEN'):
        docker += ['--env', 'HF_TOKEN']
    if os.environ.get('CERT_HF_HUB_CACHE'):
        cache = Path(os.environ['CERT_HF_HUB_CACHE']).resolve(strict=True)
        docker += ['--mount', f'type=bind,src={cache},dst=/root/.cache/huggingface/hub', '--env', 'HF_HUB_OFFLINE=1']
    docker += ['-p', f'127.0.0.1:{second_port}:8000', args.image, '--model', args.model, '--revision', args.revision,
               '--port', '8000', *config['server_flags']]
    (evidence / 'g5-endpoints-server-command.json').write_text(json.dumps(docker, indent=2) + '\n', encoding='utf-8')
    proxies, client = [], None
    cleanup = None
    # Cleanup is armed before docker run, even if the command is interrupted.
    try:
        started = subprocess.run(docker, capture_output=True, text=True, timeout=120, check=True)
        second = started.stdout.strip()
        if not second or any(c not in '0123456789abcdef' for c in second):
            raise ValueError('second real server identity missing')
        (evidence / 'g5-endpoints-container-id.txt').write_text(second + '\n', encoding='utf-8')
        deadline = time.monotonic() + 1800
        while time.monotonic() < deadline:
            try:
                with urllib.request.urlopen(f'http://localhost:{second_port}/v1/models', timeout=5) as response:
                    models = json.load(response)
                if any(x.get('id') == args.model for x in models.get('data', [])):
                    break
            except (OSError, ValueError):
                pass
            if subprocess.run(['docker', 'inspect', '-f', '{{.State.Running}}', name], capture_output=True, text=True, timeout=30).stdout.strip() != 'true':
                raise ValueError('second real server stopped before readiness')
            time.sleep(2)
        else:
            raise ValueError('second real server readiness timeout')
        (evidence / 'g5-endpoints-models.json').write_text(json.dumps(models, indent=2) + '\n', encoding='utf-8')
        for label, listen, upstream in (('survivor', proxy_port, port), ('victim', second_proxy, second_port)):
            trace = evidence / f'g5-endpoints-{label}-trace.jsonl'
            err = (evidence / f'g5-endpoints-{label}-proxy-error.txt').open('wb')
            proxy = subprocess.Popen([sys.executable, str(Path(__file__).with_name('trace_proxy.py')), '--listen-port', str(listen),
                                      '--upstream-port', str(upstream), '--out', str(trace)], stdout=subprocess.DEVNULL, stderr=err, cwd=ROOT)
            proxies.append((proxy, err))
            g6_counter_runner.wait_for_proxy(listen, proxy)
        endpoints = evidence / 'g5-endpoints.yaml'
        endpoints.write_text(f'- name: survivor\n  url: http://localhost:{proxy_port}/v1/chat/completions\n  api_key: dummy\n'
                             f'- name: victim\n  url: http://localhost:{second_proxy}/v1/chat/completions\n  api_key: dummy\n', encoding='utf-8')
        data = evidence / 'g5-endpoints-requests.jsonl'
        command = [str(args.candidate), '--endpoints-file', str(endpoints), '--load-balancer', 'round-robin',
                   '--scenario', 'g5-endpoints', '--num-requests', '64', '--concurrency', '2', '--prompts', str(args.prompt),
                   '--mode', 'chat', '--streaming', '--model', args.model, '--data-log', str(data), '--max-tokens', '128',
                   '--temperature', '0', '--seed', '42', '--warmup-requests', '0', '--request-timeout', '60', '--fail-on-error',
                   '--debug-log', str(evidence / 'g5-endpoints-debug.log'), '--error-log', str(evidence / 'g5-endpoints-error.log'),
                   '--extra-body-json', '{"chat_template_kwargs":{"enable_thinking":false}}']
        (evidence / 'g5-endpoints-command.json').write_text(json.dumps(command, indent=2) + '\n', encoding='utf-8')
        with (evidence / 'g5-endpoints-stdout.txt').open('wb') as out, (evidence / 'g5-endpoints-stderr.txt').open('wb') as err:
            client = subprocess.Popen(command, stdout=out, stderr=err, cwd=ROOT, start_new_session=True)
            deadline = time.monotonic() + 600
            while time.monotonic() < deadline and client.poll() is None:
                measured = rows_so_far(data)
                answered = {r.get('endpoint') for r in measured if '.request.' in r.get('schema_version', '')
                            and r.get('phase') == 'measure' and r.get('answer_completed') is True and r.get('error') is None}
                if answered >= {'survivor', 'victim'}:
                    break
                time.sleep(.1)
            else:
                raise ValueError('both endpoints did not answer before the interruption')
            start_utc, start_ns = now(), time.monotonic_ns()
            killed = subprocess.run(['docker', 'kill', second], capture_output=True, text=True, timeout=30)
            end_ns, end_utc = time.monotonic_ns(), now()
            interruption = {'kill_start_utc': start_utc, 'kill_end_utc': end_utc, 'kill_start_ns': start_ns,
                            'kill_end_ns': end_ns, 'kill_exit': killed.returncode}
            (evidence / 'g5-endpoints-interruption.json').write_text(json.dumps(interruption, indent=2) + '\n', encoding='utf-8')
            if killed.returncode:
                raise ValueError('real second server kill failed')
            code = client.wait(timeout=1800)
        traces = {name: (evidence / f'g5-endpoints-{name}-trace.jsonl').read_text(encoding='utf-8') for name in ('survivor', 'victim')}
        return g5_endpoints_checks.verify(data.read_text(encoding='utf-8'), traces, interruption, code)
    finally:
        if client is not None and client.poll() is None:
            try:
                os.killpg(client.pid, signal.SIGTERM)
                client.wait(timeout=10)
            except ProcessLookupError:
                client.wait(timeout=10)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(client.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                client.wait(timeout=10)
        for proxy, err in proxies:
            proxy.terminate()
            try:
                proxy.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proxy.kill(); proxy.wait(timeout=10)
            err.close()
        logs = subprocess.run(['docker', 'logs', name], capture_output=True, text=True, timeout=30)
        (evidence / 'g5-endpoints-server.log').write_text(logs.stdout + logs.stderr, encoding='utf-8')
        cleanup = subprocess.run(['docker', 'rm', '-f', name], capture_output=True, text=True, timeout=30)
        (evidence / 'g5-endpoints-cleanup.txt').write_text(cleanup.stdout + cleanup.stderr, encoding='utf-8')
        if cleanup.returncode:
            raise ValueError('failed to remove owned second server')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('out', 'candidate', 'baseline', 'manifest', 'prompt'):
        parser.add_argument('--' + name, type=Path, required=True)
    for name in ('sha', 'model', 'revision', 'image', 'start-utc', 'owner', 'evidence-link'):
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args()
    evidence = args.out / 'evidence'
    gpu = (evidence / 'gpu.csv').read_text(encoding='utf-8').strip().split(',', 1)
    try:
        manifest = json.loads(args.manifest.read_text(encoding='utf-8'))
        observed = run(args, manifest['lanes']['G5'])
        passed = True
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as exc:
        observed, passed = {'error': str(exc)}, False
    files = ['evidence/' + name for name in ('gpu.csv', 'server.log', 'binary-hashes.txt', 'container-id.txt', 'models.json')]
    files += [str(p.relative_to(args.out)) for p in evidence.glob('g5-endpoints-*') if p.is_file()]
    record = {'ID': 'BC-GPU-015', 'result': 'PASS' if passed else 'FAIL', 'candidate_sha': args.sha, 'attempt': 1,
              'start_utc': args.start_utc, 'end_utc': now(), 'owner': args.owner, 'evidence_link': args.evidence_link,
              'gpu_class': gpu[0].strip(), 'driver': gpu[1].strip() if len(gpu) > 1 else '',
              'versions': {'server_image': args.image, 'model': args.model, 'model_revision': args.revision,
                           'candidate_binary_sha256': manifest_tool.sha256_file(args.candidate),
                           'baseline_binary_sha256': manifest_tool.sha256_file(args.baseline),
                           'manifest_sha256': manifest_tool.sha256_file(args.manifest)},
              'observed_values': observed, 'thresholds': {'minimum_gpu_memory_bytes': 48_000_000_000,
                  'gpu_memory_utilization_each': 0.45, 'real_servers': 2}, 'evidence_paths': files}
    result_writer.write_record(args.out, record)
    return 0 if passed else 1


if __name__ == '__main__':
    raise SystemExit(main())
