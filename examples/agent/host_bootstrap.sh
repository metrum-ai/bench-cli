#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Idempotent, arch-aware host bootstrap for a bench campaign.
# No secrets are read, written, or logged.
#
# Environment:
#   TAG     Release tag, for example v1.0.0 (required unless already installed).
#   BASE    Release base URL, for example https://github.com/metrum-ai/bench-cli/releases/download/<TAG>.
#   MIRROR  Optional local or air-gapped directory holding the same assets.
#   PREFIX  Install root (default /opt/bench, or $HOME/bench when /opt is not writable).
#   TARGET  Optional target triple; derived from uname when unset.

set -euo pipefail

TAG="${TAG:-}"
BASE="${BASE:-}"
MIRROR="${MIRROR:-}"
TARGET="${TARGET:-}"
PREFIX="${PREFIX:-}"

log() { printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%S.%3NZ)" "$*"; }
die() { log "error: $*" >&2; exit 1; }

detect_target() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"
    case "${os}:${arch}" in
        Linux:x86_64) echo "x86_64-unknown-linux-gnu" ;;
        Linux:aarch64 | Linux:arm64) echo "aarch64-unknown-linux-gnu" ;;
        Darwin:x86_64) echo "x86_64-apple-darwin" ;;
        Darwin:arm64) echo "aarch64-apple-darwin" ;;
        *) die "unsupported host ${os} ${arch}" ;;
    esac
}

if [ -z "${TARGET}" ]; then
    TARGET="$(detect_target)"
fi

if [ -z "${PREFIX}" ]; then
    if mkdir -p /opt/bench 2>/dev/null && [ -w /opt/bench ]; then
        PREFIX=/opt/bench
    else
        PREFIX="${HOME}/bench"
    fi
fi

BIN_DIR="${PREFIX}/bin"
REPO_DIR="${PREFIX}/repo"
ENV_DIR="${PREFIX}/env"
mkdir -p "${BIN_DIR}" "${REPO_DIR}" "${ENV_DIR}"

ARCHIVE="metrum-ai-bench-cli-${TAG}-${TARGET}.tar.gz"
MANIFEST="${ENV_DIR}/MANIFEST.json"

if [ -z "${TAG}" ]; then
    die "TAG is required (for example TAG=v1.0.0)"
fi

fetch() {
    # fetch <asset-name> <destination>
    local name="$1" dest="$2"
    if [ -n "${MIRROR}" ] && [ -f "${MIRROR}/${name}" ]; then
        cp -f "${MIRROR}/${name}" "${dest}"
        return 0
    fi
    [ -n "${BASE}" ] || die "BASE or MIRROR is required to fetch ${name}"
    curl -fsSL -o "${dest}" "${BASE}/${name}"
}

work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

log "fetching ${ARCHIVE} for ${TARGET}"
fetch "${ARCHIVE}" "${work}/${ARCHIVE}"
fetch "${ARCHIVE}.sha256" "${work}/${ARCHIVE}.sha256"

log "verifying sha256"
( cd "${work}" && sha256sum -c "${ARCHIVE}.sha256" )

if command -v cosign >/dev/null 2>&1; then
    if fetch "${ARCHIVE}.sigstore.json" "${work}/${ARCHIVE}.sigstore.json"; then
        log "verifying Sigstore bundle"
        cosign verify-blob \
            --bundle "${work}/${ARCHIVE}.sigstore.json" \
            --certificate-identity-regexp 'https://github.com/metrum-ai/bench-cli/' \
            --certificate-oidc-issuer https://token.actions.githubusercontent.com \
            "${work}/${ARCHIVE}"
    else
        log "warning: no Sigstore bundle found; sha256 only"
    fi
else
    log "warning: cosign not installed; sha256 only"
fi

log "unpacking to ${PREFIX}"
tar -xzf "${work}/${ARCHIVE}" -C "${work}"
root="$(find "${work}" -maxdepth 1 -type d -name 'metrum-ai-bench-cli-*' | head -n1)"
[ -n "${root}" ] || die "archive did not contain an unpacked root"

if [ -d "${root}/bin" ]; then
    cp -f "${root}/bin/"* "${BIN_DIR}/"
fi
if [ -d "${root}/test-data" ]; then
    mkdir -p "${REPO_DIR}/test-data"
    cp -f "${root}/test-data/"* "${REPO_DIR}/test-data/" 2>/dev/null || true
fi
if [ -d "${root}/examples" ]; then
    mkdir -p "${REPO_DIR}/examples"
    cp -f "${root}/examples/"* "${REPO_DIR}/examples/" 2>/dev/null || true
fi

# The source tag supplies fixtures when the release archive did not.
if [ ! -d "${REPO_DIR}/test-data" ] && [ -n "${BASE}" ]; then
    src="https://github.com/metrum-ai/bench-cli/archive/refs/tags/${TAG}.tar.gz"
    log "fetching source tag for test-data"
    if [ -n "${MIRROR}" ] && [ -f "${MIRROR}/source-${TAG}.tar.gz" ]; then
        cp -f "${MIRROR}/source-${TAG}.tar.gz" "${work}/source.tar.gz"
    else
        curl -fsSL -o "${work}/source.tar.gz" "${src}"
    fi
    mkdir -p "${work}/src"
    tar -xzf "${work}/source.tar.gz" -C "${work}/src"
    srcroot="$(find "${work}/src" -maxdepth 1 -type d -name 'bench-cli-*' | head -n1)"
    [ -n "${srcroot}" ] || die "source tag did not unpack"
    cp -fR "${srcroot}/test-data" "${REPO_DIR}/test-data"
fi

chmod +x "${BIN_DIR}"/metrum-ai-bench-cli* 2>/dev/null || true

log "running selftest"
"${BIN_DIR}/metrum-ai-bench-cli" selftest

log "writing ${MANIFEST}"
{
    printf '{\n'
    printf '  "tag": "%s",\n' "${TAG}"
    printf '  "target": "%s",\n' "${TARGET}"
    printf '  "prefix": "%s",\n' "${PREFIX}"
    printf '  "generated_at": "%s",\n' "$(date -u +%Y-%m-%dT%H:%M:%S.%3NZ)"
    printf '  "files": [\n'
    first=1
    while IFS= read -r f; do
        [ -f "${f}" ] || continue
        if [ "${first}" -eq 0 ]; then printf ',\n'; fi
        first=0
        printf '    {"path": "%s", "sha256": "%s"}' "${f}" "$(sha256sum "${f}" | awk '{print $1}')"
    done < <(find "${BIN_DIR}" -maxdepth 1 -type f | sort)
    printf '\n  ]\n}\n'
} > "${MANIFEST}"

log "bootstrap complete: ${PREFIX}"
