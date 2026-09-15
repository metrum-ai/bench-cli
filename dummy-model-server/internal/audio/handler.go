// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

package audio

import (
	"encoding/json"
	"fmt"
	"math/rand"
	"net/http"
	"strings"
	"time"

	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/config"
)

const defaultText = "dummy transcription"

// Handler serves POST /v1/audio/transcriptions.
type Handler struct {
	Cfg *config.Config
}

// Transcriptions handles multipart ASR requests.
func (h *Handler) Transcriptions(w http.ResponseWriter, r *http.Request) {
	if h.Cfg.ErrorRate > 0 && rand.Float64() < h.Cfg.ErrorRate {
		http.Error(w, "service unavailable", http.StatusServiceUnavailable)
		return
	}
	if h.Cfg.Latency > 0 {
		time.Sleep(h.Cfg.Latency)
	}
	const maxMem = 32 << 20
	if err := r.ParseMultipartForm(maxMem); err != nil {
		http.Error(w, `{"error":{"message":"invalid multipart form"}}`, http.StatusBadRequest)
		return
	}
	file, _, err := r.FormFile("file")
	if err != nil {
		http.Error(w, `{"error":{"message":"file is required"}}`, http.StatusBadRequest)
		return
	}
	_ = file.Close()

	model := strings.TrimSpace(r.FormValue("model"))
	if model == "" {
		model = h.Cfg.Model
	}
	_ = model
	format := strings.TrimSpace(r.FormValue("response_format"))
	if format == "" {
		format = "json"
	}

	switch format {
	case "json":
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(map[string]any{
			"text":           defaultText,
			"transcription":  defaultText,
			"inference_time": 0.1,
		})
	case "text":
		w.Header().Set("Content-Type", "text/plain; charset=utf-8")
		_, _ = w.Write([]byte(defaultText))
	case "verbose_json":
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(map[string]any{
			"text": defaultText, "duration": 1.0, "segments": []any{},
		})
	default:
		http.Error(w, fmt.Sprintf(`{"error":{"message":"unsupported response_format %q"}}`, format), http.StatusBadRequest)
	}
}
