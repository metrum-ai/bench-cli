# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Functional tokenizer capability is separate from version/pin validation."""
import json
import tempfile
import unittest
from pathlib import Path
from subprocess import CompletedProcess
from unittest.mock import patch
import freeze_case
import tokenizer_probe
from execution_evidence import digest


def row(**changes):
    return {"schema_version": "bench.request.v1", "error": None, "answer_completed": True,
            "tokenized_completion_tokens": 3, "usage_completion_tokens_present": False, **changes}


class TokenizerProbeTests(unittest.TestCase):
    def test_requires_positive_actual_fallback_counts(self):
        tokenizer_probe.check_rows(json.dumps(row()))
        for count in (None, 0, -1, True):
            with self.subTest(count=count), self.assertRaises(ValueError):
                tokenizer_probe.check_rows(json.dumps(row(tokenized_completion_tokens=count)))

    def test_rejects_server_counts_incomplete_answers_or_multiple_records(self):
        for value in (row(usage_completion_tokens_present=True), row(answer_completed=False), row(error={"kind": "timeout"})):
            with self.subTest(value=value), self.assertRaises(ValueError):
                tokenizer_probe.check_rows(json.dumps(value))
        with self.assertRaises(ValueError):
            tokenizer_probe.check_rows(json.dumps(row()) + '\n' + json.dumps(row()))

    def test_probe_has_no_ambient_credentials_exporters_or_proxy(self):
        def fake(command, **kwargs):
            self.assertEqual(kwargs['env']['NO_PROXY'], '*')
            self.assertNotIn('OPENAI_API_KEY', kwargs['env'])
            self.assertNotIn('OTEL_EXPORTER_OTLP_ENDPOINT', kwargs['env'])
            output = Path(command[command.index('--data-log') + 1])
            output.write_text(json.dumps(row()) + '\n')
            return CompletedProcess(command, 0, '', '')
        with patch.dict('os.environ', {'OPENAI_API_KEY': 'test-placeholder', 'OTEL_EXPORTER_OTLP_ENDPOINT': 'test-placeholder'}):
            with patch('tokenizer_probe.subprocess.run', side_effect=fake):
                tokenizer_probe.probe('/unused/frozen-binary', '/unused/tokenizer.json')

    def test_failed_cli_cannot_pass_probe(self):
        with patch('tokenizer_probe.subprocess.run', return_value=CompletedProcess([], 2, '', 'unsupported')):
            with self.assertRaisesRegex(ValueError, 'exit 2'):
                tokenizer_probe.probe('/unused/frozen-binary', '/unused/tokenizer.json')

    def test_frozen_top_tokenizer_must_match_actual_g4_runner_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'tokenizer.json'
            path.write_text('{}')
            freeze_case.check_case_tokenizer('BC-GPU-010', {'tokenizer': {'sha256': digest(path)}}, path)
            for config in ({}, {'tokenizer': None}, {'tokenizer': {'sha256': '0' * 64}}):
                with self.subTest(config=config), self.assertRaises(ValueError):
                    freeze_case.check_case_tokenizer('BC-GPU-010', config, path)
            freeze_case.check_case_tokenizer('BC-GPU-001', {}, path)


if __name__ == '__main__':
    unittest.main()
