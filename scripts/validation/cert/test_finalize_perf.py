# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""No performance PASS is published before owned resource cleanup."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import finalize_perf


class FinalizePerfTests(unittest.TestCase):
    def test_passing_case_requires_cleanup_and_failed_lane_downgrades(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); evidence = root / 'evidence'; evidence.mkdir()
            case = 'BC-PERF-002'
            record = {'ID': case, 'result': 'PASS', 'observed_values': {}, 'evidence_paths': []}
            (evidence / f'provisional-{case}.json').write_text(json.dumps(record))
            argv = ['finalize', '--out', str(root), case]
            with patch('sys.argv', argv), patch.object(finalize_perf.result_writer, 'write_record') as write:
                with self.assertRaises(ValueError): finalize_perf.main()
                write.assert_not_called()
                (evidence / 'cleanup-verified.json').write_text('{"cleanup":"verified"}')
                self.assertEqual(finalize_perf.main(), 0)
                self.assertEqual(write.call_args.args[1]['result'], 'PASS')
                self.assertIn('evidence/cleanup-verified.json', write.call_args.args[1]['evidence_paths'])
            with patch('sys.argv', ['finalize', '--out', str(root), '--fail', case]), \
                 patch.object(finalize_perf.result_writer, 'write_record') as write:
                self.assertEqual(finalize_perf.main(), 0)
                self.assertEqual(write.call_args.args[1]['result'], 'FAIL')


if __name__ == '__main__': unittest.main()
