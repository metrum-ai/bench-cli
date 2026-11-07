#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
CERT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
[[ $# -ge 4 && "$1" == --case && "$3" == --lane ]] || { echo 'required: --case ID --lane LANE followed by lane arguments' >&2; exit 2; }
case_id=$2
lane=$4
shift 4
[[ "$case_id" =~ ^BC-[A-Z]+-[0-9]{3}$ ]] || { echo 'invalid case ID' >&2; exit 2; }
[[ "$lane" =~ ^(g[1-7]|l[1-4]|p[12]|r|d|v|w|comp002)$ ]] || { echo 'invalid case lane' >&2; exit 2; }
[[ -z "${CERT_CASE_ID:-}" || "$CERT_CASE_ID" == "$case_id" ]] || { echo 'case does not match frozen launcher identity' >&2; exit 2; }
export CERT_CASE_ID=$case_id
exec bash "$CERT_DIR/lane-$lane.sh" "$@"
