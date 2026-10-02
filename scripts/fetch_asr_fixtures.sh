#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Metrum AI Bench CLI: fetch and convert the ASR speech fixtures in
# test-data/asr/ from LibriSpeech test-clean (CC BY 4.0, https://www.openslr.org/12).
#
# Downloads test-clean.tar.gz once into a cache directory, checks its MD5
# against the value OpenSLR publishes, extracts only the utterances listed in
# UTTERANCES, and converts each to 16 kHz mono 16-bit PCM WAV with ffmpeg in
# bitexact mode so reruns produce identical bytes. Then it rewrites
# test-data/asr/truth.jsonl (exact LibriSpeech transcripts) and
# test-data/asr/input.jsonl (metrum-ai-bench-cli-asr manifest).
#
# Usage: scripts/fetch_asr_fixtures.sh [--out DIR] [--cache DIR]
# Requires: curl, tar, md5sum, ffmpeg, ffprobe.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${ROOT}/test-data/asr"
CACHE="${XDG_CACHE_HOME:-${HOME}/.cache}/metrum-ai-bench-cli/librispeech"
TARBALL_URL="https://www.openslr.org/resources/12/test-clean.tar.gz"
MD5_URL="https://www.openslr.org/resources/12/md5sum.txt"

# speaker-chapter-utterance IDs: three speakers, 1.8 s to 7.8 s, 395 KB of WAV in total.
UTTERANCES=(
  "1089-134686-0030"
  "2961-961-0016"
  "6930-81414-0005"
)

while [[ $# -gt 0 ]]; do
  case "$1" in
    --out) OUT="$2"; shift 2 ;;
    --cache) CACHE="$2"; shift 2 ;;
    -h|--help) sed -n '4,16p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

for tool in curl tar md5sum ffmpeg ffprobe; do
  command -v "$tool" >/dev/null || { echo "missing required tool: $tool" >&2; exit 1; }
done

mkdir -p "$CACHE" "$OUT"
tarball="${CACHE}/test-clean.tar.gz"

expected_md5="$(curl -fsSL "$MD5_URL" | awk '$2 == "test-clean.tar.gz" {print $1}')"
[[ -n "$expected_md5" ]] || { echo "could not read MD5 for test-clean.tar.gz from $MD5_URL" >&2; exit 1; }

if [[ ! -f "$tarball" ]] || [[ "$(md5sum "$tarball" | awk '{print $1}')" != "$expected_md5" ]]; then
  echo "downloading $TARBALL_URL into $CACHE"
  curl -fL --retry 3 -C - -o "$tarball" "$TARBALL_URL"
fi
actual_md5="$(md5sum "$tarball" | awk '{print $1}')"
[[ "$actual_md5" == "$expected_md5" ]] || { echo "MD5 mismatch: got $actual_md5, want $expected_md5" >&2; exit 1; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

members=()
for utt in "${UTTERANCES[@]}"; do
  IFS=- read -r spk chap _ <<<"$utt"
  members+=("LibriSpeech/test-clean/${spk}/${chap}/${utt}.flac")
  members+=("LibriSpeech/test-clean/${spk}/${chap}/${spk}-${chap}.trans.txt")
done
# shellcheck disable=SC2046
tar -xzf "$tarball" -C "$work" $(printf '%s\n' "${members[@]}" | sort -u)

: > "${OUT}/truth.jsonl"
: > "${OUT}/input.jsonl"
for utt in "${UTTERANCES[@]}"; do
  IFS=- read -r spk chap _ <<<"$utt"
  src="${work}/LibriSpeech/test-clean/${spk}/${chap}/${utt}.flac"
  dst="${OUT}/${utt}.wav"
  ffmpeg -nostdin -loglevel error -y -i "$src" \
    -map_metadata -1 -fflags +bitexact -flags:a +bitexact \
    -ac 1 -ar 16000 -sample_fmt s16 -c:a pcm_s16le "$dst"
  transcript="$(awk -v id="$utt" '$1 == id {sub(/^[^ ]+ /, ""); print}' \
    "${work}/LibriSpeech/test-clean/${spk}/${chap}/${spk}-${chap}.trans.txt")"
  [[ -n "$transcript" ]] || { echo "no transcript for $utt" >&2; exit 1; }
  duration="$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$dst")"
  printf '{"id":"%s","transcript":"%s"}\n' "$utt" "$transcript" >> "${OUT}/truth.jsonl"
  printf '{"id":"%s","path":"test-data/asr/%s.wav","format":"wav","duration":%.3f}\n' \
    "$utt" "$utt" "$duration" >> "${OUT}/input.jsonl"
  echo "wrote $dst (${duration}s)"
done
