#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Append a timed phase to timings.jsonl.
# Usage: phase_timer.sh <timings.jsonl> <phase> <tool> -- <command...>
# Or:    phase_timer.sh <timings.jsonl> start <phase> <tool>
#        phase_timer.sh <timings.jsonl> end <phase> <tool> [notes]

set -euo pipefail

TIMINGS="${1:?timings path}"
shift

iso_now() { date -u +"%Y-%m-%dT%H:%M:%SZ"; }
epoch_s() { date +%s; }

append_json() {
  local phase="$1" tool="$2" start_iso="$3" end_iso="$4" duration="$5" notes="${6:-}"
  mkdir -p "$(dirname "$TIMINGS")"
  printf '{"phase":%s,"tool":%s,"start":%s,"end":%s,"duration_s":%s,"notes":%s}\n' \
    "$(jq -Rn --arg s "$phase" '$s')" \
    "$(jq -Rn --arg s "$tool" '$s')" \
    "$(jq -Rn --arg s "$start_iso" '$s')" \
    "$(jq -Rn --arg s "$end_iso" '$s')" \
    "$duration" \
    "$(jq -Rn --arg s "$notes" '$s')" >>"$TIMINGS"
}

mode="${1:-}"
case "$mode" in
  start)
    phase="${2:?}"; tool="${3:?}"
    mkdir -p "$(dirname "$TIMINGS")"
    echo "$(epoch_s)|$(iso_now)|${phase}|${tool}" >"${TIMINGS}.${phase}.${tool}.start"
    ;;
  end)
    phase="${2:?}"; tool="${3:?}"; notes="${4:-}"
    start_file="${TIMINGS}.${phase}.${tool}.start"
    [[ -f "$start_file" ]] || { echo "missing start marker $start_file" >&2; exit 1; }
    IFS='|' read -r start_epoch start_iso _ _ <"$start_file"
    end_epoch="$(epoch_s)"
    end_iso="$(iso_now)"
    duration=$((end_epoch - start_epoch))
    append_json "$phase" "$tool" "$start_iso" "$end_iso" "$duration" "$notes"
    rm -f "$start_file"
    ;;
  --)
    die_msg='use: phase_timer.sh timings.jsonl <phase> <tool> -- cmd...'
    echo "$die_msg" >&2
    exit 2
    ;;
  *)
    phase="$mode"
    shift
    tool="${1:?tool}"
    shift
    if [[ "${1:-}" != "--" ]]; then
      echo "usage: $0 timings.jsonl <phase> <tool> -- <command...>" >&2
      exit 2
    fi
    shift
    start_iso="$(iso_now)"
    start_epoch="$(epoch_s)"
    set +e
    "$@"
    rc=$?
    set -e
    end_iso="$(iso_now)"
    end_epoch="$(epoch_s)"
    duration=$((end_epoch - start_epoch))
    append_json "$phase" "$tool" "$start_iso" "$end_iso" "$duration" "exit=$rc"
    exit "$rc"
    ;;
esac
