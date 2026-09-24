<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->
# Metrum AI Bench CLI external docs

Customer-facing documentation for Metrum AI Bench CLI. Docusaurus v3 site with
`content/` as the docs root.

Published URL: https://docs.metrum.ai/metrum-ai-bench-cli/

## Local preview

```bash
cd external-docs
npm install
DOCS_BASE_URL=/ npm run start
```

Usually http://localhost:3000/

Use `DOCS_BASE_URL=/` so local routes are not prefixed. Production defaults to
`/metrum-ai-bench-cli/`.

## Production build

```bash
npm run build
```

This bakes `baseUrl` `/metrum-ai-bench-cli/` (override with `DOCS_BASE_URL` if
needed). Serve the `build/` output so the site is reachable at
`https://docs.metrum.ai/metrum-ai-bench-cli/`.

### Multi-version publish

Versioning is env-driven (not Docusaurus `versioned_docs/`):

| Variable | Default | Role |
| --- | --- | --- |
| `DOCS_VERSION` | `v` + root `Cargo.toml` `version` | Stamp shown in the navbar selector |
| `DOCS_BASE_URL` | `/metrum-ai-bench-cli/` | Site `baseUrl` for this build |
| `DOCS_VERSIONS_URL` | `{DOCS_BASE_URL}versions.json` | Manifest fetched by `src/theme/Root.tsx` |

Each versioned tree is published under a distinct `DOCS_BASE_URL` (for example
`/metrum-ai-bench-cli/v1.0.0/`), with a `versions.json` manifest hosted
alongside them so the navbar selector can switch trees.

This is automated end to end by `Makefile` (`make build`, `make build-latest`,
`make package`) and the `docs-bundle` job in `.github/workflows/release.yml`:

- Every GitHub Release carries the docs bundle as release assets
  (`metrum-ai-bench-cli-docs-<tag>.tar.gz`, `.sha256`, and
  `metrum-ai-bench-cli-docs-versions.json`).
- The docs web host pulls the newest Release bundle and publishes it to
  `docs.metrum.ai`. A final tag (no `-rc.` / `-alpha.` / `-beta.`) promotes
  `/latest/`; a prerelease tag only gets its own versioned path.
- `.github/workflows/deploy-docs.yml` additionally archives each build to
  Restic (a push to `main` as a dev preview, a tag as a release). It does not
  deploy anything.

The `versions.json` manifest always lists every tagged version, not only the
one just published, so earlier releases stay visible in the version picker.
See `scripts/package-docs.py` and `docs/RELEASING.md`.

## Brand

Theme tokens match the Metrum AI public site brand system. Navbar logos:

- light: `static/img/metrum_ai_bench_black.svg`
- dark: `static/img/metrum_ai_bench_white.svg`

Mono variants are also under `static/img/` for other surfaces.
