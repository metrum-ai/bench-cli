<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Releasing Metrum AI Bench

This document covers deliberate publish gates, pre-release (rc) policy, and
how to verify signed release artifacts.

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
stack (`scripts/live/serve/*.sh`) and fails the cell through
`scripts/live/assert_headline.sh`. It runs on `workflow_dispatch` and on `v*`
tags (tags only when `LIVE_RUNNER_READY` is `true`), on the self-hosted runner label `gpu-h100`
(`deploy/github-runners-bench-cli/MANUAL`, "GPU runner").

A release is blocked until the gate is green for every modality the README
claims. Link the green run in the release PR.

| Variable | Effect |
|----------|--------|
| `LIVE_GATE_REQUIRED` | When `true`, `release.yml` calls the gate as job `live-gate` on the release tag, and `github-release` and `crates-io` need it to succeed. The tag-triggered copy of the gate then skips so it does not run twice. |
| `LIVE_RUNNER_READY` | When `true`, `v*` tag pushes start the gate on their own. Leave unset until a `gpu-h100` runner is registered, so tags do not queue on a label no runner has. |
| `LIVE_HF_HOME` | Hugging Face cache path on the GPU host (default `/srv/hf-cache`). |

`LIVE_GATE_REQUIRED` is off by default because no runner carries `gpu-h100`
yet. With it on and no such runner, the `live-gate` jobs queue until GitHub
times them out and the release never publishes. Turn it on once the GPU runner
is registered and one manual dispatch is green; until then the rule above is
enforced by review, not by the workflow.

## Zero-success smoke cells block a release

Any smoke cell (live gate, `matrix_smoke.sh`, `campaign.sh`, or a manual
smoke cited in release notes) with 0 successful requests is a release
blocker until it is triaged in a GitHub issue. The issue must state the
cause (client bug, fixture, server configuration, or server bug) and either
link the fix or record a decision, signed off in the issue, to ship with the
modality marked unverified in [CLAIMS_LEDGER.md](CLAIMS_LEDGER.md). Recording
the cell as "all-error measurements" in a results document is not triage.
Campaign `matrix-20260915-195537` (0 of 472 ASR transcriptions succeeded) is
the example this rule exists for.

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
