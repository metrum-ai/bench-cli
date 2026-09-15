// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

package sse

import (
	"encoding/json"
	"net/http"
)

// Writer emits OpenAI-style SSE data lines with optional adversarial framing.
type Writer struct {
	W         http.ResponseWriter
	Flusher   http.Flusher
	SplitSSE  bool
	OmitDone  bool
}

// New prepares headers and returns a Writer. Caller must have set status 200.
func New(w http.ResponseWriter, split, omitDone bool) (*Writer, bool) {
	flusher, ok := w.(http.Flusher)
	if !ok {
		http.Error(w, "streaming unsupported", http.StatusInternalServerError)
		return nil, false
	}
	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("Cache-Control", "no-cache")
	w.Header().Set("Connection", "keep-alive")
	return &Writer{W: w, Flusher: flusher, SplitSSE: split, OmitDone: omitDone}, true
}

// Data writes a single SSE data event for v (JSON-encoded) or a raw string payload.
func (s *Writer) Data(v any) error {
	var payload []byte
	switch t := v.(type) {
	case string:
		payload = []byte(t)
	case []byte:
		payload = t
	default:
		b, err := json.Marshal(v)
		if err != nil {
			return err
		}
		payload = b
	}
	if s.SplitSSE {
		if _, err := s.W.Write([]byte("data: ")); err != nil {
			return err
		}
		s.Flusher.Flush()
		if _, err := s.W.Write(payload); err != nil {
			return err
		}
		if _, err := s.W.Write([]byte("\n\n")); err != nil {
			return err
		}
		s.Flusher.Flush()
		return nil
	}
	if _, err := s.W.Write([]byte("data: ")); err != nil {
		return err
	}
	if _, err := s.W.Write(payload); err != nil {
		return err
	}
	if _, err := s.W.Write([]byte("\n\n")); err != nil {
		return err
	}
	s.Flusher.Flush()
	return nil
}

// Done writes data: [DONE] unless OmitDone is set.
func (s *Writer) Done() error {
	if s.OmitDone {
		return nil
	}
	_, err := s.W.Write([]byte("data: [DONE]\n\n"))
	s.Flusher.Flush()
	return err
}

// ErrorObject writes a mid-stream error as an SSE data JSON object.
func (s *Writer) ErrorObject(message string) error {
	return s.Data(map[string]any{
		"error": map[string]any{
			"message": message,
			"type":    "server_error",
			"code":    "mid_stream_error",
		},
	})
}
