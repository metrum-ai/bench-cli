#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Metrum AI Bench CLI: offline self-test for scripts/live/assert_headline.sh.
# Builds synthetic data logs (no server, no GPU) and checks that each
# failure rule fires and that a healthy log passes.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ASSERT="${ROOT}/scripts/live/assert_headline.sh"
work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

req() { # seq error_json modality_metrics_json
  printf '{"schema_version":"metrum-ai-bench-cli.request.v3","seq":%s,"phase":"measure","error":%s,"modality_metrics":%s}\n' "$1" "$2" "$3"
}
summary() { # attempted successes require_sut
  printf '{"schema_version":"metrum-ai-bench-cli.summary.v3","attempted":%s,"successes":%s,"errors_by_type":{},"sut":{"name":"x"},"config":{"common":{"require_sut":%s}}}\n' "$1" "$2" "$3"
}

failures=0
expect() { # want_rc label modality log [args...]
  local want="$1" label="$2"; shift 2
  local rc=0
  "${ASSERT}" "$@" >"${work}/out.txt" 2>&1 || rc=$?
  if [[ "${want}" == pass && "${rc}" -ne 0 ]] || [[ "${want}" == fail && "${rc}" -eq 0 ]]; then
    echo "assert_headline_test: FAIL ${label} (rc=${rc})"; cat "${work}/out.txt"; failures=$((failures + 1))
  fi
}

{ req 0 null '{"wer":0.1,"cer":0.05}'; req 1 null '{"wer":0.0,"cer":0.0}'; summary 2 2 true; } >"${work}/asr-ok.jsonl"
expect pass "asr healthy" asr "${work}/asr-ok.jsonl"

{ req 0 null '{"rtfx_client":3.0}'; summary 1 1 true; } >"${work}/asr-nower.jsonl"
expect fail "asr without wer" asr "${work}/asr-nower.jsonl"

{ req 0 '{"kind":"http_status","status":400}' '{}'; summary 1 0 true; } >"${work}/zero.jsonl"
expect fail "zero successes" llm "${work}/zero.jsonl"

{ req 0 null '{}'; req 1 '{"kind":"timeout"}' '{}'; summary 2 1 true; } >"${work}/half.jsonl"
expect fail "below default ratio" llm "${work}/half.jsonl"
MIN_SUCCESS_RATIO=0.5 expect pass "ratio override" llm "${work}/half.jsonl"

{ req 0 null '{}'; summary 1 1 false; } >"${work}/nosut.jsonl"
expect fail "no --require-sut" llm "${work}/nosut.jsonl"

{ req 0 null '{"image_count":0}'; summary 1 1 true; } >"${work}/vlm-noimg.jsonl"
expect fail "vlm image_count 0" vlm "${work}/vlm-noimg.jsonl"

{ req 0 null '{"images_returned":0}'; summary 1 1 true; } >"${work}/img-none.jsonl"
expect fail "imagegen returned nothing" imagegen "${work}/img-none.jsonl"

{ req 0 null '{"images_returned":1}'; summary 1 1 true; } >"${work}/img-ok.jsonl"
mkdir -p "${work}/art-bad" "${work}/art-good"
printf 'not a png' >"${work}/art-bad/000001-0.png"
cp "${ROOT}/test-data/vlm/shapes-512.png" "${work}/art-good/000001-0.png"
expect fail "imagegen undecodable artifact" imagegen "${work}/img-ok.jsonl" --artifact-dir "${work}/art-bad"
expect pass "imagegen decodable artifact" imagegen "${work}/img-ok.jsonl" --artifact-dir "${work}/art-good"

summary 0 0 true | head -c 20 >"${work}/truncated.jsonl"
expect fail "no summary line" llm "${work}/truncated.jsonl"

if [[ "${failures}" -ne 0 ]]; then
  echo "assert_headline_test: ${failures} case(s) failed"; exit 1
fi
echo "assert_headline_test: ok"
