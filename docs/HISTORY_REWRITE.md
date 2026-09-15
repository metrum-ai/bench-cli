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
