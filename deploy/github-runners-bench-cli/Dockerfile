# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Bake cargo/go deps so the self-hosted bench-cli job rarely apt-gets on a
# shared host lock. Base image registers via ACCESS_TOKEN / RUNNER_TOKEN.
# AppArmor is not required and is not installed in this image.

FROM myoung34/github-runner:ubuntu-noble

USER root
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        cmake \
        curl \
        git \
        jq \
        libssl-dev \
        pkg-config \
        python3 \
        python3-pip \
        python3-venv \
        unzip \
    && rm -rf /var/lib/apt/lists/*

ARG GO_VERSION=1.26.6
RUN curl -fsSL "https://go.dev/dl/go${GO_VERSION}.linux-amd64.tar.gz" \
    | tar -C /usr/local -xz \
    && ln -sf /usr/local/go/bin/go /usr/local/bin/go \
    && ln -sf /usr/local/go/bin/gofmt /usr/local/bin/gofmt

# rust-toolchain action still pins the channel; this cuts cold-start time.
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --default-toolchain stable \
    && ln -sf /root/.cargo/bin/rustc /usr/local/bin/rustc \
    && ln -sf /root/.cargo/bin/cargo /usr/local/bin/cargo \
    && ln -sf /root/.cargo/bin/rustup /usr/local/bin/rustup
ENV PATH="/root/.cargo/bin:${PATH}"

WORKDIR /actions-runner
