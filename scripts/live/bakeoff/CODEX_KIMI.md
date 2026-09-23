<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Codex on Shadeform via Kimi platform

Bake-off sessions must run **on the GPU host**. Drive Codex with **Kimi Open
Platform** (Moonshot), not the default OpenAI account.

## API key (operator host only)

On the operator machine (`gengar`), read `MOONSHOT_API_KEY` from:

`/home/cgadgil/src/router-handoff-demo/config/env.json`

Copy onto the Shadeform host at session start (SSH env or root-only file mode
600). Never commit the key.

## Wire Codex to Kimi

Codex expects the Responses API; Kimi exposes Chat Completions. Use the
documented local proxy (CC Switch or `@codeproxy/cli`) on the GPU host, then
point Codex at localhost. Confirm current model IDs and docs with a web search
before each bake-off:

- https://platform.kimi.ai/docs/guide/codex-kimi
- https://www.kimi.ai/resources/codex-api

Example pattern (verify ports/models at run time):

```bash
export MOONSHOT_API_KEY=...   # from env.json
npx @codeproxy/cli --base-url https://api.moonshot.ai/v1 \
  --model kimi-k2.7-code --apikey "$MOONSHOT_API_KEY"
```

`~/.codex/config.toml` on the GPU host:

```toml
model = "kimi-k2.7-code"
model_provider = "kimi-proxy"

[model_providers.kimi-proxy]
name = "Kimi via local proxy"
base_url = "http://127.0.0.1:8787/v1"
wire_api = "responses"
stream_idle_timeout_ms = 600000
```

## Recording

```bash
asciinema rec -c 'codex' docs/reviews/bakeoff/<date>-<sku>/codex-metrum.cast
# separate session for aiperf path:
asciinema rec -c 'codex' docs/reviews/bakeoff/<date>-<sku>/codex-aiperf.cast
```

Paste `scripts/live/bakeoff/CODEX_PROMPT_METRUM.md` (or `_AIPERF.md`) into
Codex. Use `phase_timer.sh` for every phase; keep `timings.jsonl` next to the
casts.
