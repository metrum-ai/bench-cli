# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0

ARG GO_BASE_IMAGE=golang:1.26-alpine
ARG ALPINE_BASE_IMAGE=alpine:3.20

FROM ${GO_BASE_IMAGE} AS builder
WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=0 go build -o /dummy-model-server ./cmd/dummy-model-server

FROM ${ALPINE_BASE_IMAGE}
COPY --from=builder /dummy-model-server /dummy-model-server
EXPOSE 8000
ENTRYPOINT ["/dummy-model-server"]
