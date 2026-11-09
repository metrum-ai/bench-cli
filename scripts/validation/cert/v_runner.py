#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""BC-REASON-005: run seven real thinking-enabled pinned-image VLM streams."""
import argparse
import datetime as dt
import json
import subprocess
import sys
from pathlib import Path

import g4_modality_runner
import manifest_tool
import result_writer
import v_checks


def case_inputs(args, config):
    flags = config['server_flags']
    if args.model != 'Qwen/Qwen3-VL-8B-Thinking' or '--no-enable-prefix-caching' not in flags or not any(
        flags[i:i+2] == ['--reasoning-parser', 'qwen3'] or flag == '--reasoning-parser=qwen3'
        for i, flag in enumerate(flags)):
        raise ValueError('case requires pinned Qwen3-VL-8B-Thinking with the qwen3 parser and no prefix cache')
    binary = g4_modality_runner.pinned_file(config, 'vlm_binary', 'metrum-ai-bench-cli-vlm', args.candidate)
    image = g4_modality_runner.pinned_file(config, 'vlm_image')
    g4_modality_runner.image_prompts(args.prompt, image)
    return binary, image


def benchmark_command(args, vlm_binary, data, evidence):
    """Assemble the real VLM product invocation."""
    return [str(vlm_binary), '--url', f'http://localhost:{args.proxy_port}/v1/chat/completions', '--api-key', 'dummy',
            '--scenario', 'v-reasoning', '--num-requests', '7', '--concurrency', '1', '--prompts', str(args.prompt),
            '--model', args.model, '--data-log', str(data), '--streaming', '--max-tokens', '4096', '--temperature', '0',
            '--warmup-requests', '0', '--seed', '42', '--request-timeout', '1200',
            '--extra-body-json', '{"chat_template_kwargs":{"enable_thinking":true}}',
            '--debug-log', str(evidence / 'v-reasoning-debug.log'), '--error-log', str(evidence / 'v-reasoning-error.log')]


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
    vlm_binary = image = None
    try:
        config = json.loads(args.manifest.read_text(encoding='utf-8'))['lanes']['V']
        vlm_binary, image = case_inputs(args, config)
        models = json.loads((evidence / 'models.json').read_text(encoding='utf-8'))
        if not any(row.get('id') == args.model for row in models.get('data', [])):
            raise ValueError('real server does not serve the pinned model')
        data = evidence / 'v-reasoning-requests.jsonl'
        bench = benchmark_command(args, vlm_binary, data, evidence)
        trace = g4_modality_runner.run_proxy(args, 'v-reasoning', bench, image=True)
        observed = v_checks.evaluate(data.read_text(encoding='utf-8'), trace, 0, manifest_tool.sha256_file(image), args.model)
        passed = True
    except (OSError, ValueError, KeyError, TypeError, AttributeError, subprocess.SubprocessError) as exc:
        observed, passed = {'error': str(exc)}, False
    files = ['evidence/' + name for name in ('gpu.csv', 'server.log', 'binary-hashes.txt', 'container-id.txt', 'models.json')]
    files += [str(path.relative_to(args.out)) for path in evidence.glob('v-reasoning-*') if path.is_file()]
    record = {'ID': 'BC-REASON-005', 'result': 'PASS' if passed else 'FAIL', 'candidate_sha': args.sha, 'attempt': 1,
              'start_utc': args.start_utc, 'end_utc': dt.datetime.now(dt.timezone.utc).isoformat().replace('+00:00', 'Z'),
              'owner': args.owner, 'evidence_link': args.evidence_link, 'gpu_class': gpu[0].strip(),
              'driver': gpu[1].strip() if len(gpu) > 1 else '',
              'versions': {'server_image': args.image, 'model': args.model, 'model_revision': args.revision,
                           'candidate_binary_sha256': manifest_tool.sha256_file(args.candidate),
                           'baseline_binary_sha256': manifest_tool.sha256_file(args.baseline),
                           'vlm_binary_sha256': manifest_tool.sha256_file(vlm_binary) if vlm_binary else None,
                           'image_sha256': manifest_tool.sha256_file(image) if image else None,
                           'manifest_sha256': manifest_tool.sha256_file(args.manifest)},
              'observed_values': observed, 'thresholds': {'measured': 7, 'completed': 7, 'reasoning_streams_min': 1,
                                                         'image_count_min': 1, 'clock_error_seconds_max': 0.05},
              'evidence_paths': files}
    result_writer.write_record(args.out, record)
    return 0 if passed else 1


if __name__ == '__main__':
    sys.exit(main())
