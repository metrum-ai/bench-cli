// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

package audio_test

import (
	"bytes"
	"encoding/json"
	"mime/multipart"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/config"
	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/server"
)

func TestTranscriptionJSONAndText(t *testing.T) {
	h := server.New(&config.Config{Model: "whisper-dummy", Compat: config.CompatOpenAI, MaxImages: 4, AllowAnyModel: true})

	for _, format := range []string{"json", "text"} {
		var buf bytes.Buffer
		mw := multipart.NewWriter(&buf)
		fw, err := mw.CreateFormFile("file", "a.wav")
		if err != nil {
			t.Fatal(err)
		}
		_, _ = fw.Write([]byte("RIFF....WAVE"))
		_ = mw.WriteField("model", "whisper-dummy")
		_ = mw.WriteField("response_format", format)
		_ = mw.Close()

		req := httptest.NewRequest(http.MethodPost, "/v1/audio/transcriptions", &buf)
		req.Header.Set("Content-Type", mw.FormDataContentType())
		rec := httptest.NewRecorder()
		h.ServeHTTP(rec, req)
		if rec.Code != http.StatusOK {
			t.Fatalf("%s: status %d %s", format, rec.Code, rec.Body.String())
		}
		if format == "json" {
			var out map[string]any
			if err := json.Unmarshal(rec.Body.Bytes(), &out); err != nil {
				t.Fatal(err)
			}
			if out["text"] != "dummy transcription" {
				t.Fatalf("text=%v", out["text"])
			}
		} else if rec.Body.String() != "dummy transcription" {
			t.Fatalf("text body=%q", rec.Body.String())
		}
	}
}
