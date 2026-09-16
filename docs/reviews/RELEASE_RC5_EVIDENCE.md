<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# RC5 release gate evidence — 2026-09-16

Local gate on `main` at `1.0.0-rc.5` (post-merge of #78–#84):

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --all-targets --all-features -- -D warnings` | pass |
| `dummy-model-server` `go vet` + `go build` | pass |
| `cargo test --all-features` | pass |
| `cargo package --allow-dirty --no-verify` | pass (`metrum-ai-bench` `1.0.0-rc.5`) |
| `cargo deny check` | pass |
| `gitleaks git --config .gitleaks.toml` | pass (historical leak commits allowlisted) |
| `scripts/check_headers.sh` | pass |
| `scripts/tests/check_naming_test.sh` | pass |
| `scripts/tests/gitleaks/run.sh` | pass |
| zero `unsafe` in `src/` | pass |
| no `v1.0.0` tag | pass |
| boundary strings (license key / upgrade / sales) | pass |
| `cargo llvm-cov --lib … --fail-under-lines 80` | pass |
| determinism with/without `--sut` (seed 42) | pass (distributions match; `sut`/`hostname` differ) |
| deprecated modality shim notice names `metrum-ai-bench-llm` | pass |

Tag: annotated `v1.0.0-rc.5` (no signing key configured).

Phase 3: deleted tags/releases `v0.1.80`–`v0.1.82`. Bundle at
`/home/cgadgil/src/bench-cli-pre-remediation-20260916T204234Z.bundle`.
Ancestor commits of `main` still contain the blob (see HISTORY_REWRITE.md);
human GitHub Support purge + credential rotation still required.
