#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Real LLM/VLM/strategic rate comparisons; never substitute mock servers."""
import argparse
import datetime as dt
import json
import os
import secrets
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

import g4_modality_checks
import g6_counter_runner
import g7_runner
import l1_runner
import manifest_tool
import r_runner
import result_writer

ROOT = Path(__file__).resolve().parents[3]


def pinned_file(config, key, expected_name=None, candidate=None):
    item = config[key]
    path = Path(item['path']).resolve(strict=True)
    if not path.is_file() or manifest_tool.sha256_file(path) != item['sha256']:
        raise ValueError(f'{key} pin differs')
    if expected_name and (path != candidate.resolve().parent / expected_name or not os.access(path, os.X_OK)):
        raise ValueError(f'{key} must be an executable beside the candidate')
    return path


def image_prompts(prompt, image):
    rows = [json.loads(line) for line in prompt.read_text(encoding='utf-8').splitlines() if line.strip()]
    if not rows:
        raise ValueError('pinned VLM prompt set is empty')
    for row in rows:
        paths = row.get('image_urls', [row.get('image_url')])
        if not isinstance(paths, list) or len(paths) != 1 or not isinstance(paths[0], str):
            raise ValueError('each VLM prompt requires one pinned local image')
        path = paths[0].removeprefix('file://')
        if not Path(path).is_absolute() or Path(path).resolve(strict=True) != image:
            raise ValueError('VLM image must be the pinned local file; remote inputs are forbidden')


def run_proxy(args, label, command, image=False, env=None, cwd=None):
    evidence = args.out / 'evidence'
    trace = evidence / f'{label}-trace.jsonl'
    proxy_cmd = [sys.executable, str(Path(__file__).with_name('trace_proxy.py')), '--listen-port', str(args.proxy_port),
                 '--upstream-port', str(args.port), '--out', str(trace), *(['--record-image-identity'] if image else [])]
    (evidence / f'{label}-command.json').write_text(json.dumps({'command': command, 'proxy': proxy_cmd}, indent=2) + '\n', encoding='utf-8')
    with (evidence / f'{label}-proxy-error.txt').open('wb') as perr:
        proxy = subprocess.Popen(proxy_cmd, stdout=subprocess.DEVNULL, stderr=perr, cwd=ROOT, env=env)
        try:
            g6_counter_runner.wait_for_proxy(args.proxy_port, proxy)
            code = l1_runner.call(command, evidence / f'{label}-stdout.txt', evidence / f'{label}-stderr.txt', 1800, env=env, cwd=cwd)
            if code != 0:
                raise ValueError(f'{label} exited {code}')
            g7_runner.wait_for_trace(trace)
        finally:
            proxy.terminate()
            try: proxy.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proxy.kill(); proxy.wait(timeout=10)
    return trace.read_text(encoding='utf-8')


def strategic_command(args, binary, label, model, prompt, kind):
    evidence = args.out / 'evidence'
    csv_path, ndjson = evidence / f'{label}.csv', evidence / f'{label}-telemetry.jsonl'
    command = [str(binary), '--url', f'http://localhost:{args.proxy_port}/v1/chat/completions', '--api-key', 'dummy',
               '--model', model, '--kind', kind, '--streaming', '--requests-per-stage', '4', '--sweep', '1',
               '--prompts', str(prompt), '--max-tokens', '4096', '--temperature', '0', '--seed', '42',
               '--extra-body-json', '{"chat_template_kwargs":{"enable_thinking":true}}', '--timeout-seconds', '1200',
               '--csv', str(csv_path), '--ndjson', str(ndjson)]
    return command, csv_path, ndjson


def strategic(args, binary, label, model, prompt, kind, env=None, cwd=None):
    evidence = args.out / 'evidence'
    command, csv_path, ndjson = strategic_command(args, binary, label, model, prompt, kind)
    trace = run_proxy(args, label, command, image=(kind == 'vlm'), env=env, cwd=cwd)
    return {'summary': (evidence / f'{label}-stdout.txt').read_text(encoding='utf-8'),
            'csv': csv_path.read_text(encoding='utf-8'), 'telemetry': ndjson.read_text(encoding='utf-8'), 'trace': trace}


def modality_inputs(config, candidate):
    """Validate and assemble the pinned modality inputs used by this runner."""
    strategic_bin = pinned_file(config, 'strategic_binary', 'metrum-ai-bench-cli-strategic', candidate)
    vlm_bin = pinned_file(config, 'vlm_binary', 'metrum-ai-bench-cli-vlm', candidate)
    image = pinned_file(config, 'vlm_image')
    prompt = pinned_file(config['prompt_sets'], 'vlm')
    image_prompts(prompt, image)
    vlm = config['vlm']
    import r4_model_pins
    r4_model_pins.check(vlm, 'V')
    flags = vlm.get('server_flags')
    if not isinstance(flags, list) or '--no-enable-prefix-caching' not in flags or not any(
        flags[i:i+2] == ['--reasoning-parser', 'qwen3'] or flag == '--reasoning-parser=qwen3' for i, flag in enumerate(flags)):
        raise ValueError('pinned compatible qwen3 parser and disabled prefix caching required')
    if any(not isinstance(flag, str) or flag.split('=', 1)[0] in manifest_tool.FORBIDDEN for flag in flags):
        raise ValueError('invalid VLM server flags')
    return strategic_bin, vlm_bin, image, prompt, vlm, flags


def vlm_command(args, binary, prompt, model, data, telemetry):
    """Assemble the actual G4 VLM product invocation."""
    evidence = args.out / 'evidence'
    return [str(binary), '--url', f'http://localhost:{args.proxy_port}/v1/chat/completions', '--api-key', 'dummy',
            '--scenario', 'g4-modality-vlm', '--num-requests', '4', '--concurrency', '1', '--prompts', str(prompt),
            '--model', model, '--data-log', str(data), '--streaming', '--max-tokens', '4096', '--temperature', '0',
            '--warmup-requests', '0', '--seed', '42', '--request-timeout', '1200', '--slo', 'e2e=600',
            '--extra-body-json', '{"chat_template_kwargs":{"enable_thinking":true}}', '--ndjson', str(telemetry),
            '--debug-log', str(evidence / 'g4-modality-vlm-debug.log'), '--error-log', str(evidence / 'g4-modality-vlm-error.log')]


def run(args, config):
    evidence = args.out / 'evidence'
    strategic_bin, vlm_bin, image, prompt, vlm, flags = modality_inputs(config, args.candidate)
    models = json.loads((evidence / 'models.json').read_text(encoding='utf-8'))
    if not any(row.get('id') == args.model for row in models.get('data', [])):
        raise ValueError('first server did not serve the pinned LLM model')
    llm_ndjson = evidence / 'g4-modality-llm-telemetry.jsonl'
    llm_data, llm_trace, code, _ = r_runner.run_cell(args, 'g4-modality-llm', args.candidate, 4, 4096, True,
                                                       ['--slo', 'e2e=600', '--ndjson', str(llm_ndjson)], concurrency=1)
    if code:
        raise ValueError('live LLM run failed')
    llm_strategic = strategic(args, strategic_bin, 'g4-modality-llm-strategic', args.model, args.prompt, 'chat')
    first = (evidence / 'container-id.txt').read_text(encoding='utf-8').strip()
    if not first or any(c not in '0123456789abcdef' for c in first):
        raise ValueError('owned LLM server identity missing')
    stop = subprocess.run(['docker', 'stop', '--time', '10', first], capture_output=True, text=True, timeout=30)
    if stop.returncode:
        raise ValueError('could not stop owned LLM server before VLM')
    name = 'cert-g4-vlm-' + secrets.token_hex(12)
    command = ['docker', 'run', '-d', '--name', name, '--gpus', 'device=' + os.environ['CUDA_VISIBLE_DEVICES'],
               '--ipc=host', '--env', 'CUDA_VISIBLE_DEVICES=0']
    if os.environ.get('HF_TOKEN'): command += ['--env', 'HF_TOKEN']
    if os.environ.get('CERT_HF_HUB_CACHE'):
        cache = Path(os.environ['CERT_HF_HUB_CACHE']).resolve(strict=True)
        command += ['--mount', f'type=bind,src={cache},dst=/root/.cache/huggingface/hub', '--env', 'HF_HUB_OFFLINE=1']
    command += ['-p', f'127.0.0.1:{args.port}:8000', vlm['image'], '--model', vlm['model'], '--revision', vlm['revision'], '--port', '8000', *flags]
    (evidence / 'g4-modality-vlm-server-command.json').write_text(json.dumps(command, indent=2) + '\n', encoding='utf-8')
    try:
        started = subprocess.run(command, capture_output=True, text=True, timeout=120, check=True)
        identifier = started.stdout.strip()
        if not identifier or any(c not in '0123456789abcdef' for c in identifier):
            raise ValueError('real VLM server identity missing')
        (evidence / 'g4-modality-vlm-container-id.txt').write_text(identifier + '\n', encoding='utf-8')
        deadline = time.monotonic() + 1800
        while time.monotonic() < deadline:
            try:
                with urllib.request.urlopen(f'http://localhost:{args.port}/v1/models', timeout=5) as response:
                    models = json.load(response)
                if any(row.get('id') == vlm['model'] for row in models.get('data', [])):
                    break
            except (OSError, ValueError):
                pass
            if subprocess.run(['docker', 'inspect', '-f', '{{.State.Running}}', name], capture_output=True, text=True, timeout=30).stdout.strip() != 'true':
                raise ValueError('VLM server stopped before readiness')
            time.sleep(2)
        else:
            raise ValueError('real VLM server readiness timeout')
        (evidence / 'g4-modality-vlm-models.json').write_text(json.dumps(models, indent=2) + '\n', encoding='utf-8')
        data, telemetry = evidence / 'g4-modality-vlm-requests.jsonl', evidence / 'g4-modality-vlm-telemetry.jsonl'
        bench = vlm_command(args, vlm_bin, prompt, vlm['model'], data, telemetry)
        trace = run_proxy(args, 'g4-modality-vlm', bench, image=True)
        vlm_strategic = strategic(args, strategic_bin, 'g4-modality-vlm-strategic', vlm['model'], prompt, 'vlm')
        return g4_modality_checks.evaluate(llm_data, llm_trace, llm_ndjson.read_text(encoding='utf-8'), llm_strategic,
                                           data.read_text(encoding='utf-8'), trace, telemetry.read_text(encoding='utf-8'),
                                           vlm_strategic, manifest_tool.sha256_file(image), vlm['model'])
    finally:
        logs = subprocess.run(['docker', 'logs', name], capture_output=True, text=True, timeout=30)
        (evidence / 'g4-modality-vlm-server.log').write_text(logs.stdout + logs.stderr, encoding='utf-8')
        cleanup = subprocess.run(['docker', 'rm', '-f', name], capture_output=True, text=True, timeout=30)
        (evidence / 'g4-modality-vlm-cleanup.txt').write_text(cleanup.stdout + cleanup.stderr, encoding='utf-8')
        if cleanup.returncode:
            raise ValueError('failed to remove owned VLM server')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('out', 'candidate', 'baseline', 'manifest', 'prompt'):
        parser.add_argument('--' + name, type=Path, required=True)
    for name in ('sha', 'model', 'revision', 'image', 'start-utc', 'owner', 'evidence-link'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--port', type=int, required=True)
    parser.add_argument('--proxy-port', type=int, required=True)
    args = parser.parse_args()
    evidence = args.out / 'evidence'
    gpu = (evidence / 'gpu.csv').read_text(encoding='utf-8').strip().split(',', 1)
    config = {}
    try:
        config = json.loads(args.manifest.read_text(encoding='utf-8'))['lanes']['G4']
        observed = run(args, config)
        passed = True
    except (OSError, ValueError, KeyError, TypeError, AttributeError, subprocess.SubprocessError) as exc:
        observed, passed = {'error': str(exc)}, False
    for field in ('vlm', 'vlm_binary', 'strategic_binary', 'vlm_image'):
        if not isinstance(config.get(field), dict):
            config[field] = {}
    files = ['evidence/' + name for name in ('gpu.csv', 'server.log', 'binary-hashes.txt', 'container-id.txt', 'models.json')]
    files += [str(p.relative_to(args.out)) for p in evidence.glob('g4-modality-*') if p.is_file()]
    record = {'ID': 'BC-GPU-011', 'result': 'PASS' if passed else 'FAIL', 'candidate_sha': args.sha, 'attempt': 1,
              'start_utc': args.start_utc, 'end_utc': dt.datetime.now(dt.timezone.utc).isoformat().replace('+00:00', 'Z'),
              'owner': args.owner, 'evidence_link': args.evidence_link, 'gpu_class': gpu[0].strip(),
              'driver': gpu[1].strip() if len(gpu) > 1 else '',
              'versions': {'server_image': args.image, 'model': args.model, 'model_revision': args.revision,
                           'candidate_binary_sha256': manifest_tool.sha256_file(args.candidate),
                           'baseline_binary_sha256': manifest_tool.sha256_file(args.baseline),
                           'manifest_sha256': manifest_tool.sha256_file(args.manifest),
                           'vlm_model': config.get('vlm', {}).get('model'),
                           'vlm_model_revision': config.get('vlm', {}).get('revision'),
                           'vlm_server_image': config.get('vlm', {}).get('image'),
                           'vlm_binary_sha256': config.get('vlm_binary', {}).get('sha256'),
                           'strategic_binary_sha256': config.get('strategic_binary', {}).get('sha256'),
                           'image_sha256': config.get('vlm_image', {}).get('sha256')},
              'observed_values': observed, 'thresholds': {'requests_per_modality': 4, 'rate_tolerance_pct': 1,
                           'clock_uncertainty_s': .05, 'actual_image_and_thinking_required': True}, 'evidence_paths': files}
    result_writer.write_record(args.out, record)
    return 0 if passed else 1


if __name__ == '__main__':
    raise SystemExit(main())
