# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
.PHONY: all debug release test lint clean smoke-regen check-headers render-cli-help render-data-points

all: debug

debug:
	cargo build --all-targets

release:
	cargo build --release

test:
	cargo test --all-targets

lint:
	cargo fmt --all -- --check
	cargo clippy --all-targets -- -D warnings

clean:
	cargo clean

# After the campaign skill completes a live run, publish its manifest without rerunning load.
# Usage: make smoke-regen CAMPAIGN_ID=<id> MANIFEST=<path-to-run-manifest.json>
smoke-regen:
	cargo xtask campaign-manifest --campaign-id "$(CAMPAIGN_ID)" --source "$(MANIFEST)"

# CI gate helpers (see cargo xtask --help)
check-headers:
	cargo xtask check-headers

render-cli-help:
	cargo xtask render-cli-help

render-data-points:
	cargo xtask render-data-points

