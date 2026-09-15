// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

package images_test

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"image/png"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/config"
	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/server"
)

func TestImageGenerationsB64(t *testing.T) {
	h := server.New(&config.Config{
		Model: "dummy", Compat: config.CompatOpenAI, ImageSize: "32x32",
		MaxImages: 4, AllowAnyModel: true, Seed: 42,
	})
	body := `{"model":"dummy","prompt":"a horse","n":2,"size":"32x32","response_format":"b64_json"}`
	req := httptest.NewRequest(http.MethodPost, "/v1/images/generations", strings.NewReader(body))
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("status %d %s", rec.Code, rec.Body.String())
	}
	var resp struct {
		Data []struct {
			B64JSON string `json:"b64_json"`
		} `json:"data"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &resp); err != nil {
		t.Fatal(err)
	}
	if len(resp.Data) != 2 {
		t.Fatalf("n=%d", len(resp.Data))
	}
	raw, err := base64.StdEncoding.DecodeString(resp.Data[0].B64JSON)
	if err != nil {
		t.Fatal(err)
	}
	img, err := png.Decode(bytes.NewReader(raw))
	if err != nil {
		t.Fatal(err)
	}
	if img.Bounds().Dx() != 32 || img.Bounds().Dy() != 32 {
		t.Fatalf("dims %dx%d", img.Bounds().Dx(), img.Bounds().Dy())
	}
}
