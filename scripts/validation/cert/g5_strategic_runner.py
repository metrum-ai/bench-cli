#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Real strategic multi-stage restart and export-failure development runner."""
import argparse
import datetime as dt
import json
import os
import subprocess
import sys
import time
from pathlib import Path

import g3_runner
import g5_strategic_checks
import g6_counter_runner
import l1_runner
import manifest_tool
import result_writer

ROOT = Path(__file__).resolve().parents[3]


def invocations(args, strategic, evidence):
    """Build the production restart and export-failure commands."""
    prefix = 'g5-strategic-'
    trace = evidence / (prefix + 'trace.jsonl')
    csv_path = evidence / (prefix + 'records.csv')
    command = [str(strategic), '--url', f'http://localhost:{args.proxy_port}/v1/chat/completions', '--api-key', 'dummy',
               '--model', args.model, '--kind', 'chat', '--streaming', '--requests-per-stage', '4', '--sweep', '1,2',
               '--prompts', str(args.prompt), '--max-tokens', '128', '--temperature', '0', '--seed', '42',
               '--extra-body-json', '{"chat_template_kwargs":{"enable_thinking":false}}', '--timeout-seconds', '600',
               '--fail-on-error', '--csv', str(csv_path), '--html', str(evidence / (prefix + 'report.html')),
               '--ndjson', str(evidence / (prefix + 'telemetry.jsonl'))]
    proxy = [sys.executable, str(Path(__file__).with_name('trace_proxy.py')), '--listen-port', str(args.proxy_port),
             '--upstream-port', str(args.port), '--out', str(trace), '--retry-seconds', '240']
    failure_dir = evidence / (prefix + 'invalid-csv-destination')
    failure = [str(strategic), '--url', f'http://localhost:{args.proxy_port}/v1/chat/completions', '--api-key', 'dummy',
               '--model', args.model, '--kind', 'chat', '--streaming', '--requests-per-stage', '1', '--sweep', '1',
               '--prompts', str(args.prompt), '--max-tokens', '128', '--temperature', '0', '--seed', '42',
               '--extra-body-json', '{"chat_template_kwargs":{"enable_thinking":false}}', '--timeout-seconds', '600',
               '--fail-on-error', '--csv', str(failure_dir)]
    return command, proxy, failure, trace, csv_path, failure_dir


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
    proxy = client = None
    try:
        config = json.loads(args.manifest.read_text(encoding='utf-8'))['lanes']['G5']
        import g4_modality_runner
        strategic = g4_modality_runner.pinned_file(config, 'strategic_binary', 'metrum-ai-bench-cli-strategic', args.candidate)
        container = (evidence / 'container-id.txt').read_text(encoding='utf-8').strip()
        if not container or any(c not in '0123456789abcdef' for c in container):
            raise ValueError('owned real server identity missing')
        prefix = 'g5-strategic-'
        command, proxy_command, failure_command, trace, csv_path, failure_dir = invocations(args, strategic, evidence)
        (evidence / (prefix + 'command.json')).write_text(json.dumps({'benchmark': command, 'proxy': proxy_command}, indent=2) + '\n', encoding='utf-8')
        with (evidence / (prefix + 'proxy-error.txt')).open('wb') as perr, (evidence / (prefix + 'summary.json')).open('wb') as out, (evidence / (prefix + 'stderr.txt')).open('wb') as err:
            proxy = subprocess.Popen(proxy_command, stdout=subprocess.DEVNULL, stderr=perr, cwd=ROOT)
            g6_counter_runner.wait_for_proxy(args.proxy_port, proxy)
            client = subprocess.Popen(command, stdout=out, stderr=err, cwd=ROOT, start_new_session=True)
            deadline = time.monotonic() + 900
            while time.monotonic() < deadline and client.poll() is None:
                events = g3_runner.events(trace)
                completed = {e.get('request') for e in events if e.get('kind') == 'complete'}
                active = {e.get('request') for e in events if e.get('kind') == 'chunk'} - completed
                if len(completed) >= 4 and active:
                    break
                time.sleep(.05)
            else:
                raise ValueError('a completed first stage and active second-stage stream were not observed')
            start_ns = time.monotonic_ns()
            killed = subprocess.run(['docker', 'kill', container], capture_output=True, text=True, timeout=30)
            end_ns = time.monotonic_ns()
            started = subprocess.run(['docker', 'start', container], capture_output=True, text=True, timeout=30)
            interruption = {'kill_start_ns': start_ns, 'kill_end_ns': end_ns, 'kill_exit': killed.returncode, 'start_exit': started.returncode}
            (evidence / (prefix + 'interruption.json')).write_text(json.dumps(interruption, indent=2) + '\n', encoding='utf-8')
            if killed.returncode or started.returncode:
                raise ValueError('owned server restart failed')
            code = client.wait(timeout=1800)
        # A separate successful real one-request stage has a deliberately invalid
        # export destination; JSON must already be emitted when that export fails.
        failure_dir.mkdir()
        (evidence / (prefix + 'export-failure-command.json')).write_text(json.dumps(failure_command, indent=2) + '\n', encoding='utf-8')
        fail_out, fail_err = evidence / (prefix + 'export-failure.json'), evidence / (prefix + 'export-failure-stderr.txt')
        failure_code = l1_runner.call(failure_command, fail_out, fail_err, 900)
        observed = g5_strategic_checks.verify((evidence / (prefix + 'summary.json')).read_text(encoding='utf-8'),
                                             csv_path.read_text(encoding='utf-8'), trace.read_text(encoding='utf-8'),
                                             interruption, code, fail_out.read_text(encoding='utf-8'), failure_code)
        passed = True
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, subprocess.SubprocessError) as exc:
        observed, passed = {'error': str(exc)}, False
    finally:
        if client is not None and client.poll() is None:
            g3_runner.stop_client(client)
        if proxy is not None:
            proxy.terminate()
            try: proxy.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proxy.kill(); proxy.wait(timeout=10)
    files = ['evidence/' + name for name in ('gpu.csv', 'server.log', 'binary-hashes.txt', 'container-id.txt')]
    files += [str(p.relative_to(args.out)) for p in evidence.glob('g5-strategic-*') if p.is_file()]
    record = {'ID': 'BC-GPU-013', 'result': 'PASS' if passed else 'FAIL', 'candidate_sha': args.sha, 'attempt': 1,
              'start_utc': args.start_utc, 'end_utc': dt.datetime.now(dt.timezone.utc).isoformat().replace('+00:00', 'Z'),
              'owner': args.owner, 'evidence_link': args.evidence_link, 'gpu_class': gpu[0].strip(),
              'driver': gpu[1].strip() if len(gpu) > 1 else '',
              'versions': {'server_image': args.image, 'model': args.model, 'model_revision': args.revision,
                           'candidate_binary_sha256': manifest_tool.sha256_file(args.candidate),
                           'baseline_binary_sha256': manifest_tool.sha256_file(args.baseline),
                           'manifest_sha256': manifest_tool.sha256_file(args.manifest)},
              'observed_values': observed, 'thresholds': {'stages': [1, 2], 'requests_per_stage': 4,
                           'rate_tolerance_pct': 1, 'real_restart': True, 'failed_export_check': True}, 'evidence_paths': files}
    result_writer.write_record(args.out, record)
    return 0 if passed else 1


if __name__ == '__main__':
    raise SystemExit(main())
