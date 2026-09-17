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

Publish each versioned tree under a distinct `DOCS_BASE_URL` (for example
`/metrum-ai-bench-cli/v1.0.0-rc.6/`) and host a `versions.json` listing
`{ version, label, path }` entries so the navbar selector can switch trees.
Until that manifest is deployed, the selector falls back to the current
`DOCS_VERSION` only.

## Brand

Theme tokens match the Metrum AI public site brand system. Navbar logos:

- light: `static/img/metrum_ai_bench_black.svg`
- dark: `static/img/metrum_ai_bench_white.svg`

Mono variants are also under `static/img/` for other surfaces.
