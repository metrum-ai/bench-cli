// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

package limiter_test

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/config"
	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/server"
)

func TestReqPerSecReturns429(t *testing.T) {
	h := server.New(&config.Config{
		Model: "dummy", Compat: config.CompatOpenAI, ReqPerSec: 1,
		IncludeUsage: true, MaxImages: 4, AllowAnyModel: true,
	})
	body := `{"model":"dummy","messages":[{"role":"user","content":"Hi"}],"max_tokens":1,"stream":false}`
	codes := make([]int, 0, 2)
	for i := 0; i < 2; i++ {
		req := httptest.NewRequest(http.MethodPost, "/v1/chat/completions", strings.NewReader(body))
		rec := httptest.NewRecorder()
		h.ServeHTTP(rec, req)
		codes = append(codes, rec.Code)
		if rec.Code == http.StatusTooManyRequests {
			if rec.Header().Get("Retry-After") == "" {
				t.Fatal("missing Retry-After")
			}
		}
	}
	if codes[0] != http.StatusOK {
		t.Fatalf("first=%d want 200", codes[0])
	}
	if codes[1] != http.StatusTooManyRequests {
		t.Fatalf("second=%d want 429", codes[1])
	}
}
