<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->
# Pre-tag certification dry run

The `pretag-certification.yml` workflow is manual-only. Dispatch it on the
workflow revision to test, supplying `candidate_sha` (a full, lowercase
40-character hexadecimal commit SHA) and `version` (SemVer without `v`).
There is no default candidate and no push or pull-request certification trigger.
Missing or malformed inputs fail before any candidate checkout or certification.
All six checkout sites use the supplied exact SHA and assert the resulting HEAD.
The package jobs also verify the crate version against the supplied version.

```sh
gh workflow run pretag-certification.yml --ref <workflow-branch> \
  -f candidate_sha=<full-commit-sha> -f version=<semver>
```

The workflow verifies the candidate package, runs Rust CodeQL and dummy-server
checks, builds release-equivalent archives for four targets, and runs native
archive smoke tests with tracked documentation byte comparison. Outputs are
workflow artifacts only: it does not publish a release, tag or crate.
A green run is evidence for review, not automatic certification or permission
to release. CPU input-validator tests in `scripts/validation/cert/test_pretag_workflow.py`
exercise missing and invalid inputs without running candidate certification.
