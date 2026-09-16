# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# RC5 CI admin follow-ups (NOT committed — paste into PR body if useful).
# Generated because branch protection API returned HTTP 403 on a private repo
# without GitHub Pro. Topics and Discussions were enabled successfully.

## Branch protection (403 — needs Pro or public repo)

```bash
gh api -X PUT repos/metrum-ai/bench-cli/branches/main/protection --input - <<'JSON'
{
  "required_status_checks": {
    "strict": true,
    "contexts": [
      "fmt, clippy, test (stable)",
      "MSRV 1.85",
      "secret scanning (gitleaks)",
      "core line coverage (80%)",
      "cargo deny check (all)",
      "dco"
    ]
  },
  "enforce_admins": true,
  "required_pull_request_reviews": {
    "required_approving_review_count": 0,
    "dismiss_stale_reviews": true
  },
  "required_linear_history": true,
  "allow_force_pushes": false,
  "allow_deletions": false,
  "required_conversation_resolution": true,
  "restrictions": null
}
JSON
```

403 body: `Upgrade to GitHub Pro or make this repository public to enable this feature.`

Post-launch: raise `required_approving_review_count` from 0 to 1.

Optional after CodeQL is green on main: add `Analyze (rust)` to required contexts.

## Topics (succeeded)

```bash
gh repo edit metrum-ai/bench-cli \
  --add-topic asr --add-topic vlm --add-topic image-generation \
  --add-topic llm --add-topic benchmarking --add-topic inference \
  --add-topic rust --add-topic openai-compatible
```

## Discussions (succeeded)

```bash
gh repo edit metrum-ai/bench-cli --enable-discussions
```
