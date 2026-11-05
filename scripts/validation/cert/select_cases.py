#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Freeze reproducible core plus stratified release-case selection."""
import argparse
import csv
import fnmatch
import hashlib
import json
import math
import sys
from pathlib import Path

import validate_results

# Unmapped paths deliberately select every extended area rather than silently
# overlooking an effect. Broad shared infrastructure maps to all areas.
PATH_AREAS = {
    '.github/**': {'CI'},
    'scripts/validation/cert/**': {'CI'},
    'docs/**': {'REL', 'PACK', 'POST', 'CI'},
    'tests/**': set(),
    'Cargo.toml': {'*'}, 'Cargo.lock': {'*'},
    'src/bin/**': {'*'},
    'src/chat_stream.rs': {'*'}, 'src/measurement.rs': {'*'},
    'src/usage.rs': {'*'}, 'src/stats.rs': {'*'}, 'src/summary.rs': {'*'}, 'src/sse.rs': {'*'},
    'src/args_common.rs': {'CLI', 'MODE', 'LIVE', 'GPU', 'INTEG'},
    'src/asr.rs': {'LIVE', 'GPU', 'MATH', 'SCHEMA'},
    'src/compare.rs': {'PERF', 'COMP', 'CI'},
    'src/concurrency.rs': {'MODE', 'MATH', 'GPU', 'PERF'},
    'src/connect_timing.rs': {'CLI', 'SSE', 'GPU', 'MATH'},
    'src/count_points.rs': {'MODE', 'MATH', 'GPU'},
    'src/endpoints.rs': {'MODE', 'GPU', 'INTEG'},
    'src/endpoints_export.rs': {'MODE', 'SCHEMA', 'GPU', 'INTEG'},
    'src/environment.rs': {'HW', 'TELEM', 'GPU', 'INTEG'},
    'src/error.rs': {'CLI', 'SSE', 'MODE', 'GPU', 'REASON'},
    'src/http_client.rs': {'CLI', 'SSE', 'MODE', 'LIVE', 'GPU', 'PERF'},
    'src/imagegen.rs': {'LIVE', 'GPU', 'MATH', 'SCHEMA'},
    'src/isl_osl.rs': {'MATH', 'MODE', 'GPU', 'COMP', 'PERF'},
    'src/jsonl.rs': {'SCHEMA', 'CLI', 'MODE', 'GPU', 'INTEG'},
    'src/lib.rs': {'*'},
    'src/load.rs': {'MODE', 'MATH', 'GPU', 'PERF'},
    'src/preflight.rs': {'CLI', 'LIVE', 'GPU', 'HW'},
    'src/prompt_inputs.rs': {'CLI', 'MODE', 'GPU', 'PERF', 'COMP'},
    'src/prompt_library.rs': {'CLI', 'MODE', 'GPU', 'PERF', 'COMP'},
    'src/prompt_sequence.rs': {'CLI', 'MODE', 'GPU', 'PERF', 'COMP'},
    'src/record.rs': {'SCHEMA', 'MATH', 'GPU', 'REASON', 'INTEG'},
    'src/request.rs': {'CLI', 'SSE', 'REASON', 'GPU', 'MATH'},
    'src/runner.rs': {'*'},
    'src/strategic.rs': {'MODE', 'MATH', 'GPU', 'PERF', 'COMP', 'SCHEMA', 'INTEG'},
    'src/sut.rs': {'HW', 'SCHEMA', 'LIVE', 'GPU', 'INTEG'},
    'src/sweep_modality.rs': {'MODE', 'LIVE', 'GPU', 'MATH'},
    'src/telemetry/config.rs': {'TELEM', 'SCHEMA', 'GPU', 'INTEG'},
    'src/telemetry/epoch.rs': {'TELEM', 'SCHEMA', 'GPU', 'INTEG'},
    'src/telemetry/mod.rs': {'TELEM', 'SCHEMA', 'GPU', 'INTEG'},
    'src/telemetry/parser.rs': {'TELEM', 'SCHEMA', 'GPU', 'INTEG'},
    'src/telemetry/row.rs': {'TELEM', 'SCHEMA', 'GPU', 'INTEG'},
    'src/telemetry/scraper.rs': {'TELEM', 'SCHEMA', 'GPU', 'INTEG'},
    'src/telemetry/session.rs': {'TELEM', 'SCHEMA', 'GPU', 'INTEG'},
    'src/telemetry/writer.rs': {'TELEM', 'SCHEMA', 'GPU', 'INTEG'},
    'src/time_weighted.rs': {'MATH', 'TELEM', 'GPU'},
    'src/tokenizer.rs': {'CLI', 'MATH', 'GPU', 'COMP', 'PERF'},
    'src/vlm.rs': {'LIVE', 'REASON', 'GPU', 'MATH', 'SCHEMA'},
    'dummy-model-server/**': {'CI'},
    'README.md': {'REL', 'PACK', 'POST', 'CI'},
    'CHANGELOG.md': {'REL', 'POST'},
    'scripts/validation/perf/**': {'PERF', 'COMP', 'CI'},
}


def catalog_hash(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_history(paths, catalog):
    releases, seen = [], set()
    for path in paths:
        with path.open(newline='', encoding='utf-8') as stream:
            reader = csv.DictReader(stream)
            if not {'Release', 'ID', 'Result'} <= set(reader.fieldnames or []):
                raise ValueError(f'{path}: history requires Release, ID, Result')
            rows = list(reader)
        names = {r['Release'] for r in rows}
        ids = [r['ID'] for r in rows]
        if len(names) != 1 or not next(iter(names), '').strip() or len(ids) != len(set(ids)):
            raise ValueError(f'{path}: history must contain one release and unique IDs')
        name = next(iter(names))
        if name in seen or any(i not in catalog for i in ids):
            raise ValueError(f'{path}: duplicate release or unknown case')
        if any(r['Result'] not in {'PASS', 'FAIL', 'N-A', 'NOT-SELECTED'} for r in rows):
            raise ValueError(f'{path}: invalid historical result')
        seen.add(name)
        releases.append({'release': name, 'ran': sorted(r['ID'] for r in rows if r['Result'] in {'PASS', 'FAIL'})})
    return releases


def make_selection(catalog, digest, candidate, rate, rotation, changes, history, seed=None):
    seed = candidate if seed is None else seed
    if not isinstance(seed, str) or not seed or len(seed) > 512:
        raise ValueError('invalid sampling seed')
    if not validate_results.SHA.fullmatch(candidate) or not isinstance(rate, (int, float)) or isinstance(rate, bool) or not 0 < rate <= 1:
        raise ValueError('candidate SHA or sample rate invalid')
    if not isinstance(rotation, int) or isinstance(rotation, bool) or rotation < 1:
        raise ValueError('rotation must be a positive integer')
    if not isinstance(changes, list) or any(not isinstance(p, str) or not p or p.startswith('/') or '..' in p.split('/') for p in changes) or changes != sorted(set(changes)):
        raise ValueError('changed paths must be sorted unique repository-relative paths')
    if not isinstance(history, list) or any(not isinstance(x, dict) or set(x) != {'release', 'ran'} or not isinstance(x['release'], str) or not x['release'] or not isinstance(x['ran'], list) or x['ran'] != sorted(set(x['ran'])) or any(c not in catalog for c in x['ran']) for x in history) or len({x['release'] for x in history}) != len(history):
        raise ValueError('invalid history')
    active = {k: r for k, r in catalog.items() if r['Tier'] != 'PERIODIC'}
    if any(r.get('Selection') not in {'core', 'extended'} or not r.get('Area') or not r.get('Parallel group') for r in active.values()):
        raise ValueError('active catalog rows require Selection, Area and Parallel group')
    areas = {r['Area'] for r in active.values()}
    changed_areas, unmapped = set(), []
    for path in changes:
        matched = [area for pattern, area in PATH_AREAS.items() if fnmatch.fnmatchcase(path, pattern)]
        if not matched:
            unmapped.append(path)
            changed_areas.update(areas)
        else:
            for area in matched:
                changed_areas.update(areas if '*' in area else area)
    reasons = {case: [] for case in active}
    extended = {}
    for case, row in active.items():
        if row['Selection'] == 'core':
            reasons[case].append('core')
        else:
            extended.setdefault(row['Area'], []).append(case)
            if row['Area'] in changed_areas:
                reasons[case].append('changed-area')
            if not any(case in r['ran'] for r in history[:rotation]):
                reasons[case].append('rotation')
    # Stable SHA ordering per area avoids dependence on runtime RNG versions.
    for area, cases in sorted(extended.items()):
        cases.sort(key=lambda case: (hashlib.sha256((seed + '\0' + area + '\0' + case).encode()).digest(), case))
        for case in cases[:max(1, math.ceil(len(cases) * rate))]:
            reasons[case].append('sample')
    selected = {case: sorted(why) for case, why in sorted(reasons.items()) if why}
    lanes = {}
    for case in selected:
        row = active[case]
        group = row['Parallel group']
        phase = 'post' if row['Tier'] == 'POST' else 'cleanup' if group == 'CLEANUP' else 'parallel'
        lanes.setdefault((phase, group), []).append(case)
    return {'version': 1, 'candidate_sha': candidate, 'catalog_sha256': digest,
            'sample_rate': rate, 'seed': seed, 'rotation_releases': rotation, 'changed_files': changes,
            'history_newest_first': history, 'changed_areas': sorted(changed_areas & areas),
            'unmapped_files': unmapped, 'selected': selected,
            'selected_count': len(selected), 'eligible_count': len(active),
            'lane_plan': [{'phase': p, 'lane': g, 'cases': v} for (p, g), v in sorted(lanes.items())]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--catalog', type=Path, required=True)
    parser.add_argument('--candidate-sha', required=True)
    parser.add_argument('--changed-files', type=Path, required=True, help='one repo-relative changed path per line; empty file means no changes')
    parser.add_argument('--history', type=Path, action='append', default=[], help='one CSV per release, newest first; repeat')
    parser.add_argument('--sample-rate', type=float, default=0.3)
    parser.add_argument('--seed', help='sampling seed; default is candidate SHA')
    parser.add_argument('--rotation', type=int, default=3)
    parser.add_argument('--out', type=Path, default=Path('selection.json'))
    args = parser.parse_args()
    try:
        catalog = validate_results.load_catalog(args.catalog)
        changes = sorted(set(args.changed_files.read_text(encoding='utf-8').splitlines()))
        value = make_selection(catalog, catalog_hash(args.catalog), args.candidate_sha, args.sample_rate, args.rotation, changes, load_history(args.history, catalog), args.seed)
        args.out.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n', encoding='utf-8')
    except (OSError, ValueError, csv.Error) as exc:
        print(f'selection failed: {exc}', file=sys.stderr)
        return 2
    return 0


if __name__ == '__main__':
    sys.exit(main())
