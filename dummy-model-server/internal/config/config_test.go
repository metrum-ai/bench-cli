// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

package config_test

import (
	"testing"
	"time"

	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/config"
)

func TestParseFlags(t *testing.T) {
	cfg, err := config.ParseFlags([]string{
		"-port", "9000",
		"-latency", "100ms",
		"-chunk-interval", "20ms",
		"-compat", "vllm",
		"-req-per-sec", "1",
		"-tokens-per-sec", "100",
		"-reasoning",
	})
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Port != 9000 || cfg.Latency != 100*time.Millisecond || cfg.ChunkInterval != 20*time.Millisecond {
		t.Fatalf("unexpected cfg: %+v", cfg)
	}
	if cfg.Compat != config.CompatVLLM || !cfg.Reasoning {
		t.Fatalf("compat/reasoning: %+v", cfg)
	}
}

func TestParseFlagsBadCompat(t *testing.T) {
	_, err := config.ParseFlags([]string{"-compat", "nope"})
	if err == nil {
		t.Fatal("expected error")
	}
}

func TestParseFlagsStrictMedia(t *testing.T) {
	cfg, err := config.ParseFlags(nil)
	if err != nil {
		t.Fatal(err)
	}
	if cfg.StrictMedia {
		t.Fatal("strict-media must default to false")
	}
	cfg, err = config.ParseFlags([]string{"-strict-media"})
	if err != nil {
		t.Fatal(err)
	}
	if !cfg.StrictMedia {
		t.Fatal("-strict-media should set StrictMedia")
	}
	cfg, err = config.ParseFlags([]string{"-strict-media=false"})
	if err != nil {
		t.Fatal(err)
	}
	if cfg.StrictMedia {
		t.Fatal("-strict-media=false should clear StrictMedia")
	}
}

func TestParseFlagsDoneTail(t *testing.T) {
	cfg, err := config.ParseFlags([]string{"-done-tail", "20ms"})
	if err != nil {
		t.Fatal(err)
	}
	if cfg.DoneTail != 20*time.Millisecond {
		t.Fatalf("DoneTail = %v", cfg.DoneTail)
	}
	if _, err := config.ParseFlags([]string{"-done-tail", "-5ms"}); err == nil {
		t.Fatal("negative -done-tail must be rejected")
	}
}

func TestParseFlagsIdleTimeout(t *testing.T) {
	cfg, err := config.ParseFlags(nil)
	if err != nil {
		t.Fatal(err)
	}
	// Default must match the previous hard-coded http.Server value.
	if cfg.IdleTimeout != 60*time.Second {
		t.Fatalf("IdleTimeout default = %v, want 60s", cfg.IdleTimeout)
	}
	cfg, err = config.ParseFlags([]string{"-idle-timeout", "30m"})
	if err != nil {
		t.Fatal(err)
	}
	if cfg.IdleTimeout != 30*time.Minute {
		t.Fatalf("IdleTimeout = %v, want 30m", cfg.IdleTimeout)
	}
	// 0 is accepted and left as-is: Go http.Server then falls back to
	// ReadTimeout (30 s here) for idle keep-alive connections — it does NOT
	// mean "never close idle connections".
	cfg, err = config.ParseFlags([]string{"-idle-timeout", "0"})
	if err != nil {
		t.Fatal(err)
	}
	if cfg.IdleTimeout != 0 {
		t.Fatalf("IdleTimeout = %v, want 0", cfg.IdleTimeout)
	}
	if _, err := config.ParseFlags([]string{"-idle-timeout", "soon"}); err == nil {
		t.Fatal("invalid -idle-timeout must be rejected")
	}
	if _, err := config.ParseFlags([]string{"-idle-timeout", "-1s"}); err == nil {
		t.Fatal("negative -idle-timeout must be rejected")
	}
}
