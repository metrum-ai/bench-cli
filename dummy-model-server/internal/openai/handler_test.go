// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

package openai_test

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/config"
	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/server"
)

func testCfg(mods ...func(*config.Config)) *config.Config {
	cfg := &config.Config{
		Port: 8000, Model: "dummy", Compat: config.CompatOpenAI,
		IncludeUsage: true, AllowAnyModel: true, MaxImages: 4, ImageSize: "64x64",
	}
	for _, m := range mods {
		m(cfg)
	}
	return cfg
}

func TestChatNonStream(t *testing.T) {
	h := server.New(testCfg())
	body := `{"model":"dummy","messages":[{"role":"user","content":"Hi there friend"}],"max_tokens":5,"stream":false}`
	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", strings.NewReader(body))
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("status %d body %s", rec.Code, rec.Body.String())
	}
	var resp map[string]any
	if err := json.Unmarshal(rec.Body.Bytes(), &resp); err != nil {
		t.Fatal(err)
	}
	if resp["object"] != "chat.completion" {
		t.Fatalf("object=%v", resp["object"])
	}
	usage := resp["usage"].(map[string]any)
	// len("Hi there friend")/4 = 3
	if int(usage["prompt_tokens"].(float64)) != 3 {
		t.Fatalf("prompt_tokens=%v want 3", usage["prompt_tokens"])
	}
	if int(usage["completion_tokens"].(float64)) != 5 {
		t.Fatalf("completion_tokens=%v", usage["completion_tokens"])
	}
}

func TestChatStreamDoneAndUsage(t *testing.T) {
	h := server.New(testCfg())
	body := `{"model":"dummy","messages":[{"role":"user","content":"Hi"}],"max_tokens":3,"stream":true,"stream_options":{"include_usage":true}}`
	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", strings.NewReader(body))
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("status %d", rec.Code)
	}
	if ct := rec.Header().Get("Content-Type"); ct != "text/event-stream" {
		t.Fatalf("content-type %s", ct)
	}
	raw := rec.Body.String()
	if !strings.Contains(raw, "data: [DONE]") {
		t.Fatal("missing data: [DONE]")
	}
	if !strings.Contains(raw, `"usage"`) {
		t.Fatal("missing usage on stream")
	}
	if !strings.Contains(raw, `"content":"."`) {
		t.Fatal("missing content deltas")
	}
}

func TestVLLMIgnoreEOSIncludeUsageReasoning(t *testing.T) {
	h := server.New(testCfg(func(c *config.Config) {
		c.Compat = config.CompatVLLM
		c.Reasoning = true
	}))
	body := `{"model":"dummy","messages":[{"role":"user","content":"Hi"}],"max_tokens":4,"stream":true,"ignore_eos":true,"stream_options":{"include_usage":true}}`
	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", strings.NewReader(body))
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	raw := rec.Body.String()
	if !strings.Contains(raw, "reasoning_content") {
		t.Fatal("expected reasoning_content delta")
	}
	if !strings.Contains(raw, `"usage"`) {
		t.Fatal("expected usage")
	}
	// 4 content tokens + reasoning + finish
	nContent := strings.Count(raw, `"content":"."`)
	if nContent != 4 {
		t.Fatalf("ignore_eos should emit max_tokens content chunks, got %d", nContent)
	}
}

func TestSGLangUnknownFieldsOK(t *testing.T) {
	h := server.New(testCfg(func(c *config.Config) {
		c.Compat = config.CompatSGLang
	}))
	body := `{"model":"dummy","messages":[{"role":"user","content":"Hi"}],"max_tokens":2,"stream":false,"sampling_params":{"temperature":0.7},"lora_path":"/tmp/x"}`
	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", strings.NewReader(body))
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("SGLang unknown fields should not 400: %d %s", rec.Code, rec.Body.String())
	}
}

func TestVLMImageURL(t *testing.T) {
	h := server.New(testCfg())
	body := `{"model":"dummy","messages":[{"role":"user","content":[{"type":"text","text":"What?"},{"type":"image_url","image_url":{"url":"data:image/jpeg;base64,abc"}}]}],"max_tokens":2,"stream":false}`
	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", strings.NewReader(body))
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("status %d %s", rec.Code, rec.Body.String())
	}
	var resp map[string]any
	_ = json.Unmarshal(rec.Body.Bytes(), &resp)
	usage := resp["usage"].(map[string]any)
	pt := int(usage["prompt_tokens"].(float64))
	// len("What?")/4=1 + 256 image
	if pt < 256 {
		t.Fatalf("prompt_tokens=%d want >=256 for image_url", pt)
	}
}

func TestHealth(t *testing.T) {
	h := server.New(testCfg())
	req := httptest.NewRequest(http.MethodGet, "/health", nil)
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("health %d", rec.Code)
	}
}

func TestModels(t *testing.T) {
	h := server.New(testCfg())
	req := httptest.NewRequest(http.MethodGet, "/v1/models", nil)
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatal(rec.Code)
	}
}

func TestOmitDoneAndRoleOnly(t *testing.T) {
	h := server.New(testCfg(func(c *config.Config) {
		c.OmitDone = true
		c.RoleOnly = true
	}))
	body := `{"model":"dummy","messages":[{"role":"user","content":"Hi"}],"max_tokens":5,"stream":true}`
	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", strings.NewReader(body))
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	raw := rec.Body.String()
	if strings.Contains(raw, "data: [DONE]") {
		t.Fatal("omit-done should suppress [DONE]")
	}
	if strings.Contains(raw, `"content":"."`) {
		t.Fatal("role-only should not emit content")
	}
	if !strings.Contains(raw, `"role":"assistant"`) {
		t.Fatal("role-only should emit role delta")
	}
}

func TestGoldenTimingShape(t *testing.T) {
	// latency=100ms, chunk-interval=20ms, max_tokens=20 → TTFT~120ms, RT~500ms
	h := server.New(testCfg(func(c *config.Config) {
		c.Latency = 100 * time.Millisecond
		c.ChunkInterval = 20 * time.Millisecond
	}))
	body := `{"model":"dummy","messages":[{"role":"user","content":"Hi"}],"max_tokens":20,"stream":true}`
	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", strings.NewReader(body))
	rec := httptest.NewRecorder()
	start := time.Now()
	h.ServeHTTP(rec, req)
	elapsed := time.Since(start)
	if elapsed < 450*time.Millisecond || elapsed > 700*time.Millisecond {
		t.Fatalf("RT shape: elapsed=%v want ~500ms (450-700)", elapsed)
	}
	raw := rec.Body.String()
	if strings.Count(raw, `"content":"."`) != 20 {
		t.Fatalf("want 20 content tokens, got %d", strings.Count(raw, `"content":"."`))
	}
}
