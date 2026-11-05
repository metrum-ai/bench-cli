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
candidate built with tokenizer support for 010; 011 also pins the VLM binary,
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
`CERT_HF_HUB_CACHE`. P1 additionally requires the spec-pinned all-smi checkout,
a binary reproduced offline from its committed tree, an architecture-specific
locked protoc crate and the pinned telemetry preset. A wider pre-recorded noise
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

To prepare the P1 exporter offline on the target architecture, first populate the
locked Cargo cache and obtain the architecture-specific `protoc-bin-vendored`
3.2.0 crate matching the source Cargo.lock. Then run:

```sh
python3 scripts/validation/cert/perf_all_smi_prepare.py \
  --checkout PINNED_ALL_SMI_CHECKOUT --protoc-crate PINNED_PROTOC.crate \
  --output NEW_ALL_SMI_BINARY --evidence NEW_BUILD_EVIDENCE_DIR
```

It builds twice from the committed tree with normalized paths and fails if the
bytes differ. Pin the reported binary SHA only after success and retain the
build evidence; the P1 lane independently rebuilds it again. The build and
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
[the release policy](../../../docs/RELEASE_CERTIFICATION.md). The validator
requires all selected PR, RC and RC-GPU rows and produces `results.csv` and
`CERTIFICATION-REPORT.md`, with unselected rows explicitly NOT-SELECTED:

```sh
python3 scripts/validation/cert/validate_results.py \
  --catalog docs/RELEASE_CERTIFICATION_CATALOG.csv \
  --results RESULTS/results --candidate-sha FULL_COMMIT_SHA --release v1.8.0 \
  --selection selection.json
```

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
