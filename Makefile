# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
.PHONY: all debug release test lint clean smoke-regen

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

# Live GPU campaign. Dry-run by default; pass EXECUTE=1 to launch.
smoke-regen:
	@if [ "$(EXECUTE)" = "1" ]; then \
		bash scripts/live/regen_smoke.sh --execute; \
	else \
		bash scripts/live/regen_smoke.sh; \
	fi
