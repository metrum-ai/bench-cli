<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Self-hosted CI runners

The `bench-cli` CI job runs on the `bench-cli-arc` GitHub Actions runner scale
set, backed by ARC on DOKS. Each job gets a fresh, single-use runner pod, so
Rust caches are managed by `Swatinem/rust-cache` rather than persistent runner
volumes. Nodes scale from zero, so the first job after idle can wait several
minutes while a node starts.

The `pikachu-agent-bench-cli` container runner remains available as a fallback
until ARC has been proven in CI. Do not deregister or otherwise remove it based
on this transition alone. The repository cannot independently verify the live
runner pool or its operational configuration; confirm the actual runner name
and job result from GitHub Actions before treating ARC as proven.
