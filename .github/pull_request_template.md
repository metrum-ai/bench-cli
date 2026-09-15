<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

## Summary

- 

## Test plan

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `METRUM_BENCH_REQUIRE_DUMMY=1 cargo test --all-targets --all-features`
- [ ] `cargo deny check` (when dependency or deny.toml changes)
