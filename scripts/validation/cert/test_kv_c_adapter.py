# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Fake frozen-envelope and immutable-input controls; no live server."""
import json
import subprocess
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import kv_c_adapter as adapter


class AdapterTests(unittest.TestCase):
    def test_bad_envelope_and_execution_spec_fail_before_resources(self):
        family = SimpleNamespace(CASES={'BC-KV-003'})
        with self.assertRaises(ValueError):
            adapter.preflight({'schema_version': 'wrong'}, family, 'KVS')
        with self.assertRaises(ValueError):
            adapter.validate_spec({'schema_version': 'kv-r5-c-execution.v1', 'candidate_sha': 'a' * 40,
                                   'case': 'BC-KV-005'}, family, 'KVS')
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'spec.json'
            source.write_text('{}')
            with patch.object(adapter.os, 'execve') as execute, self.assertRaises(ValueError):
                adapter.main(['--case', 'BC-KV-003', '--manifest', str(source), '--manifest-sha256', '0' * 64,
                              '--out', str(root / 'out')], family, 'KVS')
            execute.assert_not_called()

    def test_guarded_launch_reserves_cleanup_and_timeout_cannot_pass(self):
        from unittest.mock import MagicMock
        for timeout in (False, True):
            with self.subTest(timeout=timeout), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                out = root / 'run'
                (out / 'evidence').mkdir(parents=True)
                (out / 'evidence/server-container-owner.txt').write_text('a' * 32)
                candidate = root / 'candidate'
                candidate.write_text('fixture')
                spec = {'candidate': adapter.file_pin(candidate), 'baseline': adapter.file_pin(candidate),
                    'tools': {'bash': {'path': '/bin/bash'}, 'docker': {'path': '/usr/bin/docker'}},
                    'hub_cache': str(root), 'timeout_seconds': 600, 'cleanup_grace_seconds': 120,
                    'lanes': {'KVS': {'port': 8000, 'proxy_port': 8001}}}
                source = root / 'spec.json'
                source.write_text(json.dumps(spec))
                envelope = root / 'envelope.json'
                envelope.write_text(json.dumps({'lane_manifest': adapter.file_pin(source),
                    'cases': {'BC-KV-003': {'out': str(out)}}}))
                process = MagicMock()
                process.pid = 12345
                process.poll.return_value = 0
                process.wait.side_effect = [subprocess.TimeoutExpired('fixture', 360), 0] if timeout else [0]
                family = SimpleNamespace(CASES={'BC-KV-003'})
                with patch.dict('os.environ', {'CERT_INPUT_MANIFEST': str(envelope)}), \
                        patch.object(adapter, 'validate_spec'), patch.object(adapter, 'preflight'), \
                        patch.object(adapter.subprocess, 'Popen', return_value=process) as launch, \
                        patch.object(adapter.subprocess, 'run', return_value=SimpleNamespace(stdout='')), \
                        patch.object(adapter.socket, 'socket'), patch.object(adapter.os, 'killpg') as kill:
                    result = adapter.main(['--case', 'BC-KV-003', '--manifest', str(source),
                        '--manifest-sha256', adapter.file_pin(source)['sha256'], '--out', str(out)], family, 'KVS')
                    self.assertEqual(124 if timeout else 0, result)
                    self.assertEqual(360, process.wait.call_args_list[0].kwargs['timeout'])
                    self.assertEqual('BC-KV-003', launch.call_args.kwargs['env']['CERT_CASE_ID'])
                    self.assertEqual(str(root), launch.call_args.kwargs['env']['CERT_HF_HUB_CACHE'])
                    self.assertTrue(launch.call_args.kwargs['start_new_session'])
                    self.assertEqual(1 if timeout else 0, kill.call_count)
                self.assertFalse((out / 'results').exists())

    def test_complete_model_inventory_and_tokenizer_bound_cohorts_are_required(self):
        import hashlib
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            model = root / 'hub' / ('b' * 40)
            model.mkdir(parents=True)
            (model / 'weights.safetensors').write_bytes(b'synthetic weights')
            (model / 'tokenizer.json').write_text('{}')
            candidate = root / 'candidate'
            candidate.write_bytes(b'fixture')
            candidate.chmod(0o755)
            prompts = root / 'prompts.jsonl'
            prompts.write_text('{"prompt":"hello"}\n')
            lengths = root / 'lengths.json'
            lengths.write_text(json.dumps({'tokenizer_sha256': adapter.file_pin(model / 'tokenizer.json')['sha256'],
                'counts': {hashlib.sha256(b'hello').hexdigest(): 8}}))
            tools = root / 'tools.json'
            tools.write_text('{}')
            binary = adapter.file_pin(candidate)
            spec = {'schema_version': 'kv-r5-c-execution.v1', 'candidate_sha': 'a' * 40, 'case': 'BC-KV-003',
                'candidate': binary, 'baseline': binary, 'candidate_binary_sha256': binary['sha256'],
                'baseline_binary_sha256': binary['sha256'], 'tools': {}, 'tool_manifest': adapter.file_pin(tools),
                'lanes': {'KVS': {'kv_case': 'BC-KV-003', 'port': 8000, 'server_flags': [], 'revision': 'b' * 40,
                                 'prompt_sets': {'seed': adapter.file_pin(prompts)}}},
                'model_snapshot': str(model), 'hub_cache': str(root / 'hub'), 'timeout_seconds': 2100,
                'cleanup_grace_seconds': 120, 'cgroup_root': '/sys/fs/cgroup/test',
                'model_files': [adapter.file_pin(p) for p in model.iterdir()], 'prompt_token_counts': adapter.file_pin(lengths)}
            family = SimpleNamespace(CASES={'BC-KV-003'}, validate_config=lambda config: {'max_prompt_tokens': 16})
            with patch.object(adapter.execution_policy, 'validate_case_tools', return_value={}), \
                    patch.object(adapter.execution_policy, 'load_manifest', return_value={}):
                adapter.validate_spec(spec, family, 'KVS')
                for change in ('model_files', 'prompt_token_counts', 'timeout_seconds'):
                    changed = json.loads(json.dumps(spec))
                    if change == 'model_files':
                        changed[change].pop()
                    elif change == 'prompt_token_counts':
                        changed[change] = adapter.file_pin(tools)
                    else:
                        changed[change] = 0
                    with self.subTest(change=change), self.assertRaises(ValueError):
                        adapter.validate_spec(changed, family, 'KVS')

    def test_freezer_binds_exact_command_transitive_inputs_and_workflow_candidate(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / 'candidate'
            binary.write_bytes(b'synthetic binary')
            receipt = root / 'receipt.json'
            receipt.write_text('{}')
            (root / 'validation-only.log').write_text('synthetic log')
            fit = root / 'fit.json'
            fit.write_text('{}')
            catalog = root / 'catalog.csv'
            catalog.write_text('fixture')
            measured = adapter.file_pin(binary)
            spec = {'schema_version': 'kv-r5-c-execution.v1', 'candidate_sha': 'a' * 40, 'case': 'BC-KV-003',
                    'candidate': measured, 'baseline': measured, 'tools': {}, 'tool_manifest': adapter.file_pin(receipt),
                    'lanes': {'KVS': {'fit': adapter.file_pin(fit)}}}
            source = root / 'spec.json'
            source.write_text(json.dumps(spec))
            selection = root / 'selection.json'
            selection.write_text(json.dumps({'candidate_sha': 'a' * 40, 'selected': ['BC-KV-003'],
                                            'catalog_sha256': adapter.file_pin(catalog)['sha256']}))
            args = SimpleNamespace(selection=selection, catalog=catalog, lane_manifest=source, case='BC-KV-003',
                                   candidate=binary, baseline=binary, build_receipt=receipt, input=[], command_json=None,
                                   tool_manifest=receipt, out=root / 'frozen.json', run_out=root / 'run')
            family = SimpleNamespace(CASES={'BC-KV-003'})
            with patch.dict('sys.modules', {'kv_envelope': SimpleNamespace(validate=lambda *args: None)}), \
                    patch.object(adapter, 'SCHEMA', receipt), patch.object(adapter, 'validate_spec'), patch.object(adapter.build_roles, 'load_receipt', return_value=([measured], {})), \
                    patch.object(adapter.harness_integrity, 'verify', return_value=[]):
                frozen = adapter.freeze(args, family, 'KVS')
                self.assertEqual('kv-r5-frozen.v1', frozen['schema_version'])
                self.assertEqual(['BC-KV-003'], adapter.preflight(frozen, family, 'KVS'))
                changed = json.loads(json.dumps(frozen))
                changed['cases']['BC-KV-003']['command'].append('--extra')
                with self.assertRaises(ValueError):
                    adapter.preflight(changed, family, 'KVS')
                changed = json.loads(json.dumps(frozen))
                changed['inputs'] = []
                with self.assertRaises(ValueError):
                    adapter.preflight(changed, family, 'KVS')
                binary.write_bytes(b'mutated')
                with self.assertRaises(ValueError):
                    adapter.preflight(frozen, family, 'KVS')


if __name__ == '__main__':
    unittest.main()
