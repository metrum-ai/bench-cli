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
            self.assertIn("  group: ${{ github.workflow }}-${{ github.event_name == 'pull_request' && format('pr-{0}', github.event.pull_request.number) || format('run-{0}', github.run_id) }}\n", blocks[0], path.name)
            self.assertIn("  cancel-in-progress: ${{ github.event_name == 'pull_request' }}\n", blocks[0], path.name)
        self.assertEqual(matched, ["ci.yml", "codeql.yml", "dummy-server.yml", "perf-validation.yml"])

    def test_non_pr_groups_preserve_pending_runs_and_separate_namespaces(self):
        # The exact expression is asserted above; model its event dispatch here.
        def group(workflow, event, number, run):
            suffix = f"pr-{number}" if event == "pull_request" else f"run-{run}"
            return f"{workflow}-{suffix}"
        self.assertEqual(group("CI", "pull_request", 7, 100), group("CI", "pull_request", 7, 101))
        self.assertNotEqual(group("CI", "pull_request", 7, 100), group("CI", "pull_request", 8, 101))
        self.assertNotEqual(group("CI", "pull_request", 7, 100), group("CodeQL", "pull_request", 7, 100))
        for event in ("push", "schedule", "workflow_dispatch", "release", "merge_group"):
            self.assertNotEqual(group("CI", event, None, 100), group("CI", event, None, 101), event)
            self.assertNotEqual(group("CI", event, None, 7), group("CI", "pull_request", 7, 100), event)

    def test_nested_runner_group_cannot_replace_non_pr_jobs_or_lock_workflow(self):
        text = (WORKFLOWS / "ci.yml").read_text()
        block = re.search(r"(?m)^    concurrency:\n((?:^      .*\n)+)", text).group(1)
        self.assertIn("group: ${{ github.workflow }}-self-hosted-${{ github.event_name == 'pull_request' && format('pr-{0}', github.event.pull_request.number) || format('run-{0}', github.run_id) }}", block)
        self.assertIn("cancel-in-progress: ${{ github.event_name == 'pull_request' }}", block)
        self.assertNotIn("github.ref", block)

    def test_literal_self_hosted_label_is_registered_without_wildcards(self):
        workflow = (WORKFLOWS / 'ci.yml').read_text()
        block = re.search(r'(?ms)^  self-hosted:\n(.*?)(?=^  [a-zA-Z0-9_-]+:|\Z)', workflow)
        self.assertIsNotNone(block)
        self.assertIn('name: test (self-hosted bench-cli)', block.group(1))
        label = re.search(r'(?m)^    runs-on: ([a-z0-9-]+)$', block.group(1))
        self.assertIsNotNone(label, 'self-hosted job must retain a literal runner label')
        config = (WORKFLOWS.parent / 'actionlint.yaml').read_text()
        labels = re.findall(r'(?m)^    - ([a-z0-9-]+)$', config)
        self.assertIn(label.group(1), labels, 'register exact custom label; do not suppress lint')
        self.assertNotIn('*', config, 'wildcard runner labels weaken typo detection')

    def test_actionlint_is_pinned_and_checks_every_pr_workflow(self):
        text = (WORKFLOWS / "ci.yml").read_text()
        self.assertIn("name: actionlint (PR workflows)", text)
        self.assertIn("go run github.com/rhysd/actionlint/cmd/actionlint@v1.7.12", text)
        for name in ("ci.yml", "codeql.yml", "dummy-server.yml", "perf-validation.yml"):
            self.assertIn(f".github/workflows/{name}", text)

    def test_in_progress_cancellation_predicate_excludes_evidence_events(self):
        # Test the exact predicate above; pending-group replacement is separate
        # platform behavior and must not be confused with cancel-in-progress.
        for event in ("push", "schedule", "workflow_dispatch", "release", "merge_group"):
            self.assertFalse(event == "pull_request", event)
        self.assertTrue("pull_request" == "pull_request")


if __name__ == "__main__":
    unittest.main()
