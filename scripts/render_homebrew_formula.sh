#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0

set -euo pipefail

if [[ "$#" -ne 3 ]]; then
  echo "usage: $0 RELEASE_TAG ARTIFACT_DIRECTORY OUTPUT_FORMULA" >&2
  exit 2
fi

release_tag="$1"
artifact_directory="$2"
output_formula="$3"
version="${release_tag#v}"
if [[ "${release_tag}" != "v${version}" || -z "${version}" ]]; then
  echo "release tag must start with v" >&2
  exit 2
fi

checksum() {
  local target="$1"
  local file="${artifact_directory}/metrum-ai-bench-v${version}-${target}.tar.gz.sha256"
  [[ -f "${file}" ]] || {
    echo "missing checksum: ${file}" >&2
    exit 1
  }
  awk 'NF == 2 { print $1; exit }' "${file}"
}

arm_mac="$(checksum aarch64-apple-darwin)"
x64_mac="$(checksum x86_64-apple-darwin)"
arm_linux="$(checksum aarch64-unknown-linux-gnu)"
x64_linux="$(checksum x86_64-unknown-linux-gnu)"

sed \
  -e "s/version \"[^\"]*\"/version \"${version}\"/" \
  -e "0,/RELEASE_WORKFLOW_UPDATES_THIS_VALUE/s//${arm_mac}/" \
  -e "0,/RELEASE_WORKFLOW_UPDATES_THIS_VALUE/s//${x64_mac}/" \
  -e "0,/RELEASE_WORKFLOW_UPDATES_THIS_VALUE/s//${arm_linux}/" \
  -e "0,/RELEASE_WORKFLOW_UPDATES_THIS_VALUE/s//${x64_linux}/" \
  packaging/homebrew/metrum-ai-bench.rb > "${output_formula}"

if grep -q RELEASE_WORKFLOW_UPDATES_THIS_VALUE "${output_formula}"; then
  echo "formula still contains checksum placeholders" >&2
  exit 1
fi
