# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""W wrapper contract and external sign-off CPU fixtures; not GPU evidence."""
import datetime as dt
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import manifest_tool
import result_writer
import w_checks
import w_finalize


def command(mode):
    return ['/candidate', '--url', 'http://localhost', '--api-key', 'dummy', '--prompts', '/prompt', '--data-log', '/cli', '--model', 'model',
            '--num-requests', '2', '--max-tokens', '4096' if mode == 'adequate' else '128', '--mode', 'chat',
            '--warmup-requests', '0', '--streaming', '--extra-body-json', '{"chat_template_kwargs":{"enable_thinking":true}}'] + (
            [] if mode == 'adequate' else ['--strict-reasoning-truncation' if mode == 'strict' else '--ci-safety'])


def rows(mode):
    result = []
    for i in range(2):
        truncated = mode != 'adequate' and i == 1
        result.append({'seq': i, 'schema_version': 'metrum-ai-bench-cli.request.v5', 'ttft_s': .1,
                       'ttft_visible_answer_s': None if truncated else .2, 'ttft_source': 'stream',
                       'outcome': 'reasoning_truncated' if truncated else 'answer', 'answer_completed': not truncated,
                       'usage_missing': False, 'error': {'kind': 'reasoning_truncated'} if truncated else None})
    return result


def summary(mode):
    trunc = int(mode != 'adequate')
    return {'attempted': 2, 'answered_requests': 2 - trunc, 'successes': 2 - trunc, 'errors': trunc, 'usage_missing_count': 0,
            'ttft_coverage': {'reasoning_truncated': trunc}}


def wrapper(mode):
    native = [{key: row[key] for key in ('seq', *w_checks.FIELDS)} for row in rows(mode)]
    if mode != 'adequate': native[1]['warning'] = 'Reasoning truncated; unanswered'
    s = summary(mode)
    return {'cli_argv': command(mode), 'cli_exit_code': int(mode != 'adequate'), 'requests': native,
            'summary': {**{k: s[k] for k in ('attempted', 'answered_requests', 'successes', 'errors', 'usage_missing_count')},
                        'reasoning_truncated': s['ttft_coverage']['reasoning_truncated']}}


class ContractTests(unittest.TestCase):
    def evaluate(self, mode, obj):
        with patch.object(w_checks.r_checks, 'load_cell', return_value=(rows(mode), summary(mode), [])), \
             patch.object(w_checks.r_checks, 'adequate_cell', return_value={'reasoning_streams': 2}), \
             patch.object(w_checks.r_checks, 'truncation_cell', return_value={'reasoning_streams': 2}):
            return w_checks.evaluate('', '', obj, 2, mode, int(mode != 'adequate'), '/candidate', 'http://localhost', '/prompt', '/cli', 'model')

    def test_fields_and_strict_modes(self):
        for mode in ('adequate', 'strict', 'safety'):
            self.assertEqual(self.evaluate(mode, wrapper(mode))['wrapper_fields_match'], 2)
        bad = wrapper('strict'); bad['requests'][1]['answer_completed'] = True
        with self.assertRaises(ValueError): self.evaluate('strict', bad)
        bad = wrapper('strict'); bad['requests'][1].pop('warning')
        with self.assertRaises(ValueError): self.evaluate('strict', bad)
        bad = wrapper('safety'); bad['cli_argv'].remove('--ci-safety')
        with self.assertRaises(ValueError): self.evaluate('safety', bad)
        bad = wrapper('adequate'); bad['summary']['answered_requests'] = 1
        with self.assertRaises(ValueError): self.evaluate('adequate', bad)

    def test_d_reference_is_not_fabricated_by_default_lane(self):
        lane = Path(__file__).with_name('lane-d.sh').read_text()
        self.assertIn('CERT_CASES=(BC-REASON-006)', lane)
        self.assertNotIn('failure_writer.py', lane)
        self.assertNotIn('exit 1', lane)

    def test_signoff_missing_or_mismatch_cannot_pass(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); (root / 'evidence').mkdir(); (root / 'results').mkdir()
            with self.assertRaises(OSError): w_finalize.finalize(root, root / 'missing')
            self.assertFalse((root / 'results/BC-INTEG-001.json').exists())

    def test_smoke_signoff_cannot_be_copied_into_certification(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); (root / 'evidence').mkdir(); (root / 'results').mkdir()
            smoke = root / 'smoke'; smoke.mkdir()
            (smoke / result_writer.SMOKE_MARKER).write_text(json.dumps(
                {'classification': result_writer.SMOKE_CLASS, 'expectations_sha256': 'f' * 64}))
            signoff = smoke / 'signoff.json'; signoff.write_text('{}')
            with self.assertRaises(result_writer.NonCertificationEvidence):
                w_finalize.finalize(root, signoff)
            self.assertFalse((root / 'evidence/w-platform-signoff.json').exists())
            self.assertFalse((root / 'results/BC-INTEG-001.json').exists())

    def test_signoff_binds_evidence_and_never_overwrites(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); ev = root / 'evidence'; ev.mkdir(); (root / 'results').mkdir()
            paths = {}
            for key in ('candidate', 'baseline', 'manifest', 'wrapper', 'prompt'):
                path = root / key; path.write_text(key); paths[key] = str(path)
            versions = {key + '_binary_sha256': manifest_tool.sha256_file(paths[key]) for key in ('candidate', 'baseline')}
            versions.update({'manifest_sha256': manifest_tool.sha256_file(paths['manifest']), 'wrapper_sha256': manifest_tool.sha256_file(paths['wrapper']), 'model': 'model'})
            inputs = {**paths, 'prompt_sha256': manifest_tool.sha256_file(paths['prompt']), 'url': 'http://localhost', 'count': 2}
            names = ['gpu.csv', 'binary-hashes.txt', 'models.json']
            for mode in ('adequate', 'strict', 'safety'):
                for suffix in ('cli.jsonl', 'wrapper.json', 'trace.jsonl', 'stdout.txt', 'stderr.txt', 'proxy-error.txt', 'command.json'):
                    names.append(f'w-{mode}-{suffix}')
            for name in names: (ev / name).write_text('{}')
            (ev / 'cleanup-verified.json').write_text('{"cleanup":"verified"}')
            (ev / 'server.log').write_text('server'); (ev / 'server-stop.txt').write_text('stopped')
            now = dt.datetime.now(dt.timezone.utc); before = (now - dt.timedelta(seconds=2)).isoformat().replace('+00:00', 'Z')
            observed = {mode: {'fixture': True, 'wrapper_exit_code': int(mode != 'adequate')} for mode in ('adequate', 'strict', 'safety')}
            collection = {'candidate_sha': 'a' * 40, 'versions': versions, 'input': inputs, 'owner': 'owner', 'evidence_link': 'private',
                          'gpu_class': 'NVIDIA H100', 'driver': 'test', 'start_utc': before, 'collected_utc': before, 'attempt': 1,
                          'observed': observed, 'files': {f'evidence/{name}': manifest_tool.sha256_file(ev / name) for name in names}}
            (ev / 'w-collection.json').write_text(json.dumps(collection))
            signoff = {'team': 'Metrum Bench platform', 'approved': True, 'candidate_sha': 'a' * 40, 'signer': 'fixture', 'evidence_link': 'private',
                       'signed_utc': (now - dt.timedelta(seconds=1)).isoformat().replace('+00:00', 'Z'),
                       'collection_sha256': manifest_tool.sha256_file(ev / 'w-collection.json'), 'cleanup_sha256': manifest_tool.sha256_file(ev / 'cleanup-verified.json'),
                       'server_log_sha256': manifest_tool.sha256_file(ev / 'server.log'), 'wrapper_sha256': versions['wrapper_sha256']}
            path = root / 'signoff'; path.write_text(json.dumps({**signoff, 'candidate_sha': 'b' * 40}))
            with self.assertRaises(ValueError): w_finalize.finalize(root, path)
            self.assertFalse((root / 'results/BC-INTEG-001.json').exists())
            path.write_text(json.dumps(signoff))
            with patch.object(w_finalize.w_checks, 'evaluate', return_value={'fixture': True}):
                w_finalize.finalize(root, path)
            self.assertEqual(json.loads((root / 'results/BC-INTEG-001.json').read_text())['result'], 'PASS')
            with patch.object(w_finalize.w_checks, 'evaluate', return_value={'fixture': True}), self.assertRaises(FileExistsError):
                w_finalize.finalize(root, path)


if __name__ == '__main__': unittest.main()
