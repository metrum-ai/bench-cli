// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

package openai

import (
	"encoding/json"
	"errors"
	"math/rand"
	"net/http"
	"time"

	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/config"
	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/limiter"
	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/media"
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

	if h.Cfg.StrictMedia {
		if err := validateChatImages(req.Messages); err != nil {
			writeErrCode(w, http.StatusBadRequest, "Invalid image: "+err.Error())
			return
		}
	}

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
	serveChatNonStream(w, model, promptTokens, maxTokens, h.Cfg.ReasoningTokens)
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

	// Counted reasoning deltas before visible content (-reasoning-tokens).
	reasoningTokens := h.Cfg.ReasoningTokens
	for i := 0; i < reasoningTokens; i++ {
		if h.Cfg.ChunkInterval > 0 {
			time.Sleep(h.Cfg.ChunkInterval)
		}
		_ = sw.Data(streamChunk{
			ID: id, Object: "chat.completion.chunk", Created: created, Model: model,
			Choices: []streamChoice{{Index: 0, Delta: streamDelta{ReasoningContent: "think"}}},
		})
	}

	// Optional uncounted reasoning delta (vLLM-style) before visible content.
	if h.Cfg.Reasoning && reasoningTokens == 0 {
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
		final.Usage = withReasoning(&usage{
			PromptTokens:     promptTokens,
			CompletionTokens: completionTokens,
			TotalTokens:      promptTokens + completionTokens,
		}, reasoningTokens)
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
		"usage":   usage{PromptTokens: promptTokens, CompletionTokens: maxTokens, TotalTokens: promptTokens + maxTokens},
	}
	_ = sw.Data(final)
	_ = sw.Done()
}

func serveChatNonStream(w http.ResponseWriter, model string, promptTokens, completionTokens, reasoningTokens int) {
	if promptTokens == 0 {
		promptTokens = 1
	}
	msg := message{Role: "assistant", Content: "Hello."}
	if reasoningTokens > 0 {
		msg.ReasoningContent = "think"
	}
	writeJSON(w, http.StatusOK, chatCompletionResponse{
		ID: "chatcmpl-dummy", Object: "chat.completion", Created: time.Now().Unix(), Model: model,
		Choices: []choice{{
			Index: 0, Message: msg, FinishReason: "stop",
		}},
		Usage: *withReasoning(&usage{
			PromptTokens: promptTokens, CompletionTokens: completionTokens,
			TotalTokens: promptTokens + completionTokens,
		}, reasoningTokens),
	})
}

// withReasoning adds reasoning tokens to completion_tokens (OpenAI counts
// them as output) and reports them in completion_tokens_details. With 0 the
// usage is returned unchanged, so the details object stays absent.
func withReasoning(u *usage, reasoningTokens int) *usage {
	if reasoningTokens <= 0 {
		return u
	}
	u.CompletionTokens += reasoningTokens
	u.TotalTokens += reasoningTokens
	u.CompletionTokensDetails = &completionTokensDetails{ReasoningTokens: reasoningTokens}
	return u
}

func promptTokensChat(messages []chatMessage) int {
	contents := make([]vision.Content, 0, len(messages))
	for _, m := range messages {
		contents = append(contents, m.Content)
	}
	return vision.PromptTokensFromParts(contents)
}

// validateChatImages runs Metrum AI strict-media checks on every image_url
// part in the conversation and returns the first failure.
func validateChatImages(messages []chatMessage) error {
	for _, m := range messages {
		for _, p := range m.Content {
			if p.Type != "image_url" {
				continue
			}
			if p.ImageURL == nil {
				return errors.New("image_url part is missing image_url.url")
			}
			if err := media.ValidateImageURL(p.ImageURL.URL); err != nil {
				return err
			}
		}
	}
	return nil
}
