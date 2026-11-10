# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Collect W against a real platform wrapper; external sign-off is separate."""
import argparse
import datetime as dt
import json
import os
from pathlib import Path
import subprocess
import sys
import time

import g6_checks
import g6_counter_runner
import l1_runner
import manifest_tool
import w_checks

ROOT = Path(__file__).resolve().parents[3]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('out', 'candidate', 'baseline', 'manifest'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--start-utc', required=True)
    args = parser.parse_args(); evidence = args.out / 'evidence'
    try:
        manifest, config = manifest_tool.load_manifest(args.manifest, 'W', args.candidate, args.baseline, os.environ['CUDA_VISIBLE_DEVICES'])
        versions = {'manifest_sha256': manifest_tool.sha256_file(args.manifest), 'candidate_binary_sha256': manifest_tool.sha256_file(args.candidate),
                    'baseline_binary_sha256': manifest_tool.sha256_file(args.baseline), 'server_image': config['image'],
                    'model': config['model'], 'model_revision': config['revision'],
                    'wrapper_sha256': config['wrapper']['sha256'], 'wrapper_version': config['wrapper']['version']}
        models = json.loads((evidence / 'models.json').read_text(encoding='utf-8'))
        if config['model'] not in {row.get('id') for row in models.get('data', []) if isinstance(row, dict)}:
            raise ValueError('W server model identity differs from frozen manifest')
        gpu = (evidence / 'gpu.csv').read_text(encoding='utf-8').strip().split(',', 1)
        if len(gpu) != 2 or 'H100' not in gpu[0] or not gpu[1].strip():
            raise ValueError('W requires the specified NVIDIA H100 and driver evidence')
        prompt = str(Path(config['prompt_sets']['default']['path']).resolve())
        url = f"http://127.0.0.1:{config['proxy_port']}/v1/chat/completions"
        count = 8; observed = {}; files = ['evidence/gpu.csv', 'evidence/binary-hashes.txt', 'evidence/models.json',
                                         'evidence/r4-image-preflight.json', 'evidence/r4-image-runtime.json']
        modes = (('adequate', 4096, []), ('strict', 128, config['strict_args']), ('safety', 128, config['safety_args']))
        for mode, cap, strict_args in modes:
            cli = evidence / f'w-{mode}-cli.jsonl'; output = evidence / f'w-{mode}-wrapper.json'
            trace = evidence / f'w-{mode}-trace.jsonl'; stdout = evidence / f'w-{mode}-stdout.txt'
            stderr = evidence / f'w-{mode}-stderr.txt'; proxy_err = evidence / f'w-{mode}-proxy-error.txt'
            command_file = evidence / f'w-{mode}-command.json'
            values = {'{candidate}': str(args.candidate.resolve()), '{url}': url, '{prompt}': prompt,
                      '{cli_log}': str(cli.resolve()), '{wrapper_log}': str(output.resolve()),
                      '{max_tokens}': str(cap), '{count}': str(count), '{model}': config['model']}
            command = [str(Path(config['wrapper']['path']).resolve())] + [values.get(x, x) for x in config['wrapper_args']] + strict_args
            command_file.write_text(json.dumps(command, indent=2) + '\n', encoding='utf-8')
            with proxy_err.open('wb') as error:
                process = subprocess.Popen([sys.executable, str(Path(__file__).with_name('trace_proxy.py')),
                    '--listen-port', str(config['proxy_port']), '--upstream-port', str(config['port']), '--out', str(trace)],
                    stdout=subprocess.DEVNULL, stderr=error, cwd=ROOT)
                try:
                    g6_counter_runner.wait_for_proxy(config['proxy_port'], process)
                    env = {key: os.environ[key] for key in ('PATH', 'HOME', 'LD_LIBRARY_PATH', 'LANG', 'LC_ALL', 'TMPDIR') if key in os.environ}
                    code = l1_runner.call(command, stdout, stderr, 3600, env=env)
                    deadline = time.monotonic() + 15
                    while time.monotonic() < deadline and process.poll() is None:
                        try:
                            if len(g6_checks.server_usage(trace.read_text(encoding='utf-8'))) == count: break
                        except (OSError, ValueError, KeyError, TypeError):
                            pass
                        time.sleep(0.1)
                finally:
                    process.terminate()
                    try: process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        process.kill(); process.wait(timeout=10)
            wrapper = json.loads(output.read_text(encoding='utf-8'), object_pairs_hook=manifest_tool.unique_object)
            observed[mode] = w_checks.evaluate(cli.read_text(encoding='utf-8'), trace.read_text(encoding='utf-8'), wrapper,
                count, mode, code, args.candidate.resolve(), url, prompt, cli.resolve(), config['model'])
            observed[mode]['wrapper_exit_code'] = code
            files += [str(path.relative_to(args.out)) for path in (cli, output, trace, stdout, stderr, proxy_err, command_file)]
        if (manifest_tool.sha256_file(args.manifest) != versions['manifest_sha256']
                or manifest_tool.sha256_file(args.candidate) != versions['candidate_binary_sha256']
                or manifest_tool.sha256_file(args.baseline) != versions['baseline_binary_sha256']
                or manifest_tool.sha256_file(config['wrapper']['path']) != versions['wrapper_sha256']
                or manifest_tool.sha256_file(prompt) != config['prompt_sets']['default']['sha256']):
            raise ValueError('frozen W input changed during collection')
        collection = {'candidate_sha': manifest['candidate_sha'], 'versions': versions, 'owner': config['owner'],
                      'evidence_link': config['evidence_link'], 'gpu_class': gpu[0].strip(), 'driver': gpu[1].strip(),
                      'start_utc': args.start_utc, 'collected_utc': dt.datetime.now(dt.timezone.utc).isoformat().replace('+00:00', 'Z'),
                      'attempt': int(os.environ.get('CERT_ATTEMPT', '1')), 'observed': observed,
                      'input': {'candidate': str(args.candidate.resolve()), 'baseline': str(args.baseline.resolve()),
                                'manifest': str(args.manifest.resolve()), 'wrapper': str(Path(config['wrapper']['path']).resolve()),
                                'prompt': prompt, 'prompt_sha256': config['prompt_sets']['default']['sha256'], 'url': url, 'count': count},
                      'files': {name: manifest_tool.sha256_file(args.out / name) for name in files}}
        path = evidence / 'w-collection.json'; path.write_text(json.dumps(collection, indent=2, allow_nan=False) + '\n', encoding='utf-8')
        print('W collection complete; platform team must review and externally sign off collection SHA256:', manifest_tool.sha256_file(path))
        return 0
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as exc:
        (evidence / 'w-collection-error.txt').write_text(str(exc) + '\n', encoding='utf-8')
        print(f'W collection failed: {exc}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    import signal
    def interrupted(signum, _frame): raise SystemExit(128 + signum)
    signal.signal(signal.SIGTERM, interrupted); signal.signal(signal.SIGINT, interrupted)
    sys.exit(main())
