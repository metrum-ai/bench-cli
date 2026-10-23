#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail

filter="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/rust_changes.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
git -C "$tmp" init -q
git -C "$tmp" config user.name 'CI Path Filter Test'
git -C "$tmp" config user.email 'ci-path-filter@example.invalid'
git -C "$tmp" commit --allow-empty -qm base
base="$(git -C "$tmp" rev-parse HEAD)"

commit_path() {
  local path="$1"
  mkdir -p "$tmp/$(dirname "$path")"
  printf 'fixture\n' > "$tmp/$path"
  git -C "$tmp" add -- "$path"
  git -C "$tmp" commit -qm "add $path"
  head="$(git -C "$tmp" rev-parse HEAD)"
}

assert_filter() {
  local expected="$1" path="$2" output
  commit_path "$path"
  : > "$tmp/output"
  output="$(cd "$tmp" && GITHUB_OUTPUT="$tmp/output" bash "$filter" push "$base" "$head" >/dev/null && cat "$tmp/output")"
  [[ "$output" == "rust=$expected" ]] || {
    echo "path filter for $path: expected rust=$expected, got $output" >&2
    exit 1
  }
  base="$head"
}

assert_filter false docs/guide.txt
assert_filter false external-docs/static/image.svg
assert_filter false README.md
assert_filter false llms.txt
assert_filter false LICENSE
assert_filter false NOTICE
assert_filter false TRADEMARKS.md
assert_filter false THIRD_PARTY_LICENSES
assert_filter true dummy-model-server/main.go
assert_filter false docs/guide.md
assert_filter true deny.toml
assert_filter true .github/workflows/ci.yml

echo 'Rust path filter fixtures: ok'
