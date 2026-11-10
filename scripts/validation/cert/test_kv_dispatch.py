# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""All KV codes are registered; absent or mismatched launchers never execute."""
import subprocess
import unittest
from pathlib import Path
import kv_dispatch
import validate_results


class KVDispatchTests(unittest.TestCase):
    def test_catalog_mapping_and_unavailable_stubs(self):
        rows = validate_results.load_catalog(Path(__file__).resolve().parents[3] /
                                             'docs/RELEASE_CERTIFICATION_CATALOG.csv')
        self.assertEqual({'KVV', 'KVS', 'KVL', 'KVD', 'KVT', 'KVC'}, set(kv_dispatch.LANE_MODULES))
        entry = Path(__file__).with_name('case_entry.sh')
        for case, lane in kv_dispatch.CASE_LANES.items():
            with self.subTest(case=case):
                self.assertEqual(lane, rows[case]['Parallel group'])
                module = kv_dispatch.adapter(case, lane)
                self.assertIsInstance(module.AVAILABLE, bool)
                if module.AVAILABLE:
                    continue  # Lane-owned tests exercise implemented runners offline.
                with self.assertRaisesRegex(ValueError, 'runner not yet available'):
                    kv_dispatch.adapter(case, lane).main(['--case', case])
                with self.assertRaisesRegex(ValueError, 'runner not yet available'):
                    kv_dispatch.adapter(case, lane).freeze(None)
                result = subprocess.run(['bash', str(entry), '--case', case, '--lane', lane.lower()],
                                        capture_output=True, text=True, check=False)
                self.assertNotEqual(0, result.returncode)
                self.assertIn('runner not yet available', result.stderr)

    def test_wrong_lane_and_unknown_case_rejected(self):
        for case, lane in [('BC-KV-003', 'KVV'), ('BC-KV-999', 'KVC'), ('BC-GPU-001', 'KVV')]:
            with self.assertRaisesRegex(ValueError, 'mapping'):
                kv_dispatch.adapter(case, lane)


if __name__ == '__main__':
    unittest.main()
