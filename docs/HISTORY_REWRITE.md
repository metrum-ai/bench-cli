<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# History sanitization record

Before the first public release, maintainers:

1. Froze the private repository and retained it as a private archive.
2. Scanned every private ref with gitleaks; no leaks were detected.
3. Removed private platform artifacts and obsolete operator tooling.
4. Exported the approved source tree into a new single-root public history.
5. Re-ran the test, license, coverage, packaging, and full-history secret
   scanning gates against the release candidate.

The public repository has no object, branch, tag, or pull-request ancestry
from the private archive. CI scans the full public history on every change.

## 2026-09-16 — RC5 leak-tag remediation

Findings: tags `v0.1.80`, `v0.1.81`, and `v0.1.82` still contained
`docs/OSS_READINESS_ASSESSMENT.md` (SSH public key material, cleartext example
password, AWS account ID, internal hostname, personal emails). The repository
was briefly public overnight on 2026-09-16, so the contents are treated as
**exposed** regardless of tag deletion.

Actions taken (agent):

1. Created offline bundle
   `/home/cgadgil/src/bench-cli-pre-remediation-20260916T204234Z.bundle`
   (`git bundle create … --all`) before any ref deletion.
2. Deleted GitHub Releases and tags `v0.1.80`–`v0.1.82` (`gh release delete
   --cleanup-tag`). Local tags removed.
3. Verified the path is absent from `main` and from all `v1.0.0-rc.*` tags.
4. Confirmed the blob remains in **ancestor commits** of `main` (introduced in
   the import history and deleted in #44). A `main` history rewrite was **not**
   performed (human call two days before launch). Gitleaks CI allowlists those
   pre-cleanup commits under the new custom rules; the working tree is clean.

Human follow-ups (not closable by agent):

- File a GitHub Support request to purge unreachable objects / cached views
  (Settings → contact support → “remove sensitive data”; cite removed tag
  names).
- Rotate the SSH key and password shown in that file even if labelled
  “example”.
- Check the org audit log for the overnight visibility change and disable
  whatever automation or token caused it.
