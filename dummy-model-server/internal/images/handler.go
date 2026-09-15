// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

package images

import (
	"bytes"
	"crypto/sha256"
	"encoding/base64"
	"encoding/binary"
	"encoding/json"
	"fmt"
	"image"
	"image/color"
	"image/draw"
	"image/png"
	"math/rand"
	"net/http"
	"regexp"
	"strconv"
	"strings"
	"time"

	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/config"
)

var sizePattern = regexp.MustCompile(`^([1-9][0-9]{0,4})x([1-9][0-9]{0,4})$`)

// Handler serves POST /v1/images/generations.
type Handler struct {
	Cfg *config.Config
}

type request struct {
	Model          string `json:"model"`
	Prompt         string `json:"prompt"`
	N              *int   `json:"n"`
	Size           string `json:"size"`
	ResponseFormat string `json:"response_format"`
	Seed           *int64 `json:"seed"`
}

// Generations returns deterministic PNG bytes as b64_json (or url stubs).
func (h *Handler) Generations(w http.ResponseWriter, r *http.Request) {
	if h.Cfg.ErrorRate > 0 && rand.Float64() < h.Cfg.ErrorRate {
		http.Error(w, "service unavailable", http.StatusServiceUnavailable)
		return
	}
	var req request
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		writeErr(w, "invalid request body")
		return
	}
	if h.Cfg.Latency > 0 {
		time.Sleep(h.Cfg.Latency)
	}
	if strings.TrimSpace(req.Prompt) == "" {
		writeErr(w, "prompt is required")
		return
	}
	model := req.Model
	if model == "" {
		model = h.Cfg.Model
	}
	if !h.Cfg.AllowAnyModel && model != h.Cfg.Model {
		writeErr(w, fmt.Sprintf("model %q not found", model))
		return
	}
	n := 1
	if req.N != nil {
		n = *req.N
	}
	if n <= 0 || n > h.Cfg.MaxImages {
		writeErr(w, fmt.Sprintf("n must be 1..%d", h.Cfg.MaxImages))
		return
	}
	size := req.Size
	if size == "" {
		size = h.Cfg.ImageSize
	}
	width, height, ok := parseSize(size)
	if !ok {
		writeErr(w, "size must be WxH")
		return
	}
	format := req.ResponseFormat
	if format == "" {
		format = "b64_json"
	}
	seed := h.Cfg.Seed
	if req.Seed != nil {
		seed = *req.Seed
	}

	items := make([]map[string]any, 0, n)
	for i := 0; i < n; i++ {
		if format == "url" {
			u := fmt.Sprintf("http://localhost:%d/mock-images/%d-%d.png", h.Cfg.Port, seed, i)
			items = append(items, map[string]any{"url": u, "revised_prompt": req.Prompt})
			continue
		}
		pngBytes, err := deterministicPNG(model, req.Prompt, seed, size, i, width, height)
		if err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}
		items = append(items, map[string]any{
			"b64_json":       base64.StdEncoding.EncodeToString(pngBytes),
			"revised_prompt": req.Prompt,
		})
	}
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]any{
		"created": time.Now().Unix(),
		"data":    items,
	})
}

func parseSize(size string) (int, int, bool) {
	m := sizePattern.FindStringSubmatch(size)
	if m == nil {
		return 0, 0, false
	}
	w, errW := strconv.Atoi(m[1])
	h, errH := strconv.Atoi(m[2])
	if errW != nil || errH != nil || w <= 0 || h <= 0 {
		return 0, 0, false
	}
	return w, h, true
}

func deterministicPNG(model, prompt string, seed int64, size string, index, width, height int) ([]byte, error) {
	hash := sha256.Sum256([]byte(fmt.Sprintf("%s\x00%s\x00%d\x00%s", model, prompt, seed, size)))
	img := image.NewRGBA(image.Rect(0, 0, width, height))
	bg := color.RGBA{R: hash[0], G: hash[1], B: hash[2], A: 255}
	draw.Draw(img, img.Bounds(), &image.Uniform{C: bg}, image.Point{}, draw.Src)
	rng := rand.New(rand.NewSource(int64(binary.BigEndian.Uint64(hash[:8])) + int64(index*7919)))
	for i := 0; i < 16; i++ {
		x0 := rng.Intn(max(width, 1))
		y0 := rng.Intn(max(height, 1))
		x1 := min(width, x0+1+rng.Intn(max(width/3, 1)))
		y1 := min(height, y0+1+rng.Intn(max(height/3, 1)))
		c := color.RGBA{R: byte(rng.Intn(256)), G: byte(rng.Intn(256)), B: byte(rng.Intn(256)), A: 200}
		draw.Draw(img, image.Rect(x0, y0, x1, y1), &image.Uniform{C: c}, image.Point{}, draw.Over)
	}
	var buf bytes.Buffer
	if err := png.Encode(&buf, img); err != nil {
		return nil, err
	}
	return buf.Bytes(), nil
}

func writeErr(w http.ResponseWriter, msg string) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusBadRequest)
	_ = json.NewEncoder(w).Encode(map[string]any{
		"error": map[string]any{"message": msg, "type": "invalid_request_error"},
	})
}

func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}

func max(a, b int) int {
	if a > b {
		return a
	}
	return b
}
