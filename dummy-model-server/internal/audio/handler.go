// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

package audio

import (
	"encoding/json"
	"fmt"
	"io"
	"log"
	"math/rand"
	"net/http"
	"strings"
	"time"

	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/config"
	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/media"
)

const defaultText = "dummy transcription"

// invalidAudioMessage matches the error text vLLM returns for undecodable
// audio uploads, so clients see the same message under -strict-media.
const invalidAudioMessage = "Invalid or unsupported audio file"

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
	if h.Cfg.StrictMedia {
		data, err := io.ReadAll(file)
		_ = file.Close()
		if err == nil {
			err = media.ValidateAudio(data)
		}
		if err != nil {
			if h.Cfg.LogRequests {
				log.Printf("strict-media: rejected audio upload: %v", err)
			}
			writeInvalidAudio(w)
			return
		}
	} else {
		_ = file.Close()
	}

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

// writeInvalidAudio emits the Metrum AI strict-media rejection as an
// OpenAI-style JSON error with HTTP 400.
func writeInvalidAudio(w http.ResponseWriter) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusBadRequest)
	_ = json.NewEncoder(w).Encode(map[string]any{
		"error": map[string]any{
			"message": invalidAudioMessage,
			"type":    "invalid_request_error",
			"code":    http.StatusBadRequest,
		},
	})
}
