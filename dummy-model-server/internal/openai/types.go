// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

package openai

import (
	"encoding/json"
	"io"
	"net/http"

	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/vision"
)

type chatRequest struct {
	Model         string          `json:"model"`
	Messages      []chatMessage   `json:"messages"`
	MaxTokens     int             `json:"max_tokens"`
	Stream        bool            `json:"stream"`
	StreamOptions *streamOptions  `json:"stream_options"`
	IgnoreEOS     bool            `json:"ignore_eos"`
	Temperature   *float64        `json:"temperature"`
	Seed          *int64          `json:"seed"`
	TopP          *float64        `json:"top_p"`
	Extra         json.RawMessage `json:"-"`
}

type streamOptions struct {
	IncludeUsage *bool `json:"include_usage"`
}

type chatMessage struct {
	Role    string         `json:"role"`
	Content vision.Content `json:"content"`
}

type completionRequest struct {
	Model     string `json:"model"`
	Prompt    string `json:"prompt"`
	MaxTokens int    `json:"max_tokens"`
	Stream    bool   `json:"stream"`
	IgnoreEOS bool   `json:"ignore_eos"`
}

type chatCompletionResponse struct {
	ID      string   `json:"id"`
	Object  string   `json:"object"`
	Created int64    `json:"created"`
	Model   string   `json:"model"`
	Choices []choice `json:"choices"`
	Usage   usage    `json:"usage"`
}

type choice struct {
	Index        int     `json:"index"`
	Message      message `json:"message"`
	FinishReason string  `json:"finish_reason"`
}

type message struct {
	Role    string `json:"role"`
	Content string `json:"content"`
}

type usage struct {
	PromptTokens     int `json:"prompt_tokens"`
	CompletionTokens int `json:"completion_tokens"`
	TotalTokens      int `json:"total_tokens"`
}

type streamChunk struct {
	ID      string         `json:"id"`
	Object  string         `json:"object"`
	Created int64          `json:"created,omitempty"`
	Model   string         `json:"model"`
	Choices []streamChoice `json:"choices"`
	Usage   *usage         `json:"usage,omitempty"`
}

type streamChoice struct {
	Index        int         `json:"index"`
	Delta        streamDelta `json:"delta"`
	FinishReason *string     `json:"finish_reason"`
}

type streamDelta struct {
	Role             string `json:"role,omitempty"`
	Content          string `json:"content,omitempty"`
	ReasoningContent string `json:"reasoning_content,omitempty"`
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(v)
}

func writeErr(w http.ResponseWriter, status int, msg string) {
	writeJSON(w, status, map[string]any{
		"error": map[string]any{"message": msg, "type": "invalid_request_error"},
	})
}

func readBody(r *http.Request) ([]byte, error) {
	defer r.Body.Close()
	return io.ReadAll(r.Body)
}
