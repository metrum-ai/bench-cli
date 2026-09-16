#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
# Fails if any commit in the PR range lacks a Signed-off-by trailer (DCO 1.1).
set -euo pipefail
base="${1:?base sha}"
head="${2:?head sha}"
rc=0
for c in $(git rev-list "$base..$head" --no-merges); do
  if ! git log -1 --format=%B "$c" | grep -qE '^Signed-off-by: .+ <.+@.+>$'; then
    echo "DCO: commit $c lacks Signed-off-by (use git commit -s)" >&2
    rc=1
  fi
done
exit "$rc"
