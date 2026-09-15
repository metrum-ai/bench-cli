// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

package config

import (
	"flag"
	"fmt"
	"math/rand"
	"strings"
	"time"
)

// Compat selects vendor-shaped request/response quirks.
type Compat string

const (
	CompatOpenAI Compat = "openai"
	CompatVLLM   Compat = "vllm"
	CompatSGLang Compat = "sglang"
)

// Config holds CLI flags for the dummy model server.
type Config struct {
	Port           int
	Model          string
	Latency        time.Duration
	TokensPerSec   float64
	ReqPerSec      float64
	MaxConcurrency int
	ChunkInterval  time.Duration
	ErrorRate      float64
	SplitSSE       bool
	OmitDone       bool
	RoleOnly       bool
	Reasoning      bool
	IncludeUsage   bool
	Seed           int64
	Compat         Compat
	LogRequests    bool
	ImageSize      string
	MaxImages      int
	AllowAnyModel  bool
}

// ParseFlags defines and parses CLI flags.
func ParseFlags(args []string) (*Config, error) {
	fs := flag.NewFlagSet("dummy-model-server", flag.ContinueOnError)
	cfg := &Config{}

	fs.IntVar(&cfg.Port, "port", 8000, "Listen port")
	fs.StringVar(&cfg.Model, "model", "dummy", "Model id returned by /v1/models and responses")
	latency := fs.String("latency", "0", "Delay before first token / non-stream body (e.g. 100ms)")
	chunk := fs.String("chunk-interval", "0", "Delay between stream token events (e.g. 20ms)")
	fs.Float64Var(&cfg.TokensPerSec, "tokens-per-sec", 0, "Token bucket for completion tokens/s; 0 = unlimited")
	fs.Float64Var(&cfg.ReqPerSec, "req-per-sec", 0, "Token bucket for requests/s; 0 = unlimited")
	fs.IntVar(&cfg.MaxConcurrency, "max-concurrency", 0, "Max in-flight /v1 requests; 0 = unlimited")
	fs.Float64Var(&cfg.ErrorRate, "error-rate", 0, "Probability 0-1 of error (503 or mid-stream); 0 = off")
	fs.BoolVar(&cfg.SplitSSE, "split-sse", false, "Adversarial: flush mid-event SSE frames")
	fs.BoolVar(&cfg.OmitDone, "omit-done", false, "Adversarial: omit trailing data: [DONE]")
	fs.BoolVar(&cfg.RoleOnly, "role-only", false, "Adversarial: stream role delta only (no content)")
	fs.BoolVar(&cfg.Reasoning, "reasoning", false, "Emit delta.reasoning_content before content (vLLM-style)")
	fs.BoolVar(&cfg.IncludeUsage, "include-usage", true, "Include usage on final stream chunk by default")
	fs.Int64Var(&cfg.Seed, "seed", 0, "RNG seed for error injection and deterministic text; 0 = time-based")
	compat := fs.String("compat", "openai", "Vendor profile: openai|vllm|sglang")
	fs.BoolVar(&cfg.LogRequests, "log-requests", false, "Log method, path, content-length")
	fs.StringVar(&cfg.ImageSize, "image-size", "64x64", "Default image size WxH for image generations")
	fs.IntVar(&cfg.MaxImages, "max-images", 4, "Maximum n for image generations")
	fs.BoolVar(&cfg.AllowAnyModel, "allow-any-model", true, "Accept any model name on image generations")

	if err := fs.Parse(args); err != nil {
		return nil, err
	}

	if d, err := time.ParseDuration(*latency); err == nil {
		cfg.Latency = d
	} else if *latency != "0" && *latency != "" {
		return nil, fmt.Errorf("invalid -latency %q: %w", *latency, err)
	}
	if d, err := time.ParseDuration(*chunk); err == nil {
		cfg.ChunkInterval = d
	} else if *chunk != "0" && *chunk != "" {
		return nil, fmt.Errorf("invalid -chunk-interval %q: %w", *chunk, err)
	}

	c := Compat(strings.ToLower(strings.TrimSpace(*compat)))
	switch c {
	case CompatOpenAI, CompatVLLM, CompatSGLang:
		cfg.Compat = c
	default:
		return nil, fmt.Errorf("invalid -compat %q; must be openai|vllm|sglang", *compat)
	}
	if cfg.MaxImages <= 0 {
		return nil, fmt.Errorf("invalid -max-images %d", cfg.MaxImages)
	}
	if cfg.Seed != 0 {
		rand.Seed(cfg.Seed) //nolint:staticcheck // intentional global seed for error-rate
	}
	return cfg, nil
}
