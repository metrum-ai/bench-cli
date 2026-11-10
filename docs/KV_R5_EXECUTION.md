<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Frozen KV execution inputs

The `kv_r5_*_runner.py` entry points consume `kv-r5-execution.v1`
manifests. They run on an already assigned single GPU machine with staged
official images, model files and workflow-origin candidate binaries. They do
not provision hardware, fetch models, install engines or build the candidate.
These are development runners; offline tests and exporter fixtures do not
establish live certification or change release selection.

The outer [frozen envelope schema](../schemas/kv-r5-frozen.v1.json) is
`kv-r5-frozen.v1`. `freeze(args)` validates selection/catalog identity,
workflow-origin measured build receipts and every nested pin, then writes a
new envelope with one case. `preflight(manifest)` rechecks the envelope and
returns `cases`. The inner `lane_manifest` pin references the immutable engine
execution specification described below. `main(argv)` accepts `--case ID
--manifest EXECUTION_SPEC --manifest-sha256 HASH --out DIR [--preflight]`.
The sole command must match the case/lane, immutable spec hash and output path.
No shell evaluation or r4 adapter fallback is allowed. A matching SHA string
alone is insufficient: independently verify the workflow artifact origin, build
receipt, complete input closure, actual-host binding and cleanup. Structural
schema validation does not establish those properties.

Invoke the selected engine entry point with `--manifest FILE
--manifest-sha256 SHA256 --out NEW_DIRECTORY`; add `--preflight` to check
frozen local inputs without starting Docker. Full release intake still uses
`validate_results.py`. Missing evidence produces FAIL, never a PENDING result
enum or a zero. Its coordination status can remain pending outside intake.

The manifest must contain:

| Field | Meaning |
| --- | --- |
| `schema_version`, `case`, `candidate_sha` | Exact schema, assigned case and full measured candidate commit. |
| `candidate` | `{path, sha256}` of the measured strategic release executable. |
| `image`, `oci_index`, `oci_child` | Official immutable image reference and independently staged descriptor `{path, sha256}` pins. |
| `source_revision`, `source_files` | Reviewed source revision and absolute runtime source paths mapped to reviewed file hashes. Import-location checks supplement hashing. Image labels alone are insufficient. |
| `model`, `model_revision`, `served_model` | Model identity, full upstream revision and exact served model name. |
| `model_snapshot`, `model_files` | Staged model directory and complete file-lock array of `{path, sha256}` objects, including tokenizer/configuration. Review the completeness against upstream separately. |
| `tools` | Complete native executable pin map required by `execution_policy.validate_case_tools`, including `bash`, `env`, `python3`, `docker`, `nvidia-smi` and the shared launcher utilities. |
| `inputs` | Named `{path, sha256}` pins for every workload, telemetry configuration, engine options file, dependency and harness source. |
| `gpu_uuid`, `gpu_class`, `minimum_gpu_mib` | Actual execution GPU identity and measured fit envelope. Inventory is checked before launch. |
| `port`, `proxy_port`, `metrics_port`, `metrics_internal_port` | Explicit loopback ports; the inference port maps to container port 8000. |
| `timeout_seconds`, `cleanup_grace_seconds` | Entire case envelope; freeze `cleanup_grace_seconds: 120` inside that envelope for scoped cleanup. |
| `cgroup_root` | Canonical delegated cgroup-v2 root for the shared isolated launcher. |
| `owner`, `evidence_link` | Private execution-record provenance. |
| `phases` | Ordered unique phases, each containing `name`, candidate `argv` and `expected_requests`. |

Each phase command invokes only the pinned candidate, uses concurrency one
(`--sweep 1`), the loopback proxy URL, and writes `--csv
{out}/requests.csv`, `--html {out}/report.html`, `--ndjson
{out}/events.ndjson`. `{out}` is replaced by the new phase directory. Explicit
paths to workload/configuration files must appear in `inputs`. Freeze every
command argument, generation option, tokenizer/template, seed and request order.
Do not freeze credentials. The child environment excludes inherited API keys.
Planning-duration estimates do not supply execution caps: freeze an explicit
reviewed cap for each run. Only BC-KV-002 has the fixed 7200-second ceiling;
other cases do not inherit that authorization.

Every `expected_requests` entry contains `messages_sha256` (SHA256 of UTF-8
JSON messages with sorted keys, compact separators and unescaped Unicode),
`options` (every other JSON request field), `warmup` (boolean), `session_id`
(string or null), and `turn` (source index or null). Freeze all warmup and
measured requests, including recorded assistant history. The private proxy
captures raw responses and request identities; the evaluator checks exact wire
order and independently reconciles usage with product CSV rows. Warmup stays
outside measured cohorts. All phase commands must drain before returning.

The descriptor chain, observed local image and running container must agree.
Container identity and worker PID/start-time/command-hash sets are compared at
readiness and each phase boundary. Changed workers block the case. Boundary
sampling cannot exclude a process that appears and disappears entirely between
samples; independent event continuity/exclusivity review is still required.
The runner records a random ownership label before creating its container and
removes only that owner. Cleanup failure prevents PASS. A separate external
generation-bound watchdog remains necessary for machine loss, SIGKILL, and
Docker daemon failure: a local finally block cannot prove cleanup in those
conditions. Likewise, source-file hashes establish the reviewed source slice,
not a reproducible-build attestation for every package in the image. Freeze,
artifact provenance, actual-host approval, model-lock completeness, workload
isolation and exporter freshness require independent execution review.

Raw trace and inventory output belongs in private evidence storage. Do not
commit run output. Fixtures must explicitly distinguish source/exporter
serialization from live model execution.
