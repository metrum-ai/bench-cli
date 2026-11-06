#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
# Offline REL-001 classifier plus docs/help/schema/export-label checks.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
[[ $# == 1 && "$1" =~ ^[0-9a-f]{40}$ ]] || { echo 'usage: before_freeze.sh <candidate-sha>' >&2; exit 2; }
[[ "$(git -C "$ROOT" rev-parse HEAD)" == "$1" ]] || { echo 'candidate checkout SHA mismatch' >&2; exit 1; }
[[ -z "$(git -C "$ROOT" status --porcelain --untracked-files=no)" ]] || { echo 'candidate checkout has tracked modifications' >&2; exit 1; }
python3 -c 'import sys; sys.version_info >= (3, 12) or sys.exit("certification harness requires Python 3.12 or newer")'
cd "$ROOT/scripts/validation/cert"
python3 -m unittest test_release_prerelease
cd "$ROOT"
cargo xtask check-docs
cargo test --test schema_migration --test data_points --test cli_docs
printf 'BEFORE-FREEZE-OK %s\n' "$1"
