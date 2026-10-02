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
	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/media/mediatest"
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

// postAudio sends a multipart transcription request with the given file bytes.
func postAudio(t *testing.T, h http.Handler, filename string, data []byte) *httptest.ResponseRecorder {
	t.Helper()
	var buf bytes.Buffer
	mw := multipart.NewWriter(&buf)
	fw, err := mw.CreateFormFile("file", filename)
	if err != nil {
		t.Fatal(err)
	}
	_, _ = fw.Write(data)
	_ = mw.WriteField("model", "whisper-dummy")
	_ = mw.Close()
	req := httptest.NewRequest(http.MethodPost, "/v1/audio/transcriptions", &buf)
	req.Header.Set("Content-Type", mw.FormDataContentType())
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return rec
}

func audioCfg(strict bool) *config.Config {
	return &config.Config{
		Model: "whisper-dummy", Compat: config.CompatOpenAI, MaxImages: 4,
		AllowAnyModel: true, StrictMedia: strict,
	}
}

// badAudio are payloads the Metrum AI strict-media mode must reject.
var badAudio = []struct {
	name string
	file string
	data []byte
}{
	{"repo dummy.mp3", "dummy.mp3", mediatest.RepoDummyMP3()},
	{"legacy fake ID3", "a.mp3", mediatest.LegacyFakeID3()},
	{"silent wav", "silent.wav", mediatest.SilentWAV()},
	{"riff stub", "a.wav", []byte("RIFF....WAVE")},
	{"zero-padded mp3 header", "pad.mp3", append(mediatest.RepoDummyMP3(), make([]byte, 4096)...)},
}

func TestTranscriptionStrictRejects(t *testing.T) {
	h := server.New(audioCfg(true))
	for _, tc := range badAudio {
		t.Run(tc.name, func(t *testing.T) {
			rec := postAudio(t, h, tc.file, tc.data)
			if rec.Code != http.StatusBadRequest {
				t.Fatalf("status %d want 400; body %s", rec.Code, rec.Body.String())
			}
			if ct := rec.Header().Get("Content-Type"); ct != "application/json" {
				t.Fatalf("content-type %q want application/json", ct)
			}
			var out struct {
				Error struct {
					Message string `json:"message"`
					Type    string `json:"type"`
					Code    int    `json:"code"`
				} `json:"error"`
			}
			if err := json.Unmarshal(rec.Body.Bytes(), &out); err != nil {
				t.Fatalf("error body not JSON: %v %s", err, rec.Body.String())
			}
			if out.Error.Message != "Invalid or unsupported audio file" {
				t.Fatalf("message %q", out.Error.Message)
			}
			if out.Error.Code != 400 || out.Error.Type != "invalid_request_error" {
				t.Fatalf("error shape %+v", out.Error)
			}
		})
	}
}

func TestTranscriptionStrictAccepts(t *testing.T) {
	h := server.New(audioCfg(true))
	for name, data := range map[string][]byte{
		"tone.wav":  mediatest.SineWAV(),
		"synth.mp3": mediatest.SynthMP3(4),
	} {
		rec := postAudio(t, h, name, data)
		if rec.Code != http.StatusOK {
			t.Fatalf("%s: status %d %s", name, rec.Code, rec.Body.String())
		}
		var out map[string]any
		if err := json.Unmarshal(rec.Body.Bytes(), &out); err != nil {
			t.Fatal(err)
		}
		if out["text"] != "dummy transcription" {
			t.Fatalf("%s: text=%v", name, out["text"])
		}
	}
}

func TestTranscriptionNonStrictAcceptsBadPayloads(t *testing.T) {
	h := server.New(audioCfg(false))
	for _, tc := range badAudio {
		rec := postAudio(t, h, tc.file, tc.data)
		if rec.Code != http.StatusOK {
			t.Fatalf("%s: non-strict must accept, got %d %s", tc.name, rec.Code, rec.Body.String())
		}
	}
}
