<!-- Copyright (c) 2026 Metrum AI, Inc. Licensed under the Apache License, Version 2.0. -->

# Contributing to Metrum AI Bench

Thank you for your interest in contributing. This document explains how to
build, test and submit changes.

## Ground rules

- All contributions are licensed under the Apache License, Version 2.0 (see
  LICENSE). By submitting a pull request you agree to license your contribution
  under the same terms (inbound = outbound) and you certify the Developer
  Certificate of Origin (https://developercertificate.org). Sign off each
  commit with `git commit -s`.
- Every authored source file carries a copyright and SPDX identifier in any
  comment style accepted by `scripts/check_headers.sh` (the check requires the
  strings `Copyright (c) 2026 Metrum AI, Inc.` and
  `SPDX-License-Identifier: Apache-2.0`).
- Follow the Code of Conduct (CODE_OF_CONDUCT.md).
- Report security issues privately (see SECURITY.md), never in a public issue.

## Building

Requirements: a stable Rust toolchain (see rust-toolchain.toml) and, for the
end-to-end tests, Go as pinned in `.github/workflows/ci.yml` (currently 1.26.6)
to build the dummy model server.

```bash
cargo build --release
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

## Measurement changes

Metrum AI Bench publishes performance numbers that other people rely on. A
change that affects any reported metric (timing, token counting, percentile
computation, throughput windows, error accounting) must:

1. State the old and new definition of the metric in the pull request
   description and in CHANGELOG.md.
2. Update docs/METRICS.md.
3. Add or update a golden test that pins the new value on a fixed input.
4. Bump the output `schema_version` when the JSON output shape changes.

Track measurement and packaging gaps via GitHub issues labeled `measurement`, `oss-readiness`, and `priority/*`.

## Pull requests

- One logical change per pull request; keep refactors separate from behavior
  changes.
- Use conventional commit prefixes: `feat:`, `fix:`, `docs:`, `test:`,
  `refactor:`, `chore:`.
- CI must pass: fmt, clippy, tests, license check.
- Add a CHANGELOG.md entry under the unreleased section.
- Sign off commits (`git commit -s`) so DCO is satisfied.

## Reporting bugs

Open a GitHub issue with: the exact command line, the tool version
(`--version-only`), the server type and version, the relevant lines from the
data log and debug log, and what you expected instead.
