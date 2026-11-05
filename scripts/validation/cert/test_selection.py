# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Selection and qualification invariants without network or GPU."""
import datetime as dt
import json
import subprocess
import tempfile
import unittest
from pathlib import Path

import nightly_decision
import select_cases as select
import validate_results as validator


class SelectionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.catalog = self.root / 'catalog.csv'
        lines = ['ID,Tier,Test case,Area,Parallel group,Selection,Cadence',
                 'BC-CORE-001,RC,Core,PACK,PACK,core,per-release',
                 'BC-POST-001,POST,Published,POST,POST,core,per-release']
        for area in ('SSE', 'GPU'):
            for n in range(1, 5):
                lines.append(f'BC-{area}-{n:03},RC-GPU,Case {n},{area},G1,extended,per-release')
        lines.append('BC-HW-002,PERIODIC,Retired,HW,RETIRED,extended,')
        self.catalog.write_text('\n'.join(lines) + '\n', encoding='utf-8')
        self.rows = validator.load_catalog(self.catalog)
        self.sha = 'a' * 40
        self.past = [{'release': 'v1', 'ran': sorted(self.rows)}]
        self.results = self.root / 'results'
        self.results.mkdir()
        (self.root / 'proof.txt').write_text('proof')

    def build(self, changes=None, history=None):
        return select.make_selection(self.rows, select.catalog_hash(self.catalog), self.sha, .3, 3,
                                     changes or [], self.past if history is None else history)

    def record(self, case):
        return {'ID': case, 'candidate_sha': self.sha, 'result': 'PASS', 'observed_values': {'x': 1},
                'thresholds': {'x': 1}, 'evidence_paths': ['proof.txt'], 'evidence_link': 'proof',
                'owner': 'owner', 'attempt': 1, 'start_utc': '2026-01-01T00:00:00Z',
                'end_utc': '2026-01-01T00:01:00Z'}

    def test_sample_stratified_stable_and_lane_plan_selected_only(self):
        first = self.build()
        self.assertEqual(first, self.build())
        selected = first['selected']
        self.assertIn('BC-CORE-001', selected)
        self.assertIn('BC-POST-001', selected)
        self.assertNotIn('BC-HW-002', selected)
        for area in ('SSE', 'GPU'):
            self.assertEqual(2, sum(k.startswith('BC-' + area) for k in selected))
        self.assertEqual(len(selected), first['selected_count'])
        self.assertEqual(10, first['eligible_count'])
        self.assertEqual(set(selected), {c for lane in first['lane_plan'] for c in lane['cases']})
        self.assertTrue(any(x['phase'] == 'post' for x in first['lane_plan']))

    def test_change_forces_all_related_and_unknown_fails_safe(self):
        sel = self.build(['src/sse.rs'])
        self.assertTrue(all('changed-area' in sel['selected'][f'BC-SSE-{n:03}'] for n in range(1, 5)))
        sel = self.build(['unmapped-file'])
        self.assertEqual(['unmapped-file'], sel['unmapped_files'])
        self.assertEqual(10, len(sel['selected']))

    def test_rotation_includes_unrun_and_empty_history_is_conservative(self):
        past = [{'release': 'v1', 'ran': ['BC-SSE-001']}]
        sel = self.build(history=past)
        self.assertIn('rotation', sel['selected']['BC-SSE-002'])
        self.assertEqual(10, len(self.build(history=[])['selected']))

    def test_selection_cannot_omit_or_modify_frozen_policy(self):
        selection = self.build()
        path = self.root / 'selection.json'
        path.write_text(json.dumps(selection))
        self.assertEqual(selection, validator.load_selection(path, self.catalog, self.rows, self.sha))
        del selection['selected']['BC-CORE-001']
        path.write_text(json.dumps(selection))
        with self.assertRaises(ValueError):
            validator.load_selection(path, self.catalog, self.rows, self.sha)
        with self.assertRaises(ValueError):
            validator.load_selection(path, self.catalog, self.rows, 'b' * 40)

    def test_qualification_and_post_stages(self):
        selection = self.build()
        now = dt.datetime(2026, 2, 1, tzinfo=dt.timezone.utc)
        table, errors = validator.validate(self.catalog, self.results, self.sha, now, selection=selection)
        self.assertTrue(errors)
        for case in selection['selected']:
            if not case.startswith('BC-POST'):
                (self.results / f'{case}.json').write_text(json.dumps(self.record(case)))
        table, errors = validator.validate(self.catalog, self.results, self.sha, now, selection=selection)
        self.assertFalse(errors)
        self.assertEqual(4, sum(row['Result'] == 'NOT-SELECTED' for row in table))
        self.assertTrue(validator.validate(self.catalog, self.results, self.sha, now, selection=selection, include_post=True)[1])
        (self.results / 'BC-POST-001.json').write_text(json.dumps(self.record('BC-POST-001')))
        self.assertFalse(validator.validate(self.catalog, self.results, self.sha, now, selection=selection, include_post=True)[1])

    def test_history_csv_is_newest_first_and_requires_unique_release(self):
        p = self.root / 'history.csv'
        p.write_text('Release,ID,Result\nv1,BC-SSE-001,PASS\nv1,BC-SSE-002,NOT-SELECTED\n')
        history = select.load_history([p], self.rows)
        self.assertEqual([{'release': 'v1', 'ran': ['BC-SSE-001']}], history)
        with self.assertRaises(ValueError):
            select.load_history([p, p], self.rows)


class PublicCatalogTests(unittest.TestCase):
    def test_required_core_and_change_trigger_cadence(self):
        path = Path(__file__).resolve().parents[3] / 'docs' / 'RELEASE_CERTIFICATION_CATALOG.csv'
        rows = validator.load_catalog(path)
        expected_core = {'BC-LIVE-001', 'BC-LIVE-002', 'BC-LIVE-003', 'BC-LIVE-004',
                         'BC-REASON-001', 'BC-REASON-002', 'BC-REASON-003', 'BC-REASON-004',
                         'BC-REASON-006', 'BC-PERF-001', 'BC-PERF-002', 'BC-PERF-003',
                         'BC-COMP-003', 'BC-OPS-001', 'BC-INTEG-001'}
        self.assertTrue(all(rows[case]['Selection'] == 'core' for case in expected_core))
        self.assertTrue(all(r['Selection'] == 'core' for r in rows.values() if r['Area'] in {'PACK', 'REL', 'POST'}))
        self.assertTrue(all('monthly' not in r['Cadence'].split(';') for r in rows.values()))
        for case in ('BC-LIVE-001', 'BC-REASON-001', 'BC-COMP-003', 'BC-GPU-016'):
            self.assertTrue({'on-main-change', 'on-upstream-change'} <= set(rows[case]['Cadence'].split(';')))


    def test_tracked_source_files_have_explicit_mapping(self):
        root = Path(__file__).resolve().parents[3]
        output = subprocess.run(['git', 'ls-files', 'src/**'], cwd=root, check=True, capture_output=True, text=True).stdout
        paths = output.splitlines()
        self.assertTrue(paths)
        unmapped = [path for path in paths if path.endswith('.rs') and path not in select.PATH_AREAS]
        self.assertEqual([], unmapped, f'map newly tracked source paths explicitly: {unmapped}')

    def test_docs_plus_changelog_is_sampled_not_full(self):
        path = Path(__file__).resolve().parents[3] / 'docs' / 'RELEASE_CERTIFICATION_CATALOG.csv'
        catalog = validator.load_catalog(path)
        sha = 'a' * 40
        history = [{'release': 'prior', 'ran': sorted(catalog)}]
        base = select.make_selection(catalog, select.catalog_hash(path), sha, .3, 3, [], history)
        selected = select.make_selection(catalog, select.catalog_hash(path), sha, .3, 3,
                                         ['CHANGELOG.md', 'docs/CLI.md'], history)
        forced = {case for case, row in catalog.items() if row['Tier'] != 'PERIODIC' and row['Selection'] == 'extended' and row['Area'] in {'REL', 'PACK', 'POST', 'CI'}}
        self.assertEqual(set(base['selected']) | forced, set(selected['selected']))
        self.assertEqual(['CI', 'PACK', 'POST', 'REL'], selected['changed_areas'])
        self.assertLess(selected['selected_count'], selected['eligible_count'])
        self.assertFalse(selected['unmapped_files'])
        test_only = select.make_selection(catalog, select.catalog_hash(path), sha, .3, 3, ['tests/example.rs'], history)
        self.assertEqual(set(base['selected']), set(test_only['selected']))


class NightlyDecisionTests(unittest.TestCase):
    def test_live_docs_step_is_not_gated_by_skip(self):
        root = Path(__file__).resolve().parents[3]
        workflow = (root / '.github/workflows/regular-tests.yml').read_text(encoding='utf-8')
        step = workflow.split('      - name: Published documentation links\n', 1)[1].split('\n  nightly-failure:', 1)[0]
        self.assertNotIn('if:', step)
        self.assertIn('check_live_docs.py', step)

    def test_only_a_successful_full_nightly_at_same_sha_skips(self):
        def responses(step, conclusion='success', head='a' * 40):
            def get(path):
                if '/workflows/' in path:
                    return {'workflow_runs': [{'id': 1, 'head_sha': head}]}
                return {'jobs': [{'name': 'nightly', 'conclusion': conclusion, 'steps': [
                    {'name': 'Full suite and ignored tests', 'conclusion': step}]}]}
            return get
        sha = 'a' * 40
        self.assertFalse(nightly_decision.should_run('org/repo', 'refs/heads/main', sha, responses('success')))
        self.assertTrue(nightly_decision.should_run('org/repo', 'refs/heads/main', sha, responses('skipped')))
        self.assertTrue(nightly_decision.should_run('org/repo', 'refs/heads/main', sha, responses('success', 'failure')))
        self.assertTrue(nightly_decision.should_run('org/repo', 'refs/heads/main', sha, responses('success', head='b' * 40)))
        self.assertTrue(nightly_decision.should_run('org/repo', 'refs/heads/feature', sha, responses('success')))
        self.assertTrue(nightly_decision.should_run('org/repo', 'refs/heads/main', sha, lambda _: {'workflow_runs': []}))


if __name__ == '__main__':
    unittest.main()
