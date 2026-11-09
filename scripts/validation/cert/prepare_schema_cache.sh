#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
# Stage locked dependencies for the offline schema fixture, before rental.
# This is setup, not execution evidence or a certification verdict.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$ROOT"
cargo fetch --locked
cargo test --locked --offline --test schema_history --test schema_migration --no-run
printf '%s\n' 'Locked schema dependencies and test executables prepared; not certification evidence.'
