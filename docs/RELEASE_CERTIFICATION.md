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
pre-existing tolerance exceptions. Run every PR and RC case against that
candidate, including the local deterministic SSE and fixed-clock cases. Run
GPU cases only after separate explicit authorization. A skipped job, green
path-filter stub, missing data, unavailable dependency or unimplemented case
is not a pass. A failure is retained and repeated unchanged; fixing code
creates a new candidate and requires rerunning the required cases. N/A needs a
specific technical justification and independent approval before the case.

Compare metrics across versions only after matching token populations, timing
origins, window, inputs, and usage coverage. A v1.8.0 first-any headline is not
equivalent to v1.7.2 visible-answer TTFT. Preserve raw per-request evidence and
numerators, denominators, coverage, exit statuses, tool versions, and the
candidate SHA. Publish only sanitized case outcomes and links; do not publish
credentials, customer data, machine identifiers or private inputs.

Pre-tag certification requires all applicable PR, RC and RC-GPU cases to
pass or have prospectively approved N/A, including resource cleanup. POST
cases run only after release publication; a POST failure is an incident, not
a pre-tag pass. The separate wrapper integration case needs wrapper owner
signoff. See [RELEASING.md](RELEASING.md) for the release workflow and gate
responsibilities.
