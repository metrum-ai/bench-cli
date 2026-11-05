<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Releasing Metrum AI Bench

This document covers deliberate publish gates, pre-release (rc) policy, and
how to verify signed release artifacts. See Metrum AI's
[release certification policy and case catalog](RELEASE_CERTIFICATION.md)
for candidate-wide criteria, evidence and prospective N/A requirements.
Before tagging, include the frozen selection and certification table in the
release PR for every selected pre-tag case on the exact candidate SHA. A missing,
failed or unimplemented selected case blocks the tag; only a prospectively
approved N/A can satisfy a genuinely inapplicable case. Unselected cases are
NOT-SELECTED, not passes. Post-tag checks are tracked separately.

## Repository variables (publish gates)

Publish jobs are disabled unless the matching repository variable is set to
the string `true`:

| Variable | Effect |
|----------|--------|
| `CRATES_IO_PUBLISH` | Allows the `crates-io` job to run `cargo publish` |

Leave unset (or set to anything other than `true`) for rc tags and for
dry-run releases. A configured `CRATES_IO_TOKEN` alone is not enough.

## Release archives

Each signed `metrum-ai-bench-cli-v<version>-<target>.tar.gz` contains the Rust
binaries, `bin/dummy-model-server`, `examples/` (including
`sut.example.json`), and `test-data/` (fixtures such as `llm-hi.jsonl` used by
the README quickstart). The dummy is a static Go binary cross-compiled with
`CGO_ENABLED=0` for the same four targets (Linux and macOS, `x86_64` and
`aarch64`). It is not published to crates.io.

## Live modality gate

Every non-LLM test in CI runs against `dummy-model-server`, which cannot prove
that a real server accepts the media we send. `.github/workflows/live-modality-smoke.yml`
runs one smoke cell per modality (LLM, VLM, ASR, imagegen) on a real serving
stack (`cargo xtask serve`) and fails the cell through
`cargo xtask assert-headline`. It runs on `workflow_dispatch` and on `v*`
tags (tags only when `LIVE_RUNNER_READY` is `true`), on the self-hosted runner label `gpu-h100`
(`deploy/github-runners-bench-cli/MANUAL`, "GPU runner").

A release is blocked until the gate is green for every modality the README
claims. Link the green run in the release PR.

## Standard performance-number validation (required, #333)

A release is **also blocked** until the perf-number A/B gate passes on one
reference GPU class (RTX PRO 6000 Blackwell SE, H100 or H200), alongside
the live modality gate above. `scripts/validation/perf/` is the versioned
harness (Metrum AI):

- `spec.yaml` pins the serving stack (vLLM image digest, checkpoint
  revisions, serve flags), datasets, the point grid (W1/W2 × c1/c8/c32,
  30 s warmup + 180 s issuance) and the gates. CI lints it
  (`lint_spec.py`, `.github/workflows/perf-validation.yml`).
- `runner.sh` runs the grid on one GPU box, interleaving the last release
  (baseline) and the release candidate per point, and writes raw request
  logs, NDJSON and power CSVs. `runner.sh --plan` is a CI-safe dry run
  (no docker, no GPU, no requests) and is exercised in CI.
- `compare.py` emits the per-point PASS/FAIL table (markdown + JSON).
  **The gate is `system_output_tok_s` and `tpot_p50_ms` within 5% (or the
  point's recorded noise floor, if wider) between baseline and candidate;
  TTFT is informational only (#313 cohort caveats). A missing point is a
  FAIL.** `test_compare.py` unit-tests the verdict logic offline in CI.
- `baselines/` records each validated run (one JSON per box/date) so drift
  is reviewable in git.

Before tagging a release, run the grid with the previous release and the
candidate, and link the `compare.py` PASS table and recorded baseline JSON in
the release PR. A docs-only or packaging-only change does not automatically
waive this requirement. Verify resource teardown as part of the run record.

| Variable | Effect |
|----------|--------|
| `LIVE_GATE_REQUIRED` | When `true`, `release.yml` calls the gate as job `live-gate` on the release tag, and `github-release` and `crates-io` need it to succeed. The tag-triggered copy of the gate then skips so it does not run twice. |
| `LIVE_RUNNER_READY` | When `true`, `v*` tag pushes start the gate on their own. Leave unset until a `gpu-h100` runner is registered, so tags do not queue on a label no runner has. |
| `LIVE_HF_HOME` | Hugging Face cache path on the GPU host (default `/srv/hf-cache`). |

The gate runs with a least-privilege secret contract (#291): when
`release.yml` calls the reusable workflow it passes only the Hugging Face
token (`HF_TOKEN`) the gate needs for model weights — never
`secrets: inherit` — and the reusable workflow's only grant is
`permissions: contents: read`. `HF_TOKEN` is optional: it is needed only
for gated weights, the gate runs with public weights and no token today,
and an unset token forwards an empty value the gate treats as anonymous
access. Configure it as a repository secret before switching the gate to
gated models. A future secret needed by the gate must be declared in
`workflow_call.secrets` and passed explicitly by the caller.

The workflow variables control automation, not the release requirement: a
missing runner or skipped workflow never waives the pre-tag live gate. Execute
and record the equivalent real-server cells against the candidate before the
tag. Today the workflow can skip the tag-triggered gate when these variables
are unset, so the release lead must verify the pre-tag evidence independently.

## Zero-success smoke cells block a release

Any smoke cell (live gate, `cargo xtask local-smoke`, campaign skill sweep,
or a manual smoke cited in release notes) with 0 successful requests is a release
blocker. Triage must identify the cause (client bug, fixture, server
configuration, or server bug) and link the fix. A decision to mark the
modality unverified does not turn a failed certification case into a PASS or
N/A. Recording the cell as "all-error measurements" is not triage.
Campaign `matrix-20260915-195537` (0 of 472 ASR transcriptions succeeded) is
the example this rule exists for.

## Update external-docs release notes

Before each release, update `external-docs/content/docs/release-notes.mdx`
with a section matching the root Cargo package version. The external-docs
home page derives its current-release label from the documentation build
version. Run `python3 scripts/check_external_docs_version.py` and build the
site with `make -C external-docs build` to check both content and rendering.

## Data-point counts in the release notes

Every release states how many data points it tracks.
[DATA_POINTS.md](DATA_POINTS.md) is generated from the serde schemas by
`tests/data_points.rs`. Regenerate it with `cargo xtask render-data-points`
after any change to `summary.v5`, `request.v5`, the strategic sweep point,
or the `telemetry.v3` request row. Historical v4 and v3 logs remain readable by
compatible consumers. `cargo test` (CI) and the release `verify`
job (`cargo xtask render-data-points --check`) fail when the committed file is
stale. The `github-release` job builds the release body itself: GitHub's
generated notes, then the output of `cargo xtask release-notes-data-points`.
Both jobs check out the tag, also on `workflow_dispatch`. Telemetry series are selected
by YAML `include` at run time, so no release states a series count.

## Never publish an rc

Tags containing `-rc.` must never be published to crates.io. The `crates-io`
job `if:` requires:

- a `v*` tag ref (or an explicit `workflow_dispatch` with `publish_crate`),
- `vars.CRATES_IO_PUBLISH == 'true'`, and
- the tag / release tag must not contain `-rc.`

Prefer leaving `CRATES_IO_PUBLISH` unset for every `-rc.` tag.

## Docs deploy (docs.metrum.ai)

`release.yml` (job `docs-bundle`) builds `external-docs/` and attaches it to
every GitHub Release as `metrum-ai-bench-cli-docs-<tag>.tar.gz`, its
`.sha256`, and `metrum-ai-bench-cli-docs-versions.json`. The docs web host
pulls the newest Release bundle on its own schedule and publishes it to
`https://docs.metrum.ai/metrum-ai-bench-cli/`. Nothing in this repo deploys to
a specific host. `.github/workflows/deploy-docs.yml` only archives docs builds
to Restic (every push to `main` and every `v*` tag).

Same rule as crates.io above: **an rc must never become the published
latest.** A tag containing `-rc.`, `-alpha.`, or `-beta.` still publishes and
deploys to its own versioned path (so it can be previewed at
`/metrum-ai-bench-cli/<tag>/`), but the workflow does not move the
`/latest/` alias for it. Only a final `vX.Y.Z` tag promotes `/latest/`.

The pull job on the web host and the Caddy route live in
`metrum-internal-infra-admin` (`newsite/scripts/sync-metrum-ai-bench-cli-docs.sh`),
not this repo.

## Build provenance

`actions/attest-build-provenance` uses:

```yaml
continue-on-error: ${{ github.event.repository.private }}
```

While the repository is private, attestation failures are tolerated (feature /
billing may be unavailable). Once the repository is public, attestation is
required and a failure fails the release.

## Verifying Sigstore / cosign signatures

Release archives are signed with keyless Sigstore (`cosign sign-blob`) and a
`.sigstore.json` bundle is attached to the GitHub Release.

Example verify (replace tag and target):

```bash
TAG=v1.0.0
TARGET=x86_64-unknown-linux-gnu
ARCHIVE="metrum-ai-bench-cli-${TAG}-${TARGET}.tar.gz"

gh release download "$TAG" --pattern "${ARCHIVE}*"
cosign verify-blob \
  --bundle "${ARCHIVE}.sigstore.json" \
  --certificate-identity-regexp 'https://github.com/metrum-ai/bench-cli/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  "${ARCHIVE}"
```

When the repository is public, also:

```bash
gh attestation verify "${ARCHIVE}" --repo metrum-ai/bench-cli
```
