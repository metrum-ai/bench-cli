# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Bake cargo/go deps so the self-hosted bench-cli job rarely apt-gets on a
# shared host lock. Base image registers via ACCESS_TOKEN / RUNNER_TOKEN.
# AppArmor is not required and is not installed in this image.
#
# Non-root execution (#292): the base image creates the `runner` account
# (uid 1001, group gid 121, home /home/runner) and adds it to the sudo
# (passwordless, via the %sudo sudoers entry) and docker groups. This image
# REMOVES the runner account from the sudo group, installs the toolchains
# under /opt (never under /root), pre-owns every path the runner writes, and
# ends as the numeric runner user with RUN_AS_ROOT=false, which the upstream
# entrypoint supports: started non-root it executes the CMD directly (no
# gosu drop needed).
#
# Remaining risk, stated honestly: this removes the container-root and
# passwordless-sudo paths for job code, but it is defense in depth, not a
# hard boundary. The gpu-h100 compose service mounts the host Docker socket
# (/var/run/docker.sock), which is host-root-equivalent by design; that
# service is as privileged as before, just not through a root shell.

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
        procps \
        python3 \
        python3-pip \
        python3-venv \
        unzip \
        xz-utils \
    && rm -rf /var/lib/apt/lists/*

ARG GO_VERSION=1.26.6
RUN curl -fsSL "https://go.dev/dl/go${GO_VERSION}.linux-amd64.tar.gz" \
    | tar -C /usr/local -xz \
    && ln -sf /usr/local/go/bin/go /usr/local/bin/go \
    && ln -sf /usr/local/go/bin/gofmt /usr/local/bin/gofmt

# rust-toolchain action still pins the channel; this cuts cold-start time.
# Shared toolchain path (#292): install under /opt/rust, runner-owned, so
# the non-root final user can execute it and rust-cache/cargo can write
# the registry cache there. Symlinks into /usr/local/bin keep the same
# invocation paths as before.
RUN curl -fsSL https://sh.rustup.rs \
    | env CARGO_HOME=/opt/rust/cargo RUSTUP_HOME=/opt/rust/rustup \
      sh -s -- -y --default-toolchain stable --no-modify-path \
    && ln -sf /opt/rust/cargo/bin/rustc /usr/local/bin/rustc \
    && ln -sf /opt/rust/cargo/bin/cargo /usr/local/bin/cargo \
    && ln -sf /opt/rust/cargo/bin/rustup /usr/local/bin/rustup
ENV CARGO_HOME=/opt/rust/cargo \
    RUSTUP_HOME=/opt/rust/rustup

WORKDIR /actions-runner

# Non-root execution (#292): the upstream entrypoint chowns these paths only
# when it starts as root; started as the runner user it executes the CMD
# directly, so the image itself must ship the right ownership. Numeric
# uid/gid (1001/121) match USER below and never rely on a named group.
RUN chown -R 1001:121 /actions-runner \
    && chown 1001:121 /opt/hostedtoolcache \
    && mkdir -p /tmp/runner-work \
    && chown 1001:121 /tmp/runner-work \
    && chown -R 1001:121 /opt/rust

# Remove the base image's passwordless sudo grant for the runner account
# (#292): the upstream install adds runner to the sudo group and sudoers
# grants %sudo ALL=(ALL) NOPASSWD: ALL. Job code must not be able to sudo
# back to root; the runner account keeps no sudo rights (it has no
# password, so even prompted sudo is unusable).
RUN gpasswd -d runner sudo

# Final user: the base image runner account, by numeric uid/gid (1001/121).
# RUN_AS_ROOT must be false: the upstream entrypoint exits with an error if
# RUN_AS_ROOT is unset (defaults true) while the container user is non-root.
ENV RUN_AS_ROOT=false
USER 1001:121

# Runner-appropriate health check (#292): the registered agent listens as
# Runner.Listener under the runner account (uid 1001); a dead listener is an
# unhealthy container even though the entrypoint itself is still alive.
HEALTHCHECK --interval=5m --timeout=10s --start-period=2m --retries=3 \
    CMD pgrep -u 1001 -f Runner.Listener >/dev/null || exit 1
