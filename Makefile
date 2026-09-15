# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
.PHONY: all debug release test lint clean

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
