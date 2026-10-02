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
