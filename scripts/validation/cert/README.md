<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Certification evidence and regular CPU checks

Metrum AI maintains the case catalog in
[`docs/RELEASE_CERTIFICATION_CATALOG.csv`](../../../docs/RELEASE_CERTIFICATION_CATALOG.csv).
L1, R, G6 and G7 contain draft real-server runners for GPU-box debugging; they
have not yet been run on a GPU and must not be treated as certified. D, G1 and
G2 have partial runners; their unimplemented cases and the remaining GPU lanes
remain fail-closed prototypes. CPU tools do not certify GPU tests.

Run the offline tests and tracked-documentation archive dry-run:

```sh
python3 -m unittest discover -s scripts/validation/cert -p 'test_cert.py' -v
python3 scripts/validation/cert/check_package_docs.py --dry-run
```

The dry-run exercises the tracked docs packaging and links for each target
name; it does not compile platform binaries or run a release container. The
weekly job additionally checks the actual published archives against their tag.
`check_live_docs.py` crawls only links within the published documentation tree.

## GPU development runners (not yet validated)

Run only on an authorized, user-provided NVIDIA GPU host. Prepare Docker with
NVIDIA support, the exact Qwen/Qwen3-8B model revision and a vLLM image pinned
by digest. Candidate and previous-release executables, candidate prompt helper
and strategic executable must already be built; no source-built benchmark is
substituted. The checkout must be at the candidate commit. Freeze a JSON
manifest using `manifest.schema.json` and inspect `manifest_tool.py` for the
additional per-lane constraints. Its root pins the candidate commit and both
LLM binary hashes; `lanes.L1`, `lanes.R` and `lanes.G6` each specify `image`,
`model`, 40-character `revision`, `port`, `owner`, `evidence_link`, and hashed
`prompt_sets.default`. L1 additionally pins `prompt_binary` beside the candidate
LLM and a matching JSON `sut`; R and G6 specify a distinct `proxy_port` and
`server_flags` containing `--reasoning-parser qwen3` and
`--no-enable-prefix-caching`. G6 also pins `strategic_binary`. G7 instead
requires pinned `prompt_sets.W1` and `.W2`, a distinct proxy port, disabled
prefix caching, and optionally a pre-recorded pinned `noise_floor` JSON. All
pinned file entries are `{ "path": "...", "sha256": "..." }`.

Use a **fresh** absolute output directory per lane and explicitly select one
GPU by index or UUID. For example:

```sh
export CUDA_VISIBLE_DEVICES=0
bash scripts/validation/cert/lane-l1.sh --manifest /path/frozen.json \
  --candidate /path/candidate/metrum-ai-bench-cli-llm \
  --baseline /path/previous/metrum-ai-bench-cli-llm --out /private/l1-attempt-1
# Same arguments, different fresh --out: lane-r.sh, lane-g6.sh and lane-g7.sh.
```

Use `--plan` to see assigned case IDs without Docker or a GPU. A lane writes
results under `OUT/results` and raw evidence under `OUT/evidence`; it creates
and cleans up only its own vLLM container. R's BC-HW-001 requires an L40S.
Failures and interruptions remain FAIL; an unimplemented lane never passes.
Raw SSE traces, prompt data and logs can contain generated content and must be
kept private until reviewed and redacted. The local transparent recorder is
not a mock server: it forwards and records the actual vLLM response bytes.

For release evidence, `validate_results.py` requires all PR, RC and RC-GPU
catalog rows and produces `results.csv` and `CERTIFICATION-REPORT.md`:

```sh
python3 scripts/validation/cert/validate_results.py \
  --catalog docs/RELEASE_CERTIFICATION_CATALOG.csv \
  --results RESULTS/results --candidate-sha FULL_COMMIT_SHA --release v1.8.0
```

For a periodic run, pass `--cadence weekly --run-id ID --run-date-utc ISO_UTC
--branch main --compared-against VERSION`; the CSV defaults to `periodic.csv`.
Results require sanitized owner and evidence link fields, and evidence paths
relative to the parent of `results/`. Supply `--history DIR` containing all
prior attempt JSONs; a later attempt without every prior attempt is rejected.
A missing case, FAIL, incomplete record or invalid approval blocks a pass.

**Independent human review is still required:** this utility checks structure,
candidate identity and evidence-file presence, but cannot authenticate
approvers, verify that assertions were executed, or discover unprovided
history. Review raw evidence and redact outputs before publication; the utility
does not identify confidential text.
