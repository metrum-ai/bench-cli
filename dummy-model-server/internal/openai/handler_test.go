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
	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/media/mediatest"
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

// finalUsage returns the last usage object in an SSE body.
func finalUsage(t *testing.T, raw string) map[string]any {
	t.Helper()
	var found map[string]any
	for _, line := range strings.Split(raw, "\n") {
		data, ok := strings.CutPrefix(line, "data: ")
		if !ok || data == "[DONE]" {
			continue
		}
		var chunk map[string]any
		if err := json.Unmarshal([]byte(data), &chunk); err != nil {
			continue
		}
		if u, ok := chunk["usage"].(map[string]any); ok {
			found = u
		}
	}
	if found == nil {
		t.Fatal("no usage chunk")
	}
	return found
}

func TestReasoningTokensStreamUsage(t *testing.T) {
	h := server.New(testCfg(func(c *config.Config) {
		c.ReasoningTokens = 3
	}))
	body := `{"model":"dummy","messages":[{"role":"user","content":"Hi"}],"max_tokens":4,"stream":true,"stream_options":{"include_usage":true}}`
	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", strings.NewReader(body))
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	raw := rec.Body.String()
	if n := strings.Count(raw, `"reasoning_content":"think"`); n != 3 {
		t.Fatalf("reasoning chunks = %d, want 3", n)
	}
	u := finalUsage(t, raw)
	if got := int(u["completion_tokens"].(float64)); got != 7 {
		t.Fatalf("completion_tokens = %d, want 4 visible + 3 reasoning", got)
	}
	details := u["completion_tokens_details"].(map[string]any)
	if got := int(details["reasoning_tokens"].(float64)); got != 3 {
		t.Fatalf("reasoning_tokens = %d, want 3", got)
	}
}

func TestReasoningTokensNonStreamUsage(t *testing.T) {
	h := server.New(testCfg(func(c *config.Config) {
		c.ReasoningTokens = 5
	}))
	body := `{"model":"dummy","messages":[{"role":"user","content":"Hi"}],"max_tokens":4,"stream":false}`
	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", strings.NewReader(body))
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	var resp map[string]any
	if err := json.Unmarshal(rec.Body.Bytes(), &resp); err != nil {
		t.Fatal(err)
	}
	u := resp["usage"].(map[string]any)
	if got := int(u["completion_tokens"].(float64)); got != 9 {
		t.Fatalf("completion_tokens = %d, want 9", got)
	}
	if got := int(u["completion_tokens_details"].(map[string]any)["reasoning_tokens"].(float64)); got != 5 {
		t.Fatalf("reasoning_tokens = %d, want 5", got)
	}
}

func TestDefaultUsageHasNoCompletionTokensDetails(t *testing.T) {
	h := server.New(testCfg(func(c *config.Config) {
		c.Reasoning = true
	}))
	body := `{"model":"dummy","messages":[{"role":"user","content":"Hi"}],"max_tokens":2,"stream":true}`
	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", strings.NewReader(body))
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if strings.Contains(rec.Body.String(), "completion_tokens_details") {
		t.Fatal("default usage must not carry completion_tokens_details")
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

// chatWithImages posts a non-stream chat request carrying one image_url part
// per URL and returns the recorder.
func chatWithImages(t *testing.T, h http.Handler, urls ...string) *httptest.ResponseRecorder {
	t.Helper()
	parts := []map[string]any{{"type": "text", "text": "What is this?"}}
	for _, u := range urls {
		parts = append(parts, map[string]any{"type": "image_url", "image_url": map[string]any{"url": u}})
	}
	body, err := json.Marshal(map[string]any{
		"model":      "dummy",
		"messages":   []map[string]any{{"role": "user", "content": parts}},
		"max_tokens": 2,
		"stream":     false,
	})
	if err != nil {
		t.Fatal(err)
	}
	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", strings.NewReader(string(body)))
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return rec
}

// badImages are image_url values the Metrum AI strict-media mode must reject.
var badImages = []struct {
	name, url, detail string
}{
	{"1x1 png", mediatest.DataURL("image/png", mediatest.PNG(1, 1)), "image too small: 1x1"},
	{"old abc stub", "data:image/jpeg;base64,abc", "unsupported image format"},
	{"not base64", "data:image/png,raw", "must be base64-encoded"},
	{"corrupt base64", "data:image/png;base64,***", "invalid base64 in data: URL"},
	{"missing comma", "data:image/png;base64", "missing comma"},
}

func TestChatStrictMediaRejects(t *testing.T) {
	h := server.New(testCfg(func(c *config.Config) { c.StrictMedia = true }))
	good := mediatest.DataURL("image/png", mediatest.PNG(2, 2))
	for _, tc := range badImages {
		t.Run(tc.name, func(t *testing.T) {
			// Put a valid image first to prove every part is checked.
			rec := chatWithImages(t, h, good, tc.url)
			if rec.Code != http.StatusBadRequest {
				t.Fatalf("status %d want 400; body %s", rec.Code, rec.Body.String())
			}
			if ct := rec.Header().Get("Content-Type"); ct != "application/json" {
				t.Fatalf("content-type %q", ct)
			}
			var out struct {
				Error struct {
					Message string `json:"message"`
					Type    string `json:"type"`
					Code    int    `json:"code"`
				} `json:"error"`
			}
			if err := json.Unmarshal(rec.Body.Bytes(), &out); err != nil {
				t.Fatalf("error body not JSON: %v", err)
			}
			if !strings.HasPrefix(out.Error.Message, "Invalid image: ") || !strings.Contains(out.Error.Message, tc.detail) {
				t.Fatalf("message %q want prefix %q and detail %q", out.Error.Message, "Invalid image: ", tc.detail)
			}
			if out.Error.Type != "invalid_request_error" || out.Error.Code != 400 {
				t.Fatalf("error shape %+v", out.Error)
			}
		})
	}
}

func TestChatStrictMediaRejectsStream(t *testing.T) {
	h := server.New(testCfg(func(c *config.Config) { c.StrictMedia = true }))
	body := `{"model":"dummy","messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"data:image/jpeg;base64,abc"}}]}],"max_tokens":2,"stream":true}`
	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", strings.NewReader(body))
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusBadRequest {
		t.Fatalf("stream request with bad image: status %d want 400", rec.Code)
	}
	if strings.Contains(rec.Body.String(), "data:") {
		t.Fatal("rejected request must not start an SSE stream")
	}
}

func TestChatStrictMediaAccepts(t *testing.T) {
	h := server.New(testCfg(func(c *config.Config) { c.StrictMedia = true }))
	rec := chatWithImages(t, h,
		mediatest.DataURL("image/png", mediatest.PNG(2, 2)),
		mediatest.DataURL("image/jpeg", mediatest.JPEG(8, 8)),
		mediatest.DataURL("image/gif", mediatest.GIF(4, 3)),
		"https://example.com/cat.png",
	)
	if rec.Code != http.StatusOK {
		t.Fatalf("status %d %s", rec.Code, rec.Body.String())
	}
	var resp map[string]any
	if err := json.Unmarshal(rec.Body.Bytes(), &resp); err != nil {
		t.Fatal(err)
	}
	pt := int(resp["usage"].(map[string]any)["prompt_tokens"].(float64))
	if pt < 4*256 {
		t.Fatalf("prompt_tokens=%d want >= 1024 for 4 images", pt)
	}

	// Plain text requests are unaffected by strict mode.
	body := `{"model":"dummy","messages":[{"role":"user","content":"Hi"}],"max_tokens":2}`
	req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", strings.NewReader(body))
	rec = httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("text-only strict: status %d", rec.Code)
	}
}

func TestChatNonStrictAcceptsBadImages(t *testing.T) {
	h := server.New(testCfg())
	for _, tc := range badImages {
		rec := chatWithImages(t, h, tc.url)
		if rec.Code != http.StatusOK {
			t.Fatalf("%s: non-strict must accept, got %d %s", tc.name, rec.Code, rec.Body.String())
		}
	}
}
