# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0

ARG GO_BASE_IMAGE=golang:1.26.10-alpine
ARG ALPINE_BASE_IMAGE=alpine:3.20

FROM ${GO_BASE_IMAGE} AS builder
WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=0 go build -o /dummy-model-server ./cmd/dummy-model-server

FROM ${ALPINE_BASE_IMAGE}
# Track the Alpine 3.20 security repository rather than pinning transient package revisions.
# hadolint ignore=DL3018
RUN apk add --no-cache wget \
  && addgroup -S metrum && adduser -S -G metrum metrum
COPY --from=builder /dummy-model-server /dummy-model-server
# The account is created above; its numeric ID is assigned by Alpine.
# hadolint ignore=DL3066
USER metrum
EXPOSE 8000
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD ["wget", "-q", "-O", "/dev/null", "http://127.0.0.1:8000/healthz"]
ENTRYPOINT ["/dummy-model-server"]
