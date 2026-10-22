<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Self-hosted CI runner cache

The `bench-cli` self-hosted CI job stores Cargo's registry and build artifacts
under `${{ runner.tool_cache }}/bench-cli`. The runner tool-cache/work directory
must persist across job and container restarts; this job intentionally does not
use `Swatinem/rust-cache`. The active `pikachu-agent-bench-cli` runner is
reported to run in a persistent agent-workstation container with its home on a
persistent host volume. Rustup keeps its existing `RUSTUP_HOME`; the job sets a
separate persistent `CARGO_HOME` and adds its `bin` directory to `PATH`.

The repository cannot independently verify the live runner's volume mounts,
which paths persist across container recreation, or the directory's ownership
and write permissions. Confirm these operational details before relying on
cache reuse.

The old `deploy/github-runners-bench-cli/docker-compose.yml` pool is obsolete
for the active runner. If you operate that legacy Compose pool, mount a
persistent volume at `/opt/hostedtoolcache` (the runner tool-cache path used by
that pool).
