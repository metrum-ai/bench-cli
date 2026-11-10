# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""No-network CPU oracles plus product checks invoked by the Cargo integration test."""
import os
import sys
import copy
import shutil
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import execution_policy
import kv_cpu_checks as checks
import kv_cpu_runner as runner
import result_writer
import validate_results


class CPUContractTests(unittest.TestCase):
    def test_kvc_checks_exact_spec_inputs_and_workflow_binary_membership(self):
        from kv_lanes import kvc
        from build_role_test_fixture import tool_fixture
        from execution_evidence import digest
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp); tools, _ = tool_fixture(root)
            def pin(path):
                return {'path': str(Path(path).resolve()), 'sha256': digest(path)}
            origin = root / 'synthetic-origin.json'; runner.dump(origin, {})
            lock = checks.fixture_lock()
            paths = [checks.FIXTURES / name for name in lock['files']]
            paths += [checks.FIXTURES / 'lock.json', *[runner.REPO / name for name in lock['schemas']]]
            binary = root / 'metrum-ai-bench-cli-prompts'; shutil.copyfile(sys.executable, binary); binary.chmod(0o755)
            probe = root / 'metrum-ai-bench-cli-cert-kv-cpu'; shutil.copyfile(sys.executable, probe); probe.chmod(0o755)
            spec = {'schema_version': 'kv-cpu-execution.v1', 'case': 'BC-KV-012', 'cases': {'BC-KV-012': {}},
                'candidate_sha': 'a' * 40, 'tools': tools, 'public_revision': lock['public_inputs']['revision'],
                'public_inputs': str(root), 'inputs': [pin(path) for path in paths],
                'workflow_origin': pin(origin), 'binary': pin(binary), 'probe': pin(probe),
                'timeout_seconds': 120, 'cleanup_grace_seconds': 15, 'cgroup_root': '/synthetic',
                'owner': 'fixture', 'evidence_link': 'fixture'}
            with patch('pretag_artifact.verify', return_value={binary.name: digest(binary), probe.name: digest(probe)}):
                self.assertEqual(spec, kvc.validate_spec(spec))
                for mutate in (lambda b: b.update(public_revision='b' * 40),
                               lambda b: b['inputs'].pop(),
                               lambda b: b['tools'].pop('python3'),
                               lambda b: b.update(timeout_seconds=0),
                               lambda b: b.update(trust_remote_code=True),
                               lambda b: b['probe'].update(sha256='0' * 64)):
                    bad = copy.deepcopy(spec); mutate(bad)
                    with self.assertRaises((ValueError, KeyError)):
                        kvc.validate_spec(bad)
            with patch('pretag_artifact.verify', return_value={}):
                with self.assertRaisesRegex(ValueError, 'workflow archive'):
                    kvc.validate_spec(spec)

    def test_fixture_bytes_and_public_pins(self):
        lock = checks.fixture_lock()
        self.assertEqual('0666f62e581b482838ae2e17b333ee36ff3d01b0', lock['public_inputs']['revision'])
        self.assertEqual('e5480af17428c48e614a0840f30eda53acbea7f9efe869c5173cde5d5818edfd',
                         lock['public_inputs']['files']['W2.jsonl'])
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'input'
            path.write_bytes(b'changed')
            with self.assertRaisesRegex(ValueError, 'checksum'):
                checks.verify_files(Path(temp), {'input': '0' * 64})

    def test_dry_run_record_remains_ineligible_when_copied(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            runner.dump(root / result_writer.SMOKE_MARKER, {'classification': result_writer.SMOKE_CLASS,
                'expectations_sha256': 'a' * 64})
            record = result_writer.classify_smoke(root, {'ID': 'BC-KV-011'})
            with self.assertRaises(result_writer.NonCertificationEvidence):
                validate_results.check_evidence_class(record)
            with self.assertRaises(result_writer.NonCertificationEvidence):
                validate_results.reject_smoke_root(root)

    def test_renderer_and_interpreter_pins_are_not_implicit(self):
        job = runner.document_job(checks.FIXTURES / 'documents.jsonl', rendering=True)
        self.assertEqual('minijinja-chat.v2', job['rendering']['identity']['renderer_version'])
        self.assertEqual(checks.fixture_lock()['native_vocabulary_sha256'],
                         job['rendering']['identity']['tokenizer_sha256'])
        with self.assertRaises(ValueError):
            execution_policy.validate_case_tools({}, 'KVC')


@unittest.skipUnless(os.environ.get('KV_CPU_PROMPTS') and os.environ.get('KV_CPU_PROBE'),
                     'Cargo integration test supplies the two product executables')
class CPUProductTests(unittest.TestCase):
    def test_all_six_product_paths_with_independent_oracles_and_controls(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            public = root / 'public'; public.mkdir()
            # This synthetic transport fixture includes duplicate prompt text;
            # source ordinals must survive without silently dropping a row.
            (public / 'sample.jsonl').write_bytes(b'{"prompt":"same"}\n{"prompt":"same"}\n{"prompt":"different"}\n')
            (public / 'LICENSE').write_bytes((checks.FIXTURES / 'LICENSE').read_bytes())
            for case in runner.CASES:
                with self.subTest(case=case):
                    out = root / case; out.mkdir()
                    env = {'PATH': '/usr/bin:/bin', 'HOME': str(out), 'NO_BANNER': '1',
                           'HF_HUB_OFFLINE': '1', 'HF_DATASETS_OFFLINE': '1'}
                    subject = runner.Case(out, Path(os.environ['KV_CPU_PROMPTS']),
                                          Path(os.environ['KV_CPU_PROBE']), env)
                    observed = subject.execute(case, public)
                    self.assertTrue(observed)
                    self.assertGreaterEqual(len(subject.controls), 4)
                    if case == 'BC-KV-010':
                        self.assertEqual(3, observed['rows'])
                        self.assertEqual(3, observed['ids'])


if __name__ == '__main__':
    unittest.main()
