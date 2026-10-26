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

The fallback pool and its opt-in GPU runner
(`deploy/github-runners-bench-cli`) are not currently deployed: the
operator retired the compose pool and no repo-level or `gpu-h100` runners
are registered. The directory remains the fallback and GPU-gate definition;
any next deployment must use its non-root image: `USER 1001:121`,
`RUN_AS_ROOT=false`, no passwordless sudo for the runner account, and
toolchains under `/opt/rust` and `/usr/local/go` instead of `/root` (#292).
The GPU service's host Docker socket mount stays host-root-equivalent by
design; for that service the change is defense in depth, not a hard
boundary.
