<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Release certification

Metrum AI release certification uses the stable case IDs in
[RELEASE_CERTIFICATION_CATALOG.csv](RELEASE_CERTIFICATION_CATALOG.csv). The
catalog describes both implemented checks and proposed checks: a catalog entry
is not proof that a check ran or passed. The public copy omits deployment
provider, cost and host details. Keep case IDs and criteria stable; review and
version any change before a candidate is evaluated.

## Policy

Before certification, freeze the candidate commit, prior release binary,
case definitions, input hashes, seeds, server and model revisions, and any
pre-existing tolerance exceptions, changed-file list, ordered release history and
selection before execution. Run all selected pre-tag cases against that
candidate. Existing mandatory PR checks remain independent and are not weakened
by certification sampling. Run GPU cases only after separate explicit authorization. A skipped job, green
path-filter stub, missing data, unavailable dependency or unimplemented case
is not a pass. A failure is retained and repeated unchanged; fixing code
creates a new candidate and requires rerunning the required cases. N/A needs a
specific technical justification and independent approval before the case.

### Measured and validation-only builds

Certify the shipped feature profile: for v1.8.1 every measured case uses a
release-equivalent build with `--features otlp` only. Do not enable `tokenizer`
in the shipped artifact or substitute a feature-enabled binary for measurement.
Record the measured binary's SHA-256, features and frozen source commit.

Tokenizer input validation uses a separate **validation-only build** with
`--features otlp,tokenizer` from that same frozen commit. Record its distinct
SHA-256, features and validation-only role in preflight evidence. Keep the two
builds in separate output directories so neither can overwrite the other.
The validation-only binary may run preflight/input checks, never measured cases;
its successful probe is not a measured case result. If a case requires a
capability absent from the shipped profile, retain that conflict explicitly:
do not silently drop assertions, substitute builds or claim a pass.

Compare metrics across versions only after matching token populations, timing
origins, window, inputs, and usage coverage. A v1.8.0 first-any headline is not
equivalent to v1.7.2 visible-answer TTFT. Preserve raw per-request evidence and
numerators, denominators, coverage, exit statuses, tool versions, and the
candidate SHA. Publish only sanitized case outcomes and links; do not publish
credentials, customer data, machine identifiers or private inputs.

Pre-tag certification requires all selected PR, RC and RC-GPU cases to
pass or have prospectively approved N/A, including resource cleanup. Unselected
cases are NOT-SELECTED, never PASS or N/A. POST
cases run only after release publication; a POST failure is an incident, not
a pre-tag pass. The separate wrapper integration case needs wrapper owner
signoff. See [RELEASING.md](RELEASING.md) for the release workflow and gate
responsibilities.

## Frozen selection and qualification

The catalog marks every active case `core` or `extended`. Core includes the A/B
performance gates, live modality smokes, R reasoning lane, field-reported
reasoning usefulness, counter comparison, packaging, release and post-release checks,
cleanup and wrapper integration. Every core case is selected. Extended cases
are sampled deterministically within each Area: default 30%, rounded up with
at least one per Area. The default seed is the candidate SHA. Changed paths
force all extended cases in affected Areas. Docs changes force REL, PACK, POST
and CI; tests alone do not force an Area; workflow and certification harness
changes force CI. Shared runtime changes force all Areas and unmapped paths
conservatively force all Areas. Cases not run in the last K releases (default 3) are forced by
rotation. Missing history is treated conservatively: never-run extended cases
are selected. Results marked N-A or NOT-SELECTED do not count as a run.

Generate and freeze `selection.json` **before execution**, retaining the input
file list, history CSVs and catalog. Supply historical CSVs newest first (one
release per file); absence of history is allowed but selects all previously
unseen cases. `selected_count` and `eligible_count` report coverage; `lane_plan` contains
only selected IDs, packed by parallel group;
launch parallel-phase lanes concurrently with isolated devices, then cleanup,
then POST after publication. Packing is a plan, not execution or evidence.
The selector does not authorize GPU use.

```sh
python3 scripts/validation/cert/select_cases.py \
  --catalog docs/RELEASE_CERTIFICATION_CATALOG.csv --candidate-sha FULL_SHA \
  --changed-files changed-files.txt --history prior-release.csv --out selection.json
python3 scripts/validation/cert/validate_results.py \
  --catalog docs/RELEASE_CERTIFICATION_CATALOG.csv --candidate-sha FULL_SHA \
  --selection selection.json --results RESULTS/results --release VERSION
```

After publication, use `--stage full` with the same selection and all pre-tag
and POST evidence; only then can the full release be certified. The validator
recomputes the selection against the frozen catalog and candidate, but cannot
prove the supplied changes or history are complete or authenticate evidence.
Independent review must verify those inputs, randomness policy, approvals and
raw evidence. Never change a frozen selection to hide a failure.

### Opt-in carry-over of prior candidate evidence

Metrum AI's validator keeps strict candidate matching by default. An explicitly
reviewed `--carry-over-manifest PATH` permits a prior candidate record only when
it is an ancestor of the target and the manifest binds the unmodified record
bytes, case ID, attempt, evidence root, proof files and per-case reviewer
applicability disposition. The manifest is a JSON object with
`target_candidate_sha` and a nonempty `entries` array. Each entry has `ID`,
`attempt`, `source_sha`, `record_sha256`, `evidence_root`, `proofs` (a nonempty
array of `{path, sha256}`), and `disposition` (`applicable: true`, nonempty
`reviewer` and `reason`, and UTC `utc`). Paths are relative to the results
parent and must resolve inside it. Retain original records without changing
their candidate SHA or contents; preserve all historical records and evidence.

CI-001, CI-006, CI-007, PACK-001, PACK-002, REL-001 and REL-002 require target
candidate records and are never accepted as carried records. Current history
rules remain strict: attempts must be consecutive PASS/FAIL/N-A case records;
a lane interruption without a case record cannot be silently invented or
renumbered. This validator checks hash and ancestry, not reviewer identity,
proof semantics, evidence authenticity, or execution. Independent review and
actual sign-off remain mandatory.

## Regular test cadence

The catalog's `Cadence` field lists per-PR, per-release, nightly, weekly,
on-main-change and on-upstream-change selections. `.github/workflows/regular-tests.yml`
runs nightly CPU checks covering the full suite, repeated connection-reuse
checks, offline certification fixtures, historical schemas, archive
documentation comparison, history secret scan, dependency policy and
published-docs links. Weekly CPU checks cover installation and released
assets; the weekly dependency hygiene report remains in `ci.yml`. A failed
nightly opens or updates a single issue labeled `nightly-failure`. On main,
nightly skips code-dependent checks only when the last successful **full** nightly
tested the same commit; unverifiable history runs the full suite. The live docs
link check still runs on unchanged main. Weekly CPU still runs.

The agent-operated GPU regression smoke uses the R, G6 and L1 lanes against
main and the latest certified release when main changes or a new serving-stack
minor is available; the latter also triggers the small-model performance
comparison. There is no monthly full GPU schedule. These are agent-operated tasks,
not GPU jobs or cloud credentials in Actions. The GPU lane runners are implemented
and CPU-tested, but those checks do not establish GPU evidence or release
certification.
