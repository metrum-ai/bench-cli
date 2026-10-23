#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0

# Set rust=true unless every changed path is documentation-only by policy.
set -euo pipefail

if [[ $# != 3 ]]; then
  echo "usage: $0 EVENT BASE_SHA HEAD_SHA" >&2
  exit 2
fi

event="$1"
base="$2"
head="$3"
output="${GITHUB_OUTPUT:-/dev/stdout}"
rust=true

case "$event" in
  schedule|workflow_dispatch)
    ;;
  pull_request|merge_group|push)
    if [[ -z "$base" || -z "$head" || "$base" =~ ^0+$ ]]; then
      rust=true
    else
      while IFS= read -r path; do
        case "$path" in
          docs/*|external-docs/*|*.md|llms.txt|LICENSE|NOTICE|TRADEMARKS.md|THIRD_PARTY_LICENSES)
            ;;
          *) rust=true; break ;;
        esac
        rust=false
      done < <(git diff --name-only "$base" "$head")
    fi
    ;;
esac

printf 'rust=%s\n' "$rust" >> "$output"
