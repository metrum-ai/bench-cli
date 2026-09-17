# Metrum AI Bench CLI external docs

Customer-facing documentation for Metrum AI Bench CLI. Docusaurus v3 site with
`content/` as the docs root.

## Local preview

```bash
cd external-docs
npm install
npm run start
```

Usually http://localhost:3000/

## Production build

```bash
DOCS_VERSION=v1.0.0-rc.6 npm run build
```

### Multi-version publish

Versioning is env-driven (not Docusaurus `versioned_docs/`):

| Variable | Default | Role |
| --- | --- | --- |
| `DOCS_VERSION` | `v1.0.0-rc.6` | Stamp shown in the navbar selector |
| `DOCS_BASE_URL` | `/` | Site `baseUrl` for this build |
| `DOCS_VERSIONS_URL` | `/metrum-ai-bench/versions.json` | Manifest fetched by `src/theme/Root.tsx` |

Publish each versioned tree under a distinct `DOCS_BASE_URL` (for example
`/metrum-ai-bench/v1.0.0-rc.6/`) and host a `versions.json` listing
`{ version, label, path }` entries so the navbar selector can switch trees.
Until that manifest is deployed, the selector falls back to the current
`DOCS_VERSION` only.

## Brand

Theme tokens match the Metrum AI public site brand system. Navbar logos:

- light: `static/img/metrum_ai_bench_black.svg`
- dark: `static/img/metrum_ai_bench_white.svg`

Mono variants are also under `static/img/` for other surfaces.
