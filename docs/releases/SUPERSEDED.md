<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Superseded GitHub Releases

Status: Operator guidance. Owner: maintainers.

## Summary

GitHub Release tags **`v1.0.1`** and **`v1.1.0`** are **not ancestors of
`main`**. They remain published for historical download links, but they are
**superseded by [`v1.1.2`](https://github.com/metrum-ai/bench-cli/releases/tag/v1.1.2)**.

Do **not** move or delete those tags.

## Why they are superseded

- Their trees are outside the current `main` history (orphan / rewritten
  ancestry relative to today's line).
- Their GitHub Release tarballs only shipped `LICENSE` and `README.md` at the
  archive root. They do **not** include `TRADEMARKS.md` or the policy docs under
  `docs/` that `v1.1.2` archives carry.
- Install and cite **`v1.1.2` or newer**.

## Operator: update GitHub release notes

From a checkout that contains this file, a maintainer with release write
access should run:

```bash
gh release edit v1.0.1 -R metrum-ai/bench-cli \
  --notes-file docs/releases/notes-v1.0.1-superseded.md

gh release edit v1.1.0 -R metrum-ai/bench-cli \
  --notes-file docs/releases/notes-v1.1.0-superseded.md
```

Draft note bodies for those commands live next to this file.
