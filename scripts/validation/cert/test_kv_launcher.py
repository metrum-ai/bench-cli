# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""KV delegation is isolated; the full r4 launcher source path is unchanged."""
import hashlib
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch
import case_launcher
import kv_dispatch


class KVLauncherTests(unittest.TestCase):
    def test_r4_source_bytes_unchanged_outside_two_dispatch_guards(self):
        source = Path(case_launcher.__file__).read_text()
        for guard in (
            "    import kv_dispatch\n    if manifest.get('lane') in kv_dispatch.LANE_MODULES:\n        return kv_dispatch.preflight(manifest)\n",
            "    import kv_dispatch\n    if manifest.get('lane') in kv_dispatch.LANE_MODULES:\n        return kv_dispatch.run_case(manifest_path, manifest, case)\n",
        ):
            self.assertEqual(1, source.count(guard))
            source = source.replace(guard, '')
        self.assertEqual('4d0ab23dc1e708ef3bfe941eb394fc9806e6bce95832da5e1f372f53427ae8c0', hashlib.sha256(source.encode()).hexdigest())

    def test_every_unimplemented_lane_rejects_before_r4_preflight(self):
        for case, lane in kv_dispatch.CASE_LANES.items():
            if kv_dispatch.adapter(case, lane).AVAILABLE:
                continue
            manifest = {'lane': lane, 'cases': {case: {'out': '/unused'}}}
            with self.subTest(case=case), patch.object(case_launcher, 'check_harness') as r4, \
                 patch('kv_envelope.validate'):
                with self.assertRaisesRegex(ValueError, 'runner not yet available'):
                    case_launcher.preflight(manifest)
                with self.assertRaisesRegex(ValueError, 'runner not yet available'):
                    case_launcher.run_case('/unused', manifest, case)
                r4.assert_not_called()

    def test_fake_case_runs_in_isolated_process_with_frozen_envelope(self):
        import sys
        from build_role_test_fixture import tool_fixture
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            tools, _ = tool_fixture(root)
            out = root / 'attempt'
            spec = root / 'spec.json'
            spec.write_text(json.dumps({'timeout_seconds': 10, 'cleanup_grace_seconds': 2,
                                       'cgroup_root': '/synthetic-delegated-root'}))
            child = root / 'fake.py'
            child.write_text("import os,json,sys\nfrom pathlib import Path\np=Path(sys.argv[1]);p.mkdir();(p/'env.json').write_text(json.dumps(dict(os.environ)))\n")
            manifest = {'candidate_sha': 'c' * 40, 'candidate': {'path': '/candidate'}, 'lane': 'KVT',
                'lane_manifest': {'path': str(spec), 'sha256': hashlib.sha256(spec.read_bytes()).hexdigest()},
                'tools': tools, 'cases': {'BC-KV-007': {'out': str(out),
                    'command': [tools['python3']['path'], str(child), str(out)]}}}
            path = root / 'manifest.json'; path.write_text(json.dumps(manifest))
            record = {'ID': 'BC-KV-007', 'result': 'PASS', 'candidate_sha': 'c' * 40,
                'input_hashes': {'execution_manifest': hashlib.sha256(path.read_bytes()).hexdigest()},
                'versions': {'execution_tools': tools}}
            # Synthetic cgroup descriptor: the real process join wrapper writes
            # to a temporary file; existing cgroup tests exercise real semantics.
            with (root / 'synthetic-procs').open('wb') as join:
                group = Mock(join_fd=join.fileno()); group.name = 'synthetic-group'
                group.populated.return_value = False
                with patch.object(kv_dispatch, 'preflight', return_value=['BC-KV-007']), \
                     patch('kv_envelope.validate'), \
                     patch('stage0_cgroup.create', return_value=group), \
                     patch('execution_evidence.report_rows', return_value=[record]), \
                     patch.dict(os.environ, {'KV_UNTRUSTED_ENV': 'sentinel', 'CERT_MANIFEST': '/r4', 'CERT_LANE': 'R4'}):
                    case_launcher.run_case(path, manifest, 'BC-KV-007')
                    env = json.loads((out / 'env.json').read_text())
                    self.assertEqual(str(path), env['CERT_INPUT_MANIFEST'])
                    self.assertEqual(str(root / 'attempt-lifecycle'), env['CERT_PHASE_ROOT'])
                    self.assertEqual(env['CERT_PHASE_ROOT'] + '/tool-bin', env['CERT_TOOL_BIN'])
                    self.assertEqual(env['CERT_TOOL_BIN'], env['PATH'])
                    for name in ('KV_UNTRUSTED_ENV', 'CERT_MANIFEST', 'CERT_LANE'):
                        self.assertNotIn(name, env)
                    self.assertEqual('sentinel', os.environ['KV_UNTRUSTED_ENV'])
                    group.close_if_empty.assert_called_once()
                    group.kill.assert_not_called()
                    self.assertTrue((root / 'synthetic-procs').read_text().isdecimal())

    def test_timeout_and_surviving_descendant_block_terminal_acceptance(self):
        from build_role_test_fixture import tool_fixture
        from stage0_process import ProcessOutcome
        for timed_out, leaked in ((True, False), (False, True)):
            with self.subTest(timed_out=timed_out, leaked=leaked), tempfile.TemporaryDirectory() as temp:
                root = Path(temp); tools, _ = tool_fixture(root)
                spec = root / 'spec.json'; spec.write_text(json.dumps({'timeout_seconds': 2,
                    'cleanup_grace_seconds': 1, 'cgroup_root': '/synthetic'}))
                out = root / 'attempt'
                manifest = {'lane': 'KVT', 'candidate': {'path': '/candidate'}, 'tools': tools,
                    'lane_manifest': {'path': str(spec), 'sha256': 'a' * 64},
                    'cases': {'BC-KV-007': {'out': str(out), 'command': ['/unused']}}}
                path = root / 'manifest.json'; path.write_text(json.dumps(manifest))
                group = Mock(join_fd=9); group.name = 'synthetic'
                group.populated.side_effect = [leaked, False]
                with patch.object(kv_dispatch, 'preflight', return_value=['BC-KV-007']), \
                     patch('stage0_cgroup.create', return_value=group), \
                     patch('stage0_process.run_bounded', return_value=ProcessOutcome(0, timed_out, False)):
                    with self.assertRaises(ValueError):
                        case_launcher.run_case(path, manifest, 'BC-KV-007')
                    self.assertTrue((out / 'results/lane-error.json').is_file())
                    if leaked:
                        group.kill.assert_called_once()


if __name__ == '__main__':
    unittest.main()
