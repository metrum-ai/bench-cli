<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Skill: analyze bench-cli telemetry NDJSON

When the user points at a strategic `--ndjson` file:

1. Read `docs/TELEMETRY.md` and `docs/telemetry/ANALYSIS.md`.
2. Decompress if needed (`zstd -d` / `gunzip`).
3. Run `python3 docs/queries/analyze.py <file>` or DuckDB SQL under `docs/queries/`.
4. Use only metric names present in the file or the companion `telemetry.yaml`.
5. Do not call a nonexistent `correlate` CLI; analysis is offline.
