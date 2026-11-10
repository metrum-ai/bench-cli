<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Frozen SGLang and LMCache execution inputs

The `KVS` and `KVL` adapters implement `main(argv)`, `freeze(args)` and
`preflight(manifest)` for the common `kv-r5-frozen.v1` envelope. Use the shared
`freeze_case.py` and `case_launcher.py` dispatch; r4 manifest parsing is not used
for these engine specifications. The registered adapter validates its own
execution spec and the common envelope retains the measured workflow artifact,
build receipt, harness closure, tool pins and transitive inputs.

The hash-pinned `lane_manifest` is a `kv-r5-c-execution.v1` JSON object with:

| Field | Required meaning |
| --- | --- |
| `case`, `candidate_sha` | One engine case and the full selected commit. |
| `candidate`, `baseline` | Absolute `path` / `sha256` pins; executable binaries. The outer candidate must match the shipped measured workflow artifact. |
| `candidate_binary_sha256`, `baseline_binary_sha256` | Corresponding binary digests. |
| `lanes` | Exactly selected engine configuration under `KVS` or `KVL`; see the engine-specific document. |
| `tools`, `tool_manifest` | Content-pinned native executables and the pin of the tool map; the existing isolated tool policy applies. |
| `hub_cache`, `model_snapshot` | Staged offline hub cache and exact-revision snapshot within it. No credentials directory is mounted. |
| `model_files` | Complete snapshot file inventory, each with canonical resolved file `path` and independently checked `sha256`, including weights, tokenizer and configuration. Resolved files must remain inside the hub cache. |
| `prompt_token_counts` | File pin for `{ "tokenizer_sha256": "...", "counts": { "full-prompt-sha256": token_count } }`. Cover exactly every cohort prompt with positive counts within the demonstrated fit bound. These are frozen tokenizer outputs, not string-length estimates. |
| `cleanup_grace_seconds`, `cgroup_root` | Outer cleanup grace (at least 120 seconds) and the canonical delegated cgroup root. Leave another 120 seconds inside the process deadline for container removal. |
| `timeout_seconds` | Explicit setup/workload/cleanup limit, 180..14400 seconds. The launcher reserves 120 seconds for owned-container cleanup before the outer process deadline; workload timeout always fails. This is independent of planning estimates. |

The fit object and its underlying evidence receipt are also transitive inputs.
Include `driver_version` in the fit: actual GPU identity, driver, total/free
memory and host available memory are checked again immediately before engine
startup. No GPU or host headroom measurement is invented by the freezer.

`main` accepts `--case`, `--manifest`, `--manifest-sha256`, `--out` and optional
`--preflight`. Execution requires the generic frozen-envelope launcher and its
isolated tool environment. Input mutation, missing workflow origin, wrong
command, changed candidate, incomplete model lock or missing transitive pin
blocks before load. It never falls back to an r4 serving lane.

The engine shell reuses common owner labels, failure records, phase receipts
and cleanup reconciliation. It invokes only its own selected runner. Passing
results are provisional until the owned server is removed and reconciled.
The r5 Stage-0 checklist additionally verifies actual image/GPU/source identity,
launcher terminal outcome, smoke-intake rejection and proven cleanup. The r4
Stage-0 adapter remains closed to KV images.
