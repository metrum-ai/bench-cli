# shellcheck shell=bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Metrum AI Bench CLI live-script helper: find a prebuilt bench binary.
# Source this file; it defines functions only. It never compiles.
#
# Preference order (README "Before you benchmark", docs/SERVING.md):
#   1. BENCH_BIN_DIR, when set (an explicit choice always wins)
#   2. <root>/bin                    unpacked GitHub Release tarball
#   3. <root>/target/release         existing cargo release build
#   4. <root>/target/rel-user/release  release build made with --target-dir
#                                      when target/release is not writable
#   5. PATH
# Debug builds (target/debug) are never picked implicitly: their timings are
# not comparable. Point BENCH_BIN_DIR at them on purpose for plumbing tests.
# If nothing is found, build once in the primary checkout
# (`cargo build --release --bins`) or unpack a release; do not build in a
# second worktree.

# bench_bin_resolve ROOT NAME -> absolute path on stdout, or error and rc 1
bench_bin_resolve() {
  local root="$1" want="$2" c
  for c in ${BENCH_BIN_DIR:+"${BENCH_BIN_DIR}/${want}"} \
           "${root}/bin/${want}" \
           "${root}/target/release/${want}" \
           "${root}/target/rel-user/release/${want}"; do
    [[ -x "${c}" ]] && { echo "${c}"; return 0; }
  done
  if command -v "${want}" >/dev/null 2>&1; then command -v "${want}"; return 0; fi
  echo "error: ${want} not found (checked BENCH_BIN_DIR, ${root}/bin, target/release, target/rel-user/release, PATH)." >&2
  echo "       Unpack a release tarball or run one 'cargo build --release --bins' in the primary checkout." >&2
  return 1
}

# bench_bin_identity BIN ROOT -> "path=<bin> version=<--version> commit=<git describe>"
# for SUT notes, so a tip build that still prints the last release version is
# not mistaken for that release.
bench_bin_identity() {
  local bin="$1" root="$2" ver commit
  ver="$("${bin}" --version 2>/dev/null | head -n1 || echo unknown)"
  commit="$(git -C "${root}" describe --always --dirty --tags 2>/dev/null || echo "not a git checkout")"
  echo "path=${bin} version=${ver} checkout=${commit}"
}
