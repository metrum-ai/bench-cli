# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
SHELL := /bin/bash
VERSION ?= 0.1.0
DOCS_LATEST_VERSION ?= $(VERSION)
DOCS_RELEASE_VERSIONS ?= $(VERSION)
PRODUCT_SLUG := metrum-ai-bench-cli
SITE_ROOT := metrum-ai-bench-cli
DIST_DIR := dist

.PHONY: install start build build-latest package clean

install:
	npm ci

start:
	DOCS_BASE_URL=/ npm run start

build:
	@test -d node_modules || npm ci
	DOCS_VERSION=$(VERSION) \
	DOCS_BASE_URL=/$(SITE_ROOT)/$(VERSION)/ \
	DOCS_VERSIONS_URL=/$(SITE_ROOT)/versions.json \
	npm run build

build-latest:
	@test -d node_modules || npm ci
	DOCS_VERSION=$(VERSION) \
	DOCS_BASE_URL=/$(SITE_ROOT)/latest/ \
	DOCS_VERSIONS_URL=/$(SITE_ROOT)/versions.json \
	npm run build -- --out-dir build-latest

package:
	python3 scripts/package-docs.py \
		--version $(VERSION) \
		--latest-version $(DOCS_LATEST_VERSION) \
		--release-versions "$(DOCS_RELEASE_VERSIONS)" \
		--site-root $(SITE_ROOT) \
		--dist-dir $(DIST_DIR) \
		--latest-build-dir build-latest

clean:
	rm -rf build build-latest $(DIST_DIR)
	npm run clear
