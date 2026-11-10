# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Fake frozen-envelope and immutable-input controls; no live server."""
import json
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
            with patch.object(adapter, 'validate_spec'), patch.object(adapter.build_roles, 'load_receipt', return_value=([measured], {})), \
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
