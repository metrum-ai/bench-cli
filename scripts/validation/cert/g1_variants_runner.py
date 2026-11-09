#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Run GPU-003 against real server streams; no synthetic server responses."""
import argparse
import datetime as dt
import json
import subprocess
import sys
from pathlib import Path

import g1_variants_checks
import g6_counter_runner
import g7_runner
import l1_runner
import manifest_tool
import result_writer

ROOT = Path(__file__).resolve().parents[3]
TOOL = {'type': 'function', 'function': {'name': 'get_weather',
        'description': 'Get current weather for a named city.',
        'parameters': {'type': 'object', 'properties': {'city': {'type': 'string'}}, 'required': ['city']}}}


def run_cell(args, label, prompt, body, reorder):
    evidence = args.out / 'evidence'
    trace = evidence / f'{label}-trace.jsonl'
    data = evidence / f'{label}-requests.jsonl'
    # vLLM rejects n>1 with greedy sampling; the seed retains reproducibility.
    temperature = '0.1' if body.get('n', 1) > 1 else '0'
    command = [str(args.candidate), '--scenario', label, '--url', f'http://localhost:{args.proxy_port}/v1/chat/completions',
               '--api-key', 'dummy', '--num-requests', '4', '--concurrency', '1', '--prompts', str(prompt),
               '--mode', 'chat', '--streaming', '--model', args.model, '--data-log', str(data), '--max-tokens', '4096',
               '--temperature', temperature, '--seed', '42', '--warmup-requests', '0', '--request-timeout', '1200',
               '--extra-body-json', json.dumps(body), '--debug-log', str(evidence / f'{label}-debug.log'),
               '--error-log', str(evidence / f'{label}-error.log')]
    proxy_cmd = [sys.executable, str(Path(__file__).with_name('trace_proxy.py')), '--listen-port', str(args.proxy_port),
                 '--upstream-port', str(args.port), '--out', str(trace), *(['--reorder', reorder] if reorder else [])]
    (evidence / f'{label}-command.json').write_text(json.dumps({'command': command, 'proxy': proxy_cmd}, indent=2) + '\n', encoding='utf-8')
    with (evidence / f'{label}-proxy-error.txt').open('wb') as proxy_err:
        proxy = subprocess.Popen(proxy_cmd, stdout=subprocess.DEVNULL, stderr=proxy_err, cwd=ROOT)
        try:
            g6_counter_runner.wait_for_proxy(args.proxy_port, proxy)
            code = l1_runner.call(command, evidence / f'{label}-stdout.txt', evidence / f'{label}-stderr.txt', 1800)
            if code == 0:
                g7_runner.wait_for_trace(trace)
        finally:
            proxy.terminate()
            try: proxy.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proxy.kill(); proxy.wait(timeout=10)
    return data.read_text(encoding='utf-8'), trace.read_text(encoding='utf-8'), code


def tool_prompt(config):
    """Validate the same pinned tool input and server flags used by the runner."""
    flags = config['server_flags']
    if '--enable-auto-tool-choice' not in flags or not any(
        flags[i:i+2] == ['--tool-call-parser', 'hermes'] or flag == '--tool-call-parser=hermes'
        for i, flag in enumerate(flags)):
        raise ValueError('G1 requires pinned hermes tool parser and auto tool choice')
    tool = config['prompt_sets']['tool']
    prompt = Path(tool['path']).resolve(strict=True)
    if not prompt.is_file() or manifest_tool.sha256_file(prompt) != tool['sha256']:
        raise ValueError('tool prompt pin mismatch')
    return prompt


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
    files = ['evidence/gpu.csv', 'evidence/server.log', 'evidence/binary-hashes.txt']
    cells, errors = {}, []
    try:
        config = json.loads(args.manifest.read_text(encoding='utf-8'))['lanes']['G1']
        prompt_tool = tool_prompt(config)
        modes = [
            ('reasoning', args.prompt, {'chat_template_kwargs': {'enable_thinking': True}}, None, 'reasoning'),
            ('answer-only', args.prompt, {'chat_template_kwargs': {'enable_thinking': False}}, None, 'answer_only'),
            ('tool', prompt_tool, {'chat_template_kwargs': {'enable_thinking': False}, 'tools': [TOOL],
                                   'tool_choice': {'type': 'function', 'function': {'name': 'get_weather'}}}, None, 'tool'),
            ('multiple', args.prompt, {'chat_template_kwargs': {'enable_thinking': False}, 'n': 2}, 'unselected-first', 'unselected_before_selected'),
            ('answer-first', args.prompt, {'chat_template_kwargs': {'enable_thinking': True}}, 'answer-first', 'answer_before_reasoning'),
            ('same-event', args.prompt, {'chat_template_kwargs': {'enable_thinking': True}}, 'same-event', 'same_event'),
        ]
        for label, prompt, body, reorder, required in modes:
            try:
                data, trace, code = run_cell(args, 'g1-variants-' + label, prompt, body, reorder)
                cells[label] = g1_variants_checks.evaluate(data, trace, code, reorder, required)
            except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as exc:
                cells[label] = {'error': str(exc)}
                errors.append(label)
    except (OSError, ValueError, KeyError, TypeError) as exc:
        errors.append('setup')
        cells['setup'] = {'error': str(exc)}
    files.extend('evidence/' + path.name for path in evidence.glob('g1-variants-*') if path.is_file())
    record = {'ID': 'BC-GPU-003', 'result': 'FAIL' if errors else 'PASS', 'candidate_sha': args.sha, 'attempt': 1,
              'start_utc': args.start_utc, 'end_utc': dt.datetime.now(dt.timezone.utc).isoformat().replace('+00:00', 'Z'),
              'owner': args.owner, 'evidence_link': args.evidence_link, 'gpu_class': gpu[0].strip(),
              'driver': gpu[1].strip() if len(gpu) > 1 else '',
              'versions': {'server_image': args.image, 'model': args.model, 'model_revision': args.revision,
                           'candidate_binary_sha256': manifest_tool.sha256_file(args.candidate),
                           'baseline_binary_sha256': manifest_tool.sha256_file(args.baseline),
                           'manifest_sha256': manifest_tool.sha256_file(args.manifest)},
              'observed_values': {'cells': cells, 'failed_modes': errors},
              'thresholds': {'requests_per_cell': 4, 'required_event_population_min': 1, 'clock_error_seconds_max': 0.05},
              'evidence_paths': files}
    result_writer.write_record(args.out, record)
    return 1 if errors else 0


if __name__ == '__main__':
    sys.exit(main())
