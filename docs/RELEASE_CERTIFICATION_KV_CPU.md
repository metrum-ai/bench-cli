<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Offline KV data and export checks

`scripts/validation/cert/kv_cpu_runner.py` runs exactly one of BC-KV-010 through
BC-KV-015. Each case writes the existing atomic result record, content-hashed
evidence and lifecycle events. A nonzero subprocess, missing executable, failed
oracle, accepted rejection control or changed input prevents PASS. Fresh release
credit additionally requires the existing reviewed execution index and full
candidate selection. No server or GPU participates.

The runner uses only verified local inputs and offline product APIs. Prepare the
public files before execution: prompt-library `sample.jsonl`, `manifest.json`
and `LICENSE` at revision `0666f62e581b482838ae2e17b333ee36ff3d01b0`, plus
the frozen `W2.jsonl`. Exact SHA-256 pins, small synthetic controls, tokenizer,
template and schema hashes are in `scripts/validation/cert/fixtures/kv_cpu/lock.json`.
The W2 pin binds the existing selected workload; synthetic rendered-overlap
results do not claim to characterize the full prompt-library corpus. No download
or remote code execution occurs during a case. Child processes receive a minimal
environment with offline flags; executable/interpreter pins are verified.

The certification workflow package supplies `metrum-ai-bench-cli-prompts` and
`metrum-ai-bench-cli-cert-kv-cpu`. The latter exposes library-only HF transport,
structural trace and export paths as an offline adapter. It calls the product
libraries directly and does not contain a substitute cache implementation.
Its mock request rows test export correctness only. Package identity is checked
through the existing `pretag_artifact.verify` descriptor and archive verifier.
Locally compiled development probes cannot satisfy that workflow-origin check.
The helper requires the non-default `cert-probes` feature. Only its separate
certification build enables that feature; normal CLI builds keep their existing
features. Public release assets exclude certification targets even if an old
executable remains in the build cache. The published crate excludes the helper
source/target and certification-only additions, preserving its file membership
and eight installable executables. CI checks the actual normalized crate archive.

Each case has an independent Python oracle:

| Case | Checked output | Rejection controls |
| --- | --- | --- |
| 010 | Full verified HF file, exact rows and deterministic ordinal-derived IDs, preserving repeated prompts | Revision/checksum changes, moving revision, remote-code request, altered row mapping |
| 011 | Exact document/question bytes, target order, IDs, count and repeat serialization | Missing document, ambiguous question mapping, duplicate identity, altered output |
| 012 | Ordered trace IDs, exact normalized times, session/parent edges and prefix reconstruction | Missing field, duplicate ID, missing parent and reordered output |
| 013 | Source/derived hashes, manifest fingerprint, rights/license text and processing chain | Unknown rights, missing license hash, conflicting revision and altered output |
| 014 | Frozen WordLevel tokenizer/template, unique rendered bodies, exhaustive pairwise LCP and independent trie ceilings | Tokenizer/template mismatch, duplicate rendered prompts and altered overlap |
| 015 | JSON schema plus common phase IDs/counts, units and embedded evidence in CSV/HTML | Malformed state, missing phase, physical claim and altered JSON/CSV/HTML |

The deterministic first-fit synthetic corpus uses no random selection. Its
identity includes source, tokenizer and template bytes. Logical overlap and
source hashes never establish physical cache residency or observed reuse.

Development validation runs in ordinary offline CI through
`cargo test --locked --features cert-probes --test kv_cpu_certification`. That test supplies compiled
product executables to the Python product tests; no CI network fetch is needed.
Standalone Python discovery also tests the fixture pins and smoke separation.

## Frozen execution

KVC implements the shared lane interface: `freeze(args)`, `preflight(manifest)`
and `main(argv)`. Its preflight invokes `kv_envelope.validate(..., lane='KVC')`
and requires the base tools plus pinned `dirname`; there are no engine tools.
It additionally verifies exact CPU input closure, interpreter, public revision,
workflow-origin prompts/probe executables and declared time bounds.

Prepare a `kv-cpu-execution.v1` inner spec with exactly these fields:

| Fields | Values |
| --- | --- |
| `schema_version`, `case`, `cases` | `kv-cpu-execution.v1`, one CPU case ID, and `{ID: {}}` for the same case |
| `candidate_sha` | Full candidate commit matching the measured workflow artifacts |
| `binary`, `probe` | `{path,sha256}` pins for the workflow's prompts executable and offline probe |
| `workflow_origin` | Pin to the local origin JSON accepted by `pretag_artifact.verify` |
| `tools` | Canonical native executable pins for `bash`, `env`, `python3`, `dirname` |
| `public_revision`, `public_inputs` | Locked revision and canonical directory containing verified public files |
| `inputs` | Pins for every fixture, fixture lock, pinned schema and applicable public file |
| `timeout_seconds`, `cleanup_grace_seconds`, `cgroup_root` | Positive total cap at most 900 seconds, shorter cleanup reserve, and actual canonical delegated cgroup root |
| `owner`, `evidence_link` | Execution-record provenance |

Source the input digests from the fixture lock and independently verify the
files. Do not populate pins from unverified downloads. The probe and prompts
executable must come from the same successful candidate workflow package.

`kv_cpu_freeze.py` accepts `--selection`, `--catalog`, `--candidate` (prompts
executable), `--lane-manifest` (CPU spec), `--build-receipt`, `--tool-manifest`,
`--case`, `--out` (new envelope file) and `--run-out` (new evidence directory).
It verifies the production selection, build receipt and transitive closure, then
writes the shared `kv-r5-frozen.v1` envelope. No server/model/GPU placeholder is
required. Launch that envelope with `case_launcher.py --manifest ENVELOPE`;
`--preflight` checks inputs without execution. The common launcher supplies
the cgroup/process boundary, isolated environment and lifecycle. KVC preserves
its outer-envelope binding when the existing result writer captures provenance.

For a rehearsal, pass `--dry-run` to the runner. It permanently marks the evidence
root and each record with the existing non-certification classification. Both
the marked root and copied records are rejected by certification intake. A
package without the probe produces explicit FAIL records for 010/012/015;
there is no fallback to a development executable. The r4 package can exercise
011/013/014, but its rehearsal records still cannot certify r5.

With cached workflow artifacts and local inputs, allow one minute per case for
execution and input verification, excluding one-time artifact/input preparation.
This is a planning estimate, not a measured certification duration or a live
performance assertion. Each subprocess has a 120-second timeout. Cases retain
their own fresh output directories, including all failures and control logs.
