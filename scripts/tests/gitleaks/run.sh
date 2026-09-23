#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Assert custom gitleaks rules fire on scripts/tests/gitleaks fixtures.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$ROOT"

if ! command -v gitleaks >/dev/null 2>&1; then
  echo "gitleaks_test: gitleaks not installed" >&2
  exit 1
fi

# Allowlist excludes this directory from repo-wide scans; here we scan it
# explicitly with a temporary config that does not allowlist the fixtures.
TMP_CFG="$(mktemp)"
cleanup() { rm -f "$TMP_CFG"; }
trap cleanup EXIT

# Strip the fixture allowlist so detect must report findings.
grep -v "scripts/tests/gitleaks" .gitleaks.toml > "$TMP_CFG" || true
# Ensure allowlist paths array is empty / absent — rewrite a minimal config.
cat > "$TMP_CFG" <<'EOF'
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
title = "metrum-ai-bench-cli gitleaks fixture runner"
[extend]
useDefault = true
[[rules]]
id = "aws-account-id-near-account"
description = "12-digit AWS account ID adjacent to account wording"
regex = '''(?i)account[^0-9]{0,40}\b\d{12}\b|\b\d{12}\b[^0-9]{0,40}account'''
keywords = ["account"]
[[rules]]
id = "cleartext-password-assignment"
description = "Cleartext password assignment"
regex = '''(?i)password\s*[:=]\s*\S+'''
keywords = ["password"]
[[rules]]
id = "ssh-public-key-material"
description = "SSH public key material (rsa/ed25519)"
regex = '''ssh-(rsa|ed25519)\s+AAAA[0-9A-Za-z+/=]+'''
keywords = ["ssh-rsa", "ssh-ed25519"]
[[rules]]
id = "internal-hostname-suffix"
description = "Internal hostname matching the org suffix pattern"
regex = '''(?i)\b[a-z0-9][a-z0-9.-]*\.metrum\.ai\b'''
keywords = ["metrum.ai"]
EOF

set +e
gitleaks detect --no-git --source scripts/tests/gitleaks --config "$TMP_CFG" --verbose
status=$?
set -e

if [[ "$status" -eq 0 ]]; then
  echo "gitleaks_test: expected findings in fixture dir, got clean exit" >&2
  exit 1
fi

echo "gitleaks_test: ok (custom rules fired on fixtures)"
