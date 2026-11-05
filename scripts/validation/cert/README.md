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
results do not establish a release certification.

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
invocations (`lane-d.sh` and `lane-comp002.sh`); the former records the
unexecuted comparison as FAIL, not PASS. GPU 003, 010, 011, 013, the r3 runner for 015, and REASON-005 have CPU-only
development coverage, not GPU verification. V pins a thinking-capable VLM,
compatible parser, VLM binary, image and prompt. G1 pins a tool-use prompt set and requires the
Hermes tool parser and automatic tool choice alongside the qwen3 reasoning
parser. Its reordered/batched tests retain real source and forwarded frames;
missing source populations fail. G4 requires a pinned tokenizer.json and a
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
