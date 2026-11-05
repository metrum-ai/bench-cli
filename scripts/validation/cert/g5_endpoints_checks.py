#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Check a real two-endpoint disruption; fixture use is not GPU evidence."""
import datetime as dt
import json

import g6_checks


def timestamp(value):
    if not isinstance(value, str):
        raise ValueError('missing UTC event timestamp')
    parsed = dt.datetime.fromisoformat(value.replace('Z', '+00:00'))
    if parsed.tzinfo is None:
        raise ValueError('event timestamp must include a timezone')
    return parsed.timestamp()


def verify(data, traces, interruption, exit_code):
    records = [json.loads(line) for line in data.splitlines() if line.strip()]
    rows = [r for r in records if '.request.' in r.get('schema_version', '') and r.get('phase') == 'measure']
    summaries = [r for r in records if '.summary.' in r.get('schema_version', '')]
    if exit_code == 0 or not rows or len(summaries) != 1 or summaries[0].get('attempted') != len(rows):
        raise ValueError('measured rows, one summary and nonzero safety exit required')
    if sorted(r.get('seq') for r in rows) != list(range(len(rows))):
        raise ValueError('missing, duplicate or unordered request slots')
    if set(traces) != {'survivor', 'victim'} or any(r.get('endpoint') not in traces for r in rows):
        raise ValueError('both pinned endpoint identities are required')
    if interruption.get('kill_exit') != 0 or interruption.get('kill_end_ns', 0) < interruption.get('kill_start_ns', 0):
        raise ValueError('real server kill evidence is missing')
    kill_start, kill_end = timestamp(interruption.get('kill_start_utc')), timestamp(interruption.get('kill_end_utc'))
    if kill_end < kill_start:
        raise ValueError('invalid server kill wall clock')
    for name in traces:
        prior = [r for r in rows if r['endpoint'] == name and r.get('error') is None and r.get('answer_completed') is True
                 and timestamp(r.get('completed_at')) < kill_start]
        if not prior:
            raise ValueError(f'{name} lacks a completed answer before the real kill')
        events = [json.loads(line) for line in traces[name].splitlines() if line.strip()]
        complete = {e['request'] for e in events if e.get('kind') == 'complete' and e.get('monotonic_ns', 0) < interruption['kill_start_ns']}
        if not complete:
            raise ValueError(f'{name} lacks a real completed stream before the kill')
        prior_trace = '\n'.join(json.dumps(e) for e in events if e.get('request') in complete)
        if not g6_checks.server_usage(prior_trace):
            raise ValueError(f'{name} lacks real server usage')
    failed = [r for r in rows if r['endpoint'] == 'victim' and r.get('error') is not None
              and timestamp(r.get('completed_at')) >= kill_start]
    survived = [r for r in rows if r['endpoint'] == 'survivor' and r.get('error') is None
                and r.get('answer_completed') is True and timestamp(r.get('started_at')) > kill_end]
    if not failed or not survived:
        raise ValueError('victim failure and survivor post-kill answer are required')
    survivor_events = [json.loads(line) for line in traces['survivor'].splitlines() if line.strip()]
    if not any(e.get('kind') == 'send' and e.get('monotonic_ns', 0) > interruption['kill_end_ns'] for e in survivor_events):
        raise ValueError('survivor post-kill real send missing')
    errors = sum(r.get('error') is not None for r in rows)
    if summaries[0].get('errors') != errors or summaries[0].get('successes') != len(rows) - errors:
        raise ValueError('summary dropped or double-counted requests')
    return {'attempted': len(rows), 'errors': errors, 'victim_failures': len(failed),
            'survivor_post_kill_answers': len(survived), 'safety_exit': exit_code}
