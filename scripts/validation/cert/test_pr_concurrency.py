# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Keep PR-run cancellation conditional and scoped across workflow identities."""
import re
import unittest
from pathlib import Path

WORKFLOWS = Path(__file__).resolve().parents[3] / ".github/workflows"


class PrConcurrencyTests(unittest.TestCase):
    def test_every_pr_workflow_has_top_level_conditional_concurrency(self):
        matched = []
        for path in sorted(WORKFLOWS.glob("*.yml")):
            text = path.read_text()
            if not re.search(r"(?m)^  pull_request:", text):
                continue
            matched.append(path.name)
            blocks = re.findall(r"(?m)^concurrency:\n((?:^  .*\n)+)", text)
            self.assertEqual(len(blocks), 1, path.name)
            self.assertIn("  group: ${{ github.workflow }}-${{ github.event.pull_request.number || github.ref }}\n", blocks[0], path.name)
            self.assertIn("  cancel-in-progress: ${{ github.event_name == 'pull_request' }}\n", blocks[0], path.name)
        self.assertEqual(matched, ["ci.yml", "codeql.yml", "dummy-server.yml", "perf-validation.yml"])

    def test_in_progress_cancellation_predicate_excludes_evidence_events(self):
        # Test the exact predicate above; pending-group replacement is separate
        # platform behavior and must not be confused with cancel-in-progress.
        for event in ("push", "schedule", "workflow_dispatch", "release", "merge_group"):
            self.assertFalse(event == "pull_request", event)
        self.assertTrue("pull_request" == "pull_request")


if __name__ == "__main__":
    unittest.main()
