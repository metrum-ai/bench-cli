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

rm bad.txt
printf 'Metrum AI Bench CLI, formerly Metrum Insights CLI\n' > transition.txt
git add -A
git commit -q -m "use approved transition form"
if ! scripts/check_headers.sh; then
  echo "check_naming_test: approved transition form was rejected" >&2
  exit 1
fi

printf 'Metrum AI Bench CLI, formerly Metrum Insights CLI; avoid Insights CLI.\n' > transition.txt
git add transition.txt
git commit -q -m "append a second forbidden legacy name"
if scripts/check_headers.sh; then
  echo "check_naming_test: transition exception hid a second legacy name" >&2
  exit 1
fi

printf 'Metrum Insights CLI is the current product name.\n' > transition.txt
git add transition.txt
git commit -q -m "use forbidden legacy name"
if scripts/check_headers.sh; then
  echo "check_naming_test: expected non-zero exit for a bare legacy name" >&2
  exit 1
fi

printf 'Metrum Bench CLI is missing the canonical AI token.\n' > transition.txt
git add transition.txt
git commit -q -m "use incomplete product name"
if scripts/check_headers.sh; then
  echo "check_naming_test: expected non-zero exit for Metrum Bench CLI" >&2
  exit 1
fi

echo "check_naming_test: ok (forbidden names rejected; transition form accepted)"
