#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Skip unchanged main only after a successful *full* nightly on that SHA."""
import json
import os
import subprocess
import sys


def api(path):
    result = subprocess.run(['gh', 'api', path], check=True, capture_output=True, text=True, timeout=30)
    return json.loads(result.stdout)


def should_run(repo, ref, sha, get=api):
    if ref != 'refs/heads/main':
        return True
    runs = get(f'repos/{repo}/actions/workflows/regular-tests.yml/runs?branch=main&status=success&per_page=100')['workflow_runs']
    for run in runs:
        jobs = get(f"repos/{repo}/actions/runs/{run['id']}/jobs?per_page=100")['jobs']
        if any(job.get('name') == 'nightly' and job.get('conclusion') == 'success' and
               any(step.get('name') == 'Full suite and ignored tests' and step.get('conclusion') == 'success'
                   for step in job.get('steps', [])) for job in jobs):
            return run.get('head_sha') != sha
    # Never skip on absent, truncated or unverifiable history.
    return True


def main():
    try:
        run = should_run(os.environ['GITHUB_REPOSITORY'], os.environ['GITHUB_REF'], os.environ['GITHUB_SHA'])
    except (KeyError, OSError, ValueError, TypeError, subprocess.SubprocessError) as exc:
        print(f'Could not verify previous full nightly; running tests: {exc}', file=sys.stderr)
        run = True
    with open(os.environ['GITHUB_OUTPUT'], 'a', encoding='utf-8') as output:
        output.write(f"run={'true' if run else 'false'}\n")
    print('Run full nightly' if run else 'No main commits since last successful full nightly; skip')
    return 0


if __name__ == '__main__':
    sys.exit(main())
