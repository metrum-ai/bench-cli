#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Negative test for check_naming: a tree containing "MetrumBench" must fail.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TMP="$(mktemp -d)"
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT

cd "$TMP"
git init -q
git config user.email "naming-test@example.com"
git config user.name "Naming Test"
# Minimal allowlist (empty) and a forbidden token in a tracked file.
: > .naming-allow
printf 'MetrumBench is a forbidden product name.\n' > bad.txt
git add .naming-allow bad.txt
git commit -q -m "seed forbidden name"

# Copy the naming functions by invoking the real script from a checkout that
# uses this temp tree as ROOT via a wrapper: run check_naming logic in-place.
cp "$ROOT/scripts/check_headers.sh" ./check_headers.sh
# The script cds to its parent (repo root). Place it under scripts/ here.
mkdir -p scripts
mv check_headers.sh scripts/check_headers.sh
# Only naming matters; provide a no-op headers scan by ensuring no .rs/.sh/.yml
# under scan (except the script itself which has headers).
if scripts/check_headers.sh; then
  echo "check_naming_test: expected non-zero exit when MetrumBench is present" >&2
  exit 1
fi

echo "check_naming_test: ok (forbidden name rejected)"
