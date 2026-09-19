#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Migrate pikachu from bare-metal ~/github-runners-bench-cli CI runners to
# a compose pool. Run ON pikachu after copying this directory to
# ~/github-runners-bench-cli-compose (keep the old dir until the new pool
# is online). Leave the bench-cli-docs-deploy systemd runner running.
# AppArmor is not required.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"

if [[ ! -f .env ]]; then
  cp .env.example .env
  echo "Edit $ROOT/.env and set RUNNER_TOKEN (or ACCESS_TOKEN), then re-run."
  exit 1
fi

if ! grep -qE '^RUNNER_TOKEN=.+' .env && ! grep -qE '^ACCESS_TOKEN=.+' .env; then
  if command -v gh >/dev/null; then
    token="$(gh api -X POST repos/metrum-ai/bench-cli/actions/runners/registration-token --jq .token)"
    printf '\nRUNNER_TOKEN=%s\n' "$token" >> .env
    echo "Wrote short-lived RUNNER_TOKEN to .env"
  else
    echo "Set RUNNER_TOKEN or ACCESS_TOKEN in .env"
    exit 1
  fi
fi

if docker compose version >/dev/null 2>&1; then
  dc=(docker compose)
else
  dc=(docker-compose)
fi
if [[ -x "$ROOT/pool" ]]; then
  "$ROOT/pool" recreate
else
  sudo "${dc[@]}" build
  sudo "${dc[@]}" up -d
  sudo "${dc[@]}" ps
fi

cat <<'EOF'

Next:
1. GitHub -> repo Settings -> Actions -> Runners: confirm pikachu-bench-cli workers Idle.
2. Offline/remove old bare-metal runners that carry only the bench-cli label.
3. Stop those systemd units, e.g.:
     sudo systemctl disable --now actions.runner.*bench-cli* || true
   Keep the unit whose labels include bench-cli-docs-deploy.
4. Re-run CI self-hosted cargo test.

Manual later:
  ./pool start | stop | restart | status | logs
  See MANUAL in this directory.

AppArmor is not required.
Scale: copy a runner-N service block in docker-compose.yml and up -d.
EOF
