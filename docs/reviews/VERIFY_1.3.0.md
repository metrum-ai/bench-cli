<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Verify 1.3.0 packaged release

Checklist for operators verifying a **GitHub Release** tarball and live docs
after tagging `v1.3.0`. Do not mark items done until you have run them against
the published assets.

Replace `TARGET` with the archive you are checking, for example
`x86_64-unknown-linux-gnu` or `aarch64-apple-darwin`.

## Download and integrity

```bash
TAG=v1.3.0
TARGET=x86_64-unknown-linux-gnu
ARCHIVE="metrum-ai-bench-cli-${TAG}-${TARGET}.tar.gz"

gh release download "$TAG" --repo metrum-ai/bench-cli --pattern "${ARCHIVE}*"
sha256sum -c "${ARCHIVE}.sha256"   # or: shasum -a 256 -c "${ARCHIVE}.sha256"
```

## Sigstore / provenance (when repo is public)

```bash
cosign verify-blob \
  --bundle "${ARCHIVE}.sigstore.json" \
  --certificate-identity-regexp 'https://github.com/metrum-ai/bench-cli/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  "${ARCHIVE}"

gh attestation verify "${ARCHIVE}" --repo metrum-ai/bench-cli
```

## Unpack smoke

```bash
tar -xzf "${ARCHIVE}"
root="${ARCHIVE%.tar.gz}"

test -f "${root}/TRADEMARKS.md"
test -f "${root}/docs/RESULTS_PUBLICATION_POLICY.md"
test -f "${root}/docs/CLAIMS_LEDGER.md"
test -f "${root}/examples/sut.example.json"
test -f "${root}/test-data/llm-hi.jsonl"

"${root}/bin/metrum-ai-bench-cli" --version | grep -q '1.3.0'
"${root}/bin/metrum-ai-bench-cli" --help | grep -qi benchmark
"${root}/bin/metrum-ai-bench-cli" selftest
"${root}/bin/metrum-ai-bench-cli-mock-server" --help | grep -qi mock
"${root}/bin/dummy-model-server" -h 2>&1 | grep -qi 'Listen port'
```

Confirm **no** shim binaries are present:

```bash
! ls "${root}/bin"/metrumbench-* 2>/dev/null
! test -x "${root}/bin/metrum-ai-bench"
! test -x "${root}/bin/metrum-ai-bench-llm"
```

Primary names that must exist:

```bash
for b in \
  metrum-ai-bench-cli \
  metrum-ai-bench-cli-llm \
  metrum-ai-bench-cli-vlm \
  metrum-ai-bench-cli-asr \
  metrum-ai-bench-cli-imagegen \
  metrum-ai-bench-cli-prompts \
  metrum-ai-bench-cli-strategic \
  metrum-ai-bench-cli-mock-server \
  dummy-model-server
do
  test -x "${root}/bin/${b}"
done
```

## Live docs (after tag deploy)

- [ ] `https://docs.metrum.ai/metrum-ai-bench-cli/latest/` shows **1.3.0**
- [ ] Quickstart / Platforms do not recommend Homebrew
- [ ] Release notes Unreleased / 1.3.0 section lists shim and Homebrew removal

## Status

Verification of this checklist against published `v1.3.0` assets has **not**
been recorded in-tree by the release-prep change. Run the commands above after
the GitHub Release exists and tick items in your release notes or ops log.
