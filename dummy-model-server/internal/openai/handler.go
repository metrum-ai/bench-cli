// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

package openai

import (
	"encoding/json"
	"math/rand"
	"net/http"
	"time"

	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/config"
	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/limiter"
	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/sse"
	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/vision"
)

// Handler serves /v1/models, /v1/chat/completions, /v1/completions.
type Handler struct {
	Cfg     *config.Config
	Limiter *limiter.Limits
}

// Models handles GET /v1/models.
func (h *Handler) Models(w http.ResponseWriter, r *http.Request) {
	resp := map[string]any{
		"object": "list",
		"data": []map[string]any{{
			"id":       h.Cfg.Model,
			"object":   "model",
			"created":  1686935002,
			"owned_by": "dummy-server",
		}},
	}
	writeJSON(w, http.StatusOK, resp)
}

// ChatCompletions handles POST /v1/chat/completions (string or multimodal content).
func (h *Handler) ChatCompletions(w http.ResponseWriter, r *http.Request) {
	raw, err := readBody(r)
	if err != nil {
		writeErr(w, http.StatusBadRequest, "invalid request body")
		return
	}
	var req chatRequest
	if err := json.Unmarshal(raw, &req); err != nil {
		writeErr(w, http.StatusBadRequest, "invalid request body")
		return
	}
	// Unknown fields are ignored for all compat modes (SGLang requires this).
	_ = h.Cfg.Compat

	maxTokens := req.MaxTokens
	if maxTokens <= 0 {
		maxTokens = 64
	}
	// vLLM ignore_eos: pin completion length to max_tokens (always our behavior).
	_ = req.IgnoreEOS

	promptTokens := promptTokensChat(req.Messages)
	includeUsage := h.Cfg.IncludeUsage
	if req.StreamOptions != nil && req.StreamOptions.IncludeUsage != nil {
		includeUsage = *req.StreamOptions.IncludeUsage
	}

	if !h.Limiter.AllowTokens(w, maxTokens) {
		return
	}

	if h.maybeError(w, req.Stream) {
		return
	}

	if h.Cfg.Latency > 0 {
		time.Sleep(h.Cfg.Latency)
	}

	model := req.Model
	if model == "" {
		model = h.Cfg.Model
	}

	if req.Stream {
		h.serveChatStream(w, model, maxTokens, promptTokens, includeUsage)
		return
	}
	serveChatNonStream(w, model, promptTokens, maxTokens)
}

// Completions handles POST /v1/completions (legacy text completions).
func (h *Handler) Completions(w http.ResponseWriter, r *http.Request) {
	raw, err := readBody(r)
	if err != nil {
		writeErr(w, http.StatusBadRequest, "invalid request body")
		return
	}
	var req completionRequest
	if err := json.Unmarshal(raw, &req); err != nil {
		writeErr(w, http.StatusBadRequest, "invalid request body")
		return
	}
	maxTokens := req.MaxTokens
	if maxTokens <= 0 {
		maxTokens = 64
	}
	promptTokens := len(req.Prompt) / 4
	if promptTokens == 0 {
		promptTokens = 1
	}
	if !h.Limiter.AllowTokens(w, maxTokens) {
		return
	}
	if h.maybeError(w, req.Stream) {
		return
	}
	if h.Cfg.Latency > 0 {
		time.Sleep(h.Cfg.Latency)
	}
	model := req.Model
	if model == "" {
		model = h.Cfg.Model
	}
	if req.Stream {
		h.serveCompletionStream(w, model, maxTokens, promptTokens)
		return
	}
	total := promptTokens + maxTokens
	writeJSON(w, http.StatusOK, map[string]any{
		"id":      "cmpl-dummy",
		"object":  "text_completion",
		"created": time.Now().Unix(),
		"model":   model,
		"choices": []map[string]any{{
			"index": 0, "text": "Hello.", "finish_reason": "stop",
		}},
		"usage": usage{PromptTokens: promptTokens, CompletionTokens: maxTokens, TotalTokens: total},
	})
}

func (h *Handler) maybeError(w http.ResponseWriter, stream bool) bool {
	if h.Cfg.ErrorRate <= 0 || rand.Float64() >= h.Cfg.ErrorRate {
		return false
	}
	if stream {
		// Defer to stream path via sentinel: return false and let stream inject.
		return false
	}
	http.Error(w, "service unavailable", http.StatusServiceUnavailable)
	return true
}

func (h *Handler) midStreamError() bool {
	return h.Cfg.ErrorRate > 0 && rand.Float64() < h.Cfg.ErrorRate
}

func (h *Handler) serveChatStream(w http.ResponseWriter, model string, maxTokens, promptTokens int, includeUsage bool) {
	sw, ok := sse.New(w, h.Cfg.SplitSSE, h.Cfg.OmitDone)
	if !ok {
		return
	}
	id := "chatcmpl-dummy"
	created := time.Now().Unix()

	if h.Cfg.RoleOnly {
		_ = sw.Data(streamChunk{
			ID: id, Object: "chat.completion.chunk", Created: created, Model: model,
			Choices: []streamChoice{{Index: 0, Delta: streamDelta{Role: "assistant"}}},
		})
		stop := "stop"
		final := streamChunk{
			ID: id, Object: "chat.completion.chunk", Created: created, Model: model,
			Choices: []streamChoice{{Index: 0, Delta: streamDelta{}, FinishReason: &stop}},
		}
		if includeUsage {
			final.Usage = &usage{PromptTokens: promptTokens, CompletionTokens: 0, TotalTokens: promptTokens}
		}
		_ = sw.Data(final)
		_ = sw.Done()
		return
	}

	// Optional reasoning deltas (vLLM-style) before visible content.
	if h.Cfg.Reasoning {
		if h.Cfg.ChunkInterval > 0 {
			time.Sleep(h.Cfg.ChunkInterval)
		}
		_ = sw.Data(streamChunk{
			ID: id, Object: "chat.completion.chunk", Created: created, Model: model,
			Choices: []streamChoice{{Index: 0, Delta: streamDelta{ReasoningContent: "think"}}},
		})
	}

	completionTokens := 0
	injectMid := h.Cfg.ErrorRate >= 1.0
	for i := 0; i < maxTokens; i++ {
		if (injectMid && i == 1) || (!injectMid && h.midStreamError() && i > 0) {
			_ = sw.ErrorObject("injected mid-stream error")
			return
		}
		if h.Cfg.ChunkInterval > 0 {
			time.Sleep(h.Cfg.ChunkInterval)
		}
		_ = sw.Data(streamChunk{
			ID: id, Object: "chat.completion.chunk", Created: created, Model: model,
			Choices: []streamChoice{{Index: 0, Delta: streamDelta{Content: "."}}},
		})
		completionTokens++
	}
	stop := "stop"
	final := streamChunk{
		ID: id, Object: "chat.completion.chunk", Created: created, Model: model,
		Choices: []streamChoice{{Index: 0, Delta: streamDelta{}, FinishReason: &stop}},
	}
	if includeUsage {
		final.Usage = &usage{
			PromptTokens:     promptTokens,
			CompletionTokens: completionTokens,
			TotalTokens:      promptTokens + completionTokens,
		}
	}
	_ = sw.Data(final)
	_ = sw.Done()
}

func (h *Handler) serveCompletionStream(w http.ResponseWriter, model string, maxTokens, promptTokens int) {
	sw, ok := sse.New(w, h.Cfg.SplitSSE, h.Cfg.OmitDone)
	if !ok {
		return
	}
	id := "cmpl-dummy"
	for i := 0; i < maxTokens; i++ {
		if h.Cfg.ChunkInterval > 0 {
			time.Sleep(h.Cfg.ChunkInterval)
		}
		_ = sw.Data(map[string]any{
			"id": id, "object": "text_completion.chunk", "model": model,
			"choices": []map[string]any{{"index": 0, "text": ".", "finish_reason": nil}},
		})
	}
	stop := "stop"
	final := map[string]any{
		"id": id, "object": "text_completion.chunk", "model": model,
		"choices": []map[string]any{{"index": 0, "text": "", "finish_reason": stop}},
		"usage": usage{PromptTokens: promptTokens, CompletionTokens: maxTokens, TotalTokens: promptTokens + maxTokens},
	}
	_ = sw.Data(final)
	_ = sw.Done()
}

func serveChatNonStream(w http.ResponseWriter, model string, promptTokens, completionTokens int) {
	if promptTokens == 0 {
		promptTokens = 1
	}
	writeJSON(w, http.StatusOK, chatCompletionResponse{
		ID: "chatcmpl-dummy", Object: "chat.completion", Created: time.Now().Unix(), Model: model,
		Choices: []choice{{
			Index: 0, Message: message{Role: "assistant", Content: "Hello."}, FinishReason: "stop",
		}},
		Usage: usage{
			PromptTokens: promptTokens, CompletionTokens: completionTokens,
			TotalTokens: promptTokens + completionTokens,
		},
	})
}

func promptTokensChat(messages []chatMessage) int {
	contents := make([]vision.Content, 0, len(messages))
	for _, m := range messages {
		contents = append(contents, m.Content)
	}
	return vision.PromptTokensFromParts(contents)
}
