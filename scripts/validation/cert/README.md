<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Certification evidence and regular CPU checks

Metrum AI maintains the case catalog in
[`docs/RELEASE_CERTIFICATION_CATALOG.csv`](../../../docs/RELEASE_CERTIFICATION_CATALOG.csv).
GPU runners are in development; only specific cases have been exercised, and
unimplemented cases remain fail-closed. Development results are not release
certification. These CPU tools do not certify GPU tests.

Run the offline tests and tracked-documentation archive dry-run:

```sh
python3 -m unittest discover -s scripts/validation/cert -p 'test*.py' -v
python3 scripts/validation/cert/check_package_docs.py --dry-run
```

The dry-run exercises the tracked docs packaging and links for each target
name; it does not compile platform binaries or run a release container. The
weekly job additionally checks the actual published archives against their tag.
`check_live_docs.py` crawls only links within the published documentation tree.

## Execution-time records and freeze checks

The complete selected-set gate is a **loopback offline rehearsal (no netns isolation)**:
it must use each selected executable case's frozen launch path and candidate
binaries, loopback stand-ins, record writers and validator intake. Approved N/As
remain N/As, not simulated executions. The partial probes below are not that gate. They provide no OS-enforced network isolation and no GPU or
physical-tier evidence.

For catalog r4 image-backed lanes, the freezer pins the tracked OCI index and
linux/amd64 child descriptor bytes into each execution manifest. Offline
preflight rechecks both digests, platform selection and the child config digest.
Before serving, the runner checks the local image configuration against the
child descriptor; after starting each r4 container it checks the container
image identity and records index, child, platform and config digests. Missing,
changed or ambiguous descriptor/runtime evidence fails closed. These checks do
not establish physical cache-tier activity or replace the integrity audit.

The harness requires Python 3.12 or newer. The shared result writer atomically
publishes a case record, evidence SHA256 hashes, candidate identity, typed
infrastructure history and conservative carry-over eligibility. The shared lane
entry exports frozen manifest and binary locations so their hashes are captured
at write time, including every nested input pin for the selected lane.
Interrupted cases retain null unknown request/error counts. Missing or changed
inputs prevent PASS; failed attempts retain actual/null hashes with explicit
`input_integrity_errors` rather than losing their failure record.
`execution_evidence.py RUN_DIRECTORY` checks retained output hashes and derives
JSON report/sheet rows; it never constructs a PASS for a missing result. Full
catalog acceptance still requires `validate_results.py`.

Append phase observations at the actual event, never reconstruct them later:

```sh
python3 scripts/validation/cert/execution_evidence.py RUN_DIRECTORY --case BC-GPU-001 --phase inputs-ready
```

Supported append-only events are `rent`, `ssh-ready`, `inputs-ready`,
`server-ready`, `first-request`, `last-request`, and `record-validated`.
Provisioning events belong to the external operator. Shared lane lifecycle code
records input/server readiness. At case-record finalization the writer derives
first/last request timestamps from actual request `started_at`/`completed_at`
fields or proxy send/complete observations (the proxy retains UTC alongside its
monotonic clock). It appends those observed UTC values, not finalization time;
missing clocks remain null. Per-case launch validation appends record validation.
Never substitute runner start/end for actual first/last request times.

For VLM cases whose runner requires absolute image paths, prepare a new pinned
prompt file before freezing, using `prepare_vlm_input.py --source SOURCE
--source-sha256 SHA --image IMAGE --image-sha256 SHA --root REPOSITORY_ROOT
--out NEW_JSONL`. It rejects remote, mismatched, multiple and ambiguous images,
never overwrites a file, and prints the new path/hash for the lane manifest.
Freeze the original source and image as inputs too; the prepared JSONL is an
ordinary frozen input consumed unchanged by the runner. This is input preparation,
not an offline launch rehearsal or certification evidence.

For product `--require-sut` consumers, `prepare_product_sut.py --inventory
OBSERVED.json --inventory-sha256 SHA --lane-manifest LANE.json --lane-sha256 SHA
--lane L1 --out NEW_SUT.json` produces a separate product-schema declaration from
exactly one observed GPU inventory and the pinned lane. Its fields distinguish
observed inventory from declared server/model; fractional GiB is rounded down.
Pin the original inventory and lane, and pin the generated declaration as the
lane SUT. Do not use synthetic inventory as live evidence. Input preparation
alone does not prove that the product parser accepts the result. The narrow
`rehearse_product_llm.py --binary FROZEN_LLM --binary-sha256 SHA --sut NEW_SUT.json
--sut-sha256 SHA` loopback probe checks the actual pinned LLM binary's
`--require-sut` parser and resulting summary against the SUT identity; it cannot
stand in for parsing other modalities or exercising the case runner. It uses
only temporary outputs and an isolated disposable local server.

`rehearse_live_modalities.py --manifest FROZEN_CASE.json --case BC-LIVE-001`
(or BC-LIVE-002/003/004) is a partial CPU-only loopback check: it verifies the
frozen case entry argv and pinned harness, uses the same smoke/headline command
assembly as the actual runner, invokes the frozen modality binary through
`cargo --locked xtask`, and verifies the required-SUT summary and local request
cohort. For L1 it checks xtask's actual selector argv against the frozen recipe
and compares its output with an independent double generation. Since this uses
temporary output paths, it does not call the production case-output boundary;
the loopback requests precede the independent check and are never evidence.
It does not call a record writer or exercise non-live cases, physical GPU behavior
or the native offload gate, and is **not** the complete selected-set rehearsal.
Its disposable loopback server is not an OS network sandbox.

`rehearse_g1_variants.py --manifest FROZEN_CASE.json` is a partial GPU-003
check: it validates the frozen G1 inputs and actual runner's tool prompt, then
runs all six real product/proxy command variants against a synthetic loopback
server using disposable output. It checks each product request and forwarded
variant fields, but does not run the GPU-003 validator or write a case record.
Its loopback fixture is not an OS network sandbox, real-model evidence, or the
complete selected-set rehearsal.

`rehearse_g5_strategic.py --manifest FROZEN_CASE.json` exercises the GPU-013
strategic benchmark and intentional export-failure argv with the pinned product
binary, real trace proxy and synthetic loopback responses. It does not kill or
restart a server, invoke the validator or write a case record; it is not real
server evidence or the complete selected-set rehearsal. Its fixture is not an OS
network sandbox.

`rehearse_g5_endpoints.py --manifest FROZEN_CASE.json` exercises the GPU-015
four-port configuration check and actual endpoint-file/product argv against two
synthetic local servers with two trace proxies. It verifies successful measured
requests reach both endpoints, but does not kill a server, invoke the validator
or write a case record. Temporary loopback ports replace the frozen ports after
checking them; it is not an OS network sandbox, real failover evidence or the
complete selected-set rehearsal.

`rehearse_vlm_reasoning.py --manifest FROZEN_CASE.json` exercises the actual
REASON-005 VLM product/proxy argv against a synthetic local server, verifies
seven completed product requests and compares the wire-embedded image bytes
with the frozen image pin. It checks the actual runner's input validation but
does not invoke its result validator or write a case record. It is not a network
sandbox, real reasoning evidence or the complete selected-set rehearsal.

`rehearse_g4_modality.py --manifest FROZEN_CASE.json` exercises GPU-011's
actual LLM, VLM and two strategic product/proxy command cohorts against a
synthetic local server, checks model/options and pinned VLM image bytes on the
wire, and uses temporary outputs. It does not stop or launch model servers,
invoke the GPU-011 validator or write a case record. It is not a network sandbox,
real-model evidence or the complete selected-set rehearsal.

`rehearse_d_default.py --manifest FROZEN_CASE.json` exercises REASON-006's
actual 64-request field-default runner/product/proxy argv on a synthetic local
server, checking the pinned model, thinking option and 512-token cap on the
wire. It does not invoke the truncation validator or write a case record; it is
not a network sandbox, real-model evidence or the complete selected-set rehearsal.

`rehearse_g6_schema.py --manifest FROZEN_CASE.json` runs GPU-016's actual
locked/offline schema fixtures plus its LLM and strategic product argv against a
synthetic server. It fails when the executing host's Cargo cache is incomplete.
It does not invoke the live schema result validator, write a case record, provide
an OS network sandbox or replace the complete selected-set rehearsal.

For BC-COMP-002, `prepare_reference_inputs.py --source SOURCE_JSONL
--source-sha256 SHA --out-dir NEW_DIRECTORY` constructs the reference prompt,
ShareGPT and order inputs from exactly 64 distinct source prompts, validates
them with the real runner's input checks and emits a pinned receipt. The order
rows include the product parser's required concurrency-one `cohort`; regenerate
any earlier order file that lacks it before freezing. Freeze the source, receipt
and all three outputs; this preparation does not run the case.
`rehearse_comp_candidate.py --manifest FROZEN_CASE.json --tokenizer-cache CACHE`
checks the frozen tokenizer snapshot and executes the actual candidate-side argv
and prompt order against synthetic loopback. It assembles the reference command
but does **not** execute the vLLM reference parser or container, invoke the
comparison validator, or write a case record; it is not a complete rehearsal or
an OS network sandbox.

Selected-set preflight recomputes the production selection policy before case
preflight; rehashing an edited selected list is insufficient. This does not
independently attest the supplied change and history inputs, which must still be
verified against the candidate and release records.

Before going offline for BC-GPU-016, run `scripts/validation/cert/prepare_schema_cache.sh`
with the intended Cargo home and toolchain. It runs `cargo fetch --locked` followed
by an offline no-run build of the exact schema-history/migration test targets;
this preparatory step is not itself evidence that the measured case ran. The
operator must arrange and verify that cache on the actual execution host.

Freeze now also calls the actual runner's input checks for selected known cases
(GPU-003/011/013/015, REASON-005, COMP-002). These are early checks only, not
complete runner argv or SUT parsing and **not** the complete offline rehearsal
required before rental; missing selected-case coverage remains a blocker.

`case_launcher.py --manifest FROZEN.json --preflight` checks Python, the frozen
candidate executable, SUT system/GPU memory and `host_os`, tokenizer revision
and content pin, all-smi executable pin, every input pin and each launcher pin.
Tokenizer validation probes the separately pinned **validation-only build**
against a one-request loopback CPU fixture with no server usage, never the
measured candidate. Both builds must be release builds of the same frozen
commit: shipped measured features and validation-only features are both
`otlp,tokenizer`, with distinct roles and binaries. For BC-GPU-010 preflight
also probes the exact shipped binary. The probe inherits no API/exporter credentials and never
becomes GPU case evidence or a request-phase measurement. Temporary files and
the fixture server are cleaned up on success or failure. Preflight allocates
no hardware or model-serving process.

Each `freeze_case.py` invocation also requires `--tool-manifest FILE`: a JSON
object mapping command names to `{ "path": ABSOLUTE_CANONICAL_PATH, "sha256":
LOWERCASE_SHA256 }`. Pin the actual execution-host tools, including the
required interpreter and lane tools. Tool pins must be native Linux ELF files,
not shell-script shims; synthetic test manifests are not evidence.
The frozen manifest binds both the tool manifest and each executable. The
launcher creates a private pinned-tool PATH and uses the pinned `env -i` with
an allowlist, then rechecks tool hashes and links after the child exits. Record
provenance includes the absolute tool paths and hashes. Before accepting a case
PASS, the launcher also requires its record to match the exact pre-launch
execution-manifest digest and tool map, and rejects manifest changes during
execution. This local check does not authenticate records supplied directly to
the final selected-set validator. This profile deliberately
does not forward `HF_TOKEN`; stage needed inputs before execution. It is not an
OS sandbox and does not attest dynamic libraries, Cargo caches or configuration,
repository closure, host trust, or isolation from a malicious same-user process.
Those remain part of the pre-qualification integrity audit.

The manifest binds `candidate_sha`, role-tagged `candidate` and
`tokenizer_validation` artifacts, the execution-time `build_receipt`, and the
frozen `lane_manifest`. Build and log hashes are rechecked before launch.
Secondary CLI pins must belong to the measured artifact registry; a validation
binary (including renamed copies) cannot appear in measured argv/config.
All four BC-GPU-010 cells use the same workflow-built shipped LLM binary with
`otlp,tokenizer`. Its receipt, executable and workflow-origin evidence must be
frozen as inputs; a separate variant is forbidden. L1–L4 also require xtask's
canonical `BENCH_BIN_DIR` binaries to resolve to the registered measured paths;
ambient search fallback is not acceptable. After a case exits, the launcher
rechecks the build, lane and frozen input pins before validating a record. A
lane-error marker rejects the case in `report_rows` even if its runner already
wrote PASS. These checks do not replace immutable execution environments or the
pre-qualification integrity audit. Missing bindings fail closed
before any cell issues a request. Validation-only executables or their copies
cannot be substituted for the shipped binary.
Every cell must pass unchanged criteria; no waiver, N/A or skipped fallback is
accepted. Each cell writes a build proof before invocation and an exit outcome
afterward. The launcher replays the original checks from raw data/trace, verifies
command/build/proof identity and rejects partial or changed results.
Other fields include `sut`, `all_smi`, `tokenizer` pins
(`path`, `sha256`, plus tokenizer `revision`); an optional `inputs` list of pins;
and `cases`, mapping selected IDs to a pinned `launcher`, a fresh `out`
directory and literal `command` argv containing that exact `--out` value.
Each launcher must execute only its named case and retain its own attempt
record. The launcher rejects reused attempt directories. On failure or an
interrupt it stops its owned process group and preserves a typed FAIL with null
unknown counts if the case did not publish a record. Existing records are never
overwritten: a separate failure sentinel prevents an earlier PASS from masking
a launcher/cleanup failure. Once a non-W case may have recorded its startup
`evidence/server.log`, cleanup preserves that file and writes the final container
snapshot to `evidence/server-teardown.log`, with a separate
`evidence/server-teardown.sha256`. The checksum is an independent sidecar, not
an output hash in a case record already written before cleanup; retain both for
review. W has no result until external sign-off, which binds its final
`server.log` after cleanup. A sibling `OUT-lifecycle` directory retains append-only
input-ready and record-validated observations. External provisioning events must
still be supplied by the operator at their actual source. Launch one case with `--case ID`, or omit `--case` for the whole selected
set. Pins are rechecked before each launch; no shell evaluation is performed.

Before candidate freeze, run `bash scripts/validation/cert/before_freeze.sh SHA`
in a clean candidate checkout. This combines REL-001 prerelease-classifier tests,
the repository documentation audit and CLI/schema/export-surface tests. It is a
pre-freeze gate, not a substitute for final pre-tag certification. Existing
performance gate configurations are unchanged.

## Generated inputs: structural acceptance is not final preflight

`freeze_case.py --generated-recipes FILE` accepts a JSON object containing a
nonempty `recipes` list. Each recipe freezes `id`, `kind`, `generator`,
`generator_version`, `seed`, `config`, `dataset`, `tokenizer` and `template`.
The supported kind is `offline-prompt-selection-v1`: the exact receipt-verified
shipped CPU prompts CLI, explicit local JSONL dataset, fixed count and profile
(or both ISL/OSL targets), optional count slack/repeat/work/tolerance parameters.
The tokenizer pin includes its model revision. Tokenizer/template pins describe
the source workload provenance; the selector consumes existing dataset metadata,
not a new tokenizer/template implementation. Arbitrary commands, URLs, moving
Hub lookups, unknown parameters and invented future output hashes are rejected.

A lane prompt slot may then be exactly `{"generated_input": "RECIPE_ID"}`.
Offline preflight validates known structural pins and returns
`STRUCTURAL-PREFLIGHT-OK`, never final `PREFLIGHT-OK`. Default runtime/server
manifest loading still rejects symbolic slots. Model/server/baseline constraints
remain checked; only unavailable generated bytes are deferred. Fixed performance
prompt hashes and tracked modality fixtures are not waived.

Before launching a measured case, the direct launcher (also used by selected-set
execution) generates twice in fresh owned CPU process groups with no ambient
credentials/exporters/proxy. It preserves both outputs and logs, rejects contract
or seed drift and byte mismatches, then binds the actual input hashes. It creates
new runtime lane/case manifests, preserving the structural parent and hash,
revalidates every original concrete-input/domain criterion, and only then starts
the measured launcher. Reused attempts are rejected. A mismatch starts no
measured child and preserves a typed failure with unknown counts left null.
Runtime input/provenance pins and the `versions.generated_inputs` hash ledger
bind the actual and re-derived bytes, recipe, binding receipt and binding time.
Binding time is not a measured request clock. CPU/synthetic unit fixtures are
not real preflight or certification evidence.

L1 also selects a dataset inside `cargo xtask local-smoke llm`. Its case manifest
freezes an additional `derived_inputs` contract, including the lane's pinned
`template`, tokenizer and shipped prompts CLI. This preserves the existing
16-row selection, `chat-short` profile and selector seed **7** (distinct from the
workload seed 42). With `CERT_INPUT_MANIFEST` set, local-smoke calls the fixed
certification boundary helper after selection but before the measured CLI.
The original certification selector also requires explicit local JSONL and uses
the same offline allowlist environment as independent re-derivation, without
ambient credentials, proxies or exporter settings.
The helper independently re-derives the selected bytes and rejects missing
provenance, changed selector arguments or output mismatches before workload
issuance. Ordinary non-certification local-smoke behavior is unchanged. Original
selection, commands, both re-derivations and their receipt are retained; records
bind them through `versions.derived_inputs`. Every original L1 threshold remains
in force. Frozen harness inputs include xtask Rust sources, its external
`count_points` module, Cargo manifests/locks, Cargo alias configuration and any
checked-in toolchain files, alongside certification Python/shell/schema sources.
Source drift is rejected before workload issuance. Orphan runtime bindings are
rejected by preflight, launch and PASS reporting; replay requires an actual
integer seed (not a boolean or numerically equal float). Typed infrastructure
FAIL records retain unknown counts and integrity errors.

## Freeze and launch individual selected cases

After the actual candidate freeze and matching source-version update, dispatch
`pretag-certification.yml` on that frozen public ref with candidate_sha and
version inputs. Do not create a release/tag or publish. After the complete
workflow succeeds, run `freeze_builds.py --repo CLEAN_CHECKOUT --candidate-sha SHA
--version VERSION --workflow-run-id RUN_ID --out NEW_DIR`. It requires the
workflow run's head to match the frozen source, successful workflow_dispatch
status and the exact pretag workflow/repository/artifact. It uses
`gh run download` to fetch `pretag-release-x86_64-unknown-linux-gnu`, verifies the
workflow-recorded archive checksum and safely copies measured Cargo binaries.
Local measured compilation is not an option. The helper alone uses native
`cargo build --locked --release --features otlp,tokenizer` from the same source.
BC-GPU-010 uses the shipped LLM artifact for all four cells; no separate
feature-variant artifact or local measured build is used.

It emits `builds.json` only after artifact verification and validation build
succeed, binding run/artifact API responses, download log, archive/checksum,
per-binary hashes, source/features/origin and compiler log. Partial downloads
and logs remain on failure. Freeze/preflight/direct launch revalidate those
pins and each measured binary against the verified archive. Synthetic API and
compiler fixtures in unit tests are not actual workflow or case evidence.

`freeze_case.py --build-receipt NEW_DIR/builds.json` creates an exclusive
(never overwritten) single-case manifest from that receipt, selector output,
catalog, validated lane manifest, candidate/baseline binaries, SUT, tokenizer
pin and all-smi pin. It verifies the existing lane
contract offline, freezes every nested lane input pin and all transitive harness
source files, then runs the per-case offline preflight. Additional data files
can be declared with repeated `--input` arguments. Use `--help` for required
paths; missing input files fail before any execution.

The generated command uses `case_entry.sh`, which selects exactly one named
case in its existing lane. Shared shell dispatch suppresses sibling runners;
R, G3 and P1 support selected-only records. BC-HW-001 still executes all three
required reasoning prerequisites on its named class and cannot pass just because
an unrelated reasoning case did. Their prerequisite observations remain evidence,
not extra case PASS records. P1 and R reject an unknown `CERT_CASE_ID` before
argument/input processing or workloads; an unset variable retains bulk mode.
G3's exit status requires a nonempty selected case set with every case boolean
exactly true; diagnostic cell dictionaries do not count as case verdicts.
No performance gate thresholds are changed.

`selected_set.py --selection SELECTION --catalog CATALOG --index INDEX --preflight`
is the selected RC-GPU preflight entry, not an all-tier dispatcher. Omit
`--preflight` only for an authorized execution. The index contains `candidate_sha`,
`selection_sha256`, `catalog_sha256`, and ordered `manifests` entries (`path`,
`sha256`). Every executable case manifest must have exactly one case. Optional
`approved_na` entries have a `record` pin, absolute `evidence_root`, and an
`evidence` map of relative evidence paths to SHA-256. Each N/A must be selected,
match the candidate and case, have independent unexpired approval scoped to that
case, and retain all pinned evidence. N/As are reported as N/A, never launched;
independent review must authenticate the approval. Executable manifests plus
approved N/As must exactly cover selected RC-GPU IDs without duplicates or extras.
All manifests are verified before any launch, and N/A pins are rechecked at each
execution boundary. The order in the index is the execution order, so
dependencies must precede their barriers.

The freezer and launcher compare the certification source tree, xtask sources
and declared build configuration to the candidate commit and pin their bytes;
they reject changed, missing, symlinked or untracked code in that scope. Final
fresh-PASS intake independently checks the validator checkout against the same
candidate and compares the manifest's relative source names and hashes. This
is not a trusted-host or complete execution-closure attestation: Python startup
and bytecode, Git and its dependencies, Cargo caches/configuration, third-party
libraries, dynamic linkers and validator startup still require independent audit.
A matching Git tree alone does not authenticate the selected candidate.

A future Stage-0 smoke must use a dedicated root with a regular
`.stage0-smoke-root.json` marker containing exactly `classification` set to
`Stage-0 smoke, not certification` and `expectations_sha256` set to the reviewed
expectations digest. The normal writer labels records beneath that root; the
certification validator rejects labeled records. This is a defense against
accidental mixing, not an authenticated boundary against record rewriting. No
smoke controller or approved expectations are supplied by this marker alone:
never submit smoke records to certification intake, history or the sheet.

### Remaining offline launch rehearsal work (#469)

The complete-set preflight and individual synthetic loopbacks are **not** a
complete offline launch rehearsal. Before rental, every selected case, including
the live-validation case, needs fail-closed dispatch exercising its actual runner
input assembly and relevant argument/SUT parsers against frozen manifests.
Unknown or missing handlers must fail. L1's production derived-input/output
boundary, COMP-002's reference parser, the remaining selected cases, frozen
input-plan wiring and execution-host schema-cache provisioning are outstanding.
The native CPU-offload validation plan and integrity audit are separate required
work. Loopbacks are not physical-tier evidence or an OS network sandbox.

BC-COMP-001, BC-OPS-001 and externally signed BC-INTEG-001 require an explicit
pinned command via `freeze_case.py --command-json`: they are respectively a
post-measurement barrier, external resource cleanup and externally reviewed
integration evidence, not independent model-load commands. Their required source
results/sign-off must exist before freezing those case inputs. Do not invent
future hashes, bypass sign-off, or interpret collection-only output as PASS.

## GPU development runners

L1, R and G6 have been exercised on a real NVIDIA GPU. Selected cases in D,
G2, G3 and G5 have also been exercised; other cases failed, require different
hardware, or are unimplemented. G1 and G7 remain development work. These
results do not establish a release certification. G7 keeps the 5% gate unless
a per-point noise-floor file is SHA256-pinned with a `recorded_utc` before the
run and an independently reviewable `evidence_link`; the retained copy and
all frozen inputs are rechecked after collection.

Run only on an authorized GPU. Freeze the candidate and previous-release binary
hashes, pinned server image digest, model revision and prompt hashes in a
manifest conforming to `manifest.schema.json`; `manifest_tool.py` enforces
additional lane-specific constraints. Set `CUDA_VISIBLE_DEVICES` to one GPU,
and use an exclusive server and a fresh output directory for each attempt:

```sh
export CUDA_VISIBLE_DEVICES=0
bash scripts/validation/cert/lane-l1.sh --manifest /path/frozen.json \
  --candidate /path/candidate/metrum-ai-bench-cli-llm \
  --baseline /path/previous/metrum-ai-bench-cli-llm --out /private/attempt
```

`--plan` prints a lane's assigned cases without running a server. The D
field-reported configuration and the reference comparison use separate
invocations (`lane-d.sh` and `lane-comp002.sh`); the former records only
BC-REASON-006, so BC-COMP-002 remains missing until its own invocation. GPU 003, 010, 011, 013, the r3 runner for 015, REASON-005, and LIVE-002–004
have CPU-only development coverage, not GPU verification. L2–L4 require a
clean checkout matching the candidate SHA, a pinned modality binary and SUT,
fixture hashes, and a prepared offline hub cache. They launch an owned,
loopback-only container and fail closed on cleanup. V pins a thinking-capable VLM,
compatible parser, VLM binary, image and prompt. G1 pins a tool-use prompt set and requires the
Hermes tool parser and automatic tool choice alongside the qwen3 reasoning
parser. Its reordered/batched tests retain real source and forwarded frames;
missing source populations fail. G3 uses the hidden certification-only
`--cert-e2e-median-slo` option to calculate an exploratory answered-cohort e2e
median for BC-GPU-009. It requires at least two distinct answered latencies and
zero scheduling delay, and is not a prospective service SLO. G4 requires a pinned tokenizer.json and a
separate validation-only tokenizer tool. Case 010 additionally requires the
shipped workflow artifact and per-cell build bindings described above;
all four cells retain their original assertions and thresholds. Case 011 also pins the VLM binary,
strategic binary, thinking-capable VLM image/model/revision, prompt file and
local image. G5 pins the strategic binary, distinct
`second_port` and `second_proxy_port`, and `--gpu-memory-utilization 0.45` in
its server flags; 015 uses one >=48GB NVIDIA GPU and two servers. Other
unsupported cases cannot pass. Real restart
runners kill only their own serving container. The recorder forwards and
retains real server bytes; configured faults are explicitly recorded. Raw
SSE, prompts and generated output can be sensitive and must be kept private
until reviewed and redacted.

For v1.8.0 P1/P2, the approved previous release is the SHA256-verified v1.7.2
release asset, not the historical v1.5.4 study-build entry in the unchanged
perf spec. The prospective P1/P2 runners require the release archive pin in
`baseline_asset`, the exact spec model/server/input pins, and a prepopulated
`CERT_HF_HUB_CACHE`. P1 additionally requires pinned `all_smi_binary`,
`all_smi_release_asset`, `all_smi_published_checksum` (each `{path, sha256}`)
and the pinned telemetry preset. The binary must match the official
`metrum-ai/all-smi` fork release `v0.28.1-metrum.3` asset, verified against its
published `.sha256` and the harness table pin. Legacy checkout/protoc fields
are ignored. A wider pre-recorded noise
floor must be pinned with `recorded_utc` and `evidence_link` and independently
reviewed; the default is 5%. Measured prompt sequences and independent legacy
cohorts are checked for all six points per model. Candidate native first-any
TTFT and throughput are informational alongside legacy-cohort gate values.
The lanes publish performance results only after owned-container cleanup is
verified. These runners are prospective and have not been GPU-verified.

After both lanes finish, the separate comparability barrier rechecks their
retained evidence under the same frozen manifest and candidate. Its output and
the underlying bundles are private:

```sh
python3 scripts/validation/cert/perf_comp001_barrier.py \
  --p1-out P1_OUTPUT --p2-out P2_OUTPUT --manifest MANIFEST.json \
  --out NEW_OUTPUT --owner OWNER --evidence-link PRIVATE_EVIDENCE_LINK
```

This requires twelve passing legacy points and retains the candidate's native
headlines separately. CPU fixtures do not establish GPU results.

To prepare the P1 exporter offline on the target architecture, download the
release asset and its published checksum first. Choose `all-smi-linux-x86_64.tar.gz`
for GNU libc, `all-smi-linux-x86_64-musl.tar.gz` for musl, or
`all-smi-linux-aarch64.tar.gz` for aarch64 GNU libc. ELF architecture must match
both the selected asset and the host. For example:

```sh
gh release download v0.28.1-metrum.3 -R metrum-ai/all-smi \
  -p 'all-smi-linux-x86_64.tar.gz' -p 'all-smi-linux-x86_64.tar.gz.sha256'
python3 scripts/validation/cert/perf_all_smi_prepare.py \
  --asset all-smi-linux-x86_64.tar.gz --checksum all-smi-linux-x86_64.tar.gz.sha256 \
  --output NEW_ALL_SMI_BINARY --evidence NEW_PROVENANCE_DIR
```

The helper makes no network calls, verifies the checksum and asset, and stages
an executable without overwriting an existing file. Copy its printed manifest
pins and retain `all-smi-provenance.json`. The P1 lane independently verifies
these inputs again; no source rebuild or byte-equality rebuild is required.
CPU fixture results are not NVIDIA or release-certification evidence.

### W: external platform wrapper and manual sign-off

W is a two-step runner. The Metrum Bench platform team owns the wrapper run and
its **external, manual** sign-off; a missing sign-off cannot pass the case.
Freeze a real wrapper executable's path, SHA256 and version as `lanes.W.wrapper`.
The manifest also supplies `wrapper_args`, `strict_args`, and `safety_args` as
argument arrays (never shell text). `wrapper_args` must contain each standalone
placeholder `{candidate}`, `{url}`, `{prompt}`, `{cli_log}`, `{wrapper_log}`,
`{max_tokens}`, `{count}`, and `{model}`; include forwarded `--streaming` and
`--extra-body-json` with `{"chat_template_kwargs":{"enable_thinking":true}}`.
The strict and safety arrays must respectively forward
`--strict-reasoning-truncation` and `--ci-safety`. The team must configure these
arrays for its actual wrapper interface, not a fixture or mock. The wrapper must
run the pinned candidate, emit its unmodified CLI JSONL at `{cli_log}`, and
export a JSON object at `{wrapper_log}` containing `cli_argv`, `cli_exit_code`,
`requests`, and `summary`. Each wrapper request has `seq`, the v1.8.0 fields
`ttft_s`, `ttft_visible_answer_s`, `ttft_source`, `outcome`,
`answer_completed`, `usage_missing`, and a nonempty `warning` for truncation.
The wrapper summary exposes `attempted`, `answered_requests`, `successes`, `errors`,
`usage_missing_count`, and `reasoning_truncated`. These are checked against the
same actual CLI requests and a real proxy trace; strict and CI-safety truncation
must fail the CLI and wrapper. The run uses separate adequate and low-cap cells.

Collect with `lane-w.sh` using the common lane arguments and an offline model
cache. Successful collection is not a PASS; it prints the collection SHA256
and waits for the team. After server cleanup, the platform team reviews the raw
private evidence and supplies a JSON sign-off file with `team` equal to
`Metrum Bench platform`, `approved: true`, `signer`, `signed_utc`,
`evidence_link`, `candidate_sha`, `wrapper_sha256`, `collection_sha256`,
`cleanup_sha256` and `server_log_sha256`. Hash the corresponding retained files;
these values bind the approval to this particular run. Finalize with:

```sh
bash scripts/validation/cert/lane-w.sh --finalize --out W_OUTPUT --signoff PLATFORM_SIGNOFF.json
```

This checks the hashes, cleanup and field contract again before writing the case
record; without the file there is no PASS. The sign-off is external/manual: its
claimed signer and approval must be authenticated by the independent reviewer;
this offline JSON format does not cryptographically establish identity. Raw
traces, wrapper output and sign-off evidence remain private.

For release evidence, generate and freeze the selection described in
[the release policy](../../../docs/RELEASE_CERTIFICATION.md). `CONTRIBUTING.md`
uses the same documentation areas as `README.md`; combined measurement changes
still add their own areas. Unknown paths remain listed as unmapped and select
all extended areas conservatively, rather than silently excluding cases.
The validator
requires all selected PR, RC and RC-GPU rows and produces `results.csv` and
`CERTIFICATION-REPORT.md`, with unselected rows explicitly NOT-SELECTED:

```sh
python3 scripts/validation/cert/validate_results.py \
  --catalog docs/RELEASE_CERTIFICATION_CATALOG.csv \
  --results RESULTS/results --candidate-sha FULL_COMMIT_SHA --release v1.8.0 \
  --selection selection.json --execution-index execution-index.json \
  --execution-index-sha256 INDEPENDENTLY_APPROVED_SHA256
```

Fresh selected PASS records in release mode require a reviewed execution index.
The index binds `candidate_sha`, `selection_sha256`, `catalog_sha256`, a
`selected_plan` pin, and an `entries` array. Each entry contains `ID`, the
original `record_sha256`, and relative `manifest` and `phases` pins (each a
`path`/`sha256` object); runtime-generated manifests additionally require a
`structural_manifest` pin. The selected plan is the frozen case index with
matching candidate, selection and catalog hashes. Copy proof files byte-for-byte
under the results parent, and independently review the final index digest before
passing it. `build_execution_index.py` can build the final index from already
staged, unchanged records and a per-case proof map, and it runs production
intake before writing the index. It does not run cases or independently attest
its inputs. An index cannot substitute for the actual selected-set rehearsal:
the all-tier selected plan, late-bound barriers, generated input re-verification
at intake and full harness-to-candidate closure are not complete. Hashes do not
authenticate a forged bundle or replace host trust and an independent integrity
audit. Approved N/As are checked separately; they do not need a fabricated
execution entry.

For a periodic run, pass `--cadence weekly --run-id ID --run-date-utc ISO_UTC
--branch main --compared-against VERSION`; the CSV defaults to `periodic.csv`.
Results require sanitized owner and evidence link fields, and evidence paths
relative to the parent of `results/`. Supply `--history DIR` containing all
prior attempt JSONs; a later attempt without every prior attempt is rejected.
A missing selected case, FAIL, incomplete record or invalid approval blocks a
pass. Use `--stage full` after publication with both pre-tag and POST evidence.

**Independent human review is still required:** this utility checks structure,
candidate identity and evidence-file presence, but cannot authenticate
approvers, verify that assertions were executed, or discover unprovided
history. Review raw evidence and redact outputs before publication; the utility
does not identify confidential text.
