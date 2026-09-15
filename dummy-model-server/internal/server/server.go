// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

package server

import (
	"log"
	"net/http"

	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/audio"
	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/config"
	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/images"
	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/limiter"
	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/openai"
)

// New builds the HTTP handler with all modality routes.
func New(cfg *config.Config) http.Handler {
	lim := limiter.New(cfg.ReqPerSec, cfg.TokensPerSec, cfg.MaxConcurrency)
	oai := &openai.Handler{Cfg: cfg, Limiter: lim}
	asr := &audio.Handler{Cfg: cfg}
	img := &images.Handler{Cfg: cfg}

	mux := http.NewServeMux()
	mux.HandleFunc("GET /health", health)
	mux.HandleFunc("GET /healthz", health)
	mux.HandleFunc("GET /ready", health)
	mux.HandleFunc("GET /v1/models", oai.Models)
	mux.HandleFunc("POST /v1/chat/completions", oai.ChatCompletions)
	mux.HandleFunc("POST /v1/completions", oai.Completions)
	mux.HandleFunc("POST /v1/audio/transcriptions", asr.Transcriptions)
	mux.HandleFunc("POST /v1/images/generations", img.Generations)

	var h http.Handler = mux
	h = lim.Middleware(h)
	if cfg.LogRequests {
		h = requestLog(h)
	}
	return h
}

func health(w http.ResponseWriter, _ *http.Request) {
	w.WriteHeader(http.StatusOK)
}

func requestLog(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		size := r.ContentLength
		if size < 0 {
			size = 0
		}
		log.Printf("%s %s ContentLength=%d", r.Method, r.URL.Path, size)
		next.ServeHTTP(w, r)
	})
}
