#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Print docs/DATA_POINTS.md as a Metrum AI Bench release-notes section (#202):
# headings move down one level under "## Data points", the generated-file
# comments are dropped, and the per-field tables fold into one <details>
# block so the headline counts stay on top.
#
#   scripts/release_notes_data_points.sh > notes.md

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DOC="${ROOT}/docs/DATA_POINTS.md"
[[ -f "${DOC}" ]] || { echo "error: ${DOC} missing" >&2; exit 1; }

awk '
  /^<!--/ { next }
  /^# Data points$/ { print "## Data points"; next }
  /^## `summary.v3` quantities$/ && !folded {
    print "<details><summary>Every counted field and when it fires</summary>"
    print ""
    folded = 1
  }
  /^#+ / { sub(/^#/, "##") }
  { print }
  END { if (folded) { print ""; print "</details>" } }
' "${DOC}" | cat -s
