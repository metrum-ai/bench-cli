// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

package limiter

import (
	"net/http"
	"time"

	"golang.org/x/time/rate"
)

// Limits enforces req/s, tokens/s, and max concurrency with HTTP 429.
type Limits struct {
	req     *rate.Limiter
	tokens  *rate.Limiter
	maxConc int
	sem     chan struct{}
}

// New builds limiters from rates. Zero/negative rates disable that limiter.
func New(reqPerSec, tokensPerSec float64, maxConcurrency int) *Limits {
	l := &Limits{maxConc: maxConcurrency}
	if reqPerSec > 0 {
		l.req = rate.NewLimiter(rate.Limit(reqPerSec), max(1, int(reqPerSec)))
	}
	if tokensPerSec > 0 {
		burst := max(1, int(tokensPerSec))
		l.tokens = rate.NewLimiter(rate.Limit(tokensPerSec), burst)
	}
	if maxConcurrency > 0 {
		l.sem = make(chan struct{}, maxConcurrency)
	}
	return l
}

// AllowRequest checks request and optional token budget. On failure sets Retry-After.
func (l *Limits) AllowRequest(w http.ResponseWriter, tokenCost int) bool {
	if l == nil {
		return true
	}
	if l.req != nil && !l.req.Allow() {
		write429(w)
		return false
	}
	if l.tokens != nil && tokenCost > 0 {
		if !l.tokens.AllowN(time.Now(), tokenCost) {
			write429(w)
			return false
		}
	}
	return true
}

// AcquireConcurrency blocks a concurrency slot; returns false and 429 if full.
func (l *Limits) AcquireConcurrency(w http.ResponseWriter) bool {
	if l == nil || l.sem == nil {
		return true
	}
	select {
	case l.sem <- struct{}{}:
		return true
	default:
		write429(w)
		return false
	}
}

// ReleaseConcurrency frees a concurrency slot.
func (l *Limits) ReleaseConcurrency() {
	if l == nil || l.sem == nil {
		return
	}
	select {
	case <-l.sem:
	default:
	}
}

// Middleware wraps handlers with concurrency + request rate limits.
// Token cost is applied by handlers via AllowRequest when known.
func (l *Limits) Middleware(next http.Handler) http.Handler {
	if l == nil {
		return next
	}
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if isHealth(r.URL.Path) {
			next.ServeHTTP(w, r)
			return
		}
		if !l.AcquireConcurrency(w) {
			return
		}
		defer l.ReleaseConcurrency()
		if l.req != nil && !l.req.Allow() {
			write429(w)
			return
		}
		next.ServeHTTP(w, r)
	})
}

// AllowTokens spends token budget (e.g. completion tokens). Returns false on 429.
func (l *Limits) AllowTokens(w http.ResponseWriter, n int) bool {
	if l == nil || l.tokens == nil || n <= 0 {
		return true
	}
	if !l.tokens.AllowN(time.Now(), n) {
		write429(w)
		return false
	}
	return true
}

func write429(w http.ResponseWriter) {
	w.Header().Set("Retry-After", "1")
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusTooManyRequests)
	_, _ = w.Write([]byte(`{"error":{"message":"rate limit exceeded","type":"rate_limit_error","code":"rate_limit_exceeded"}}`))
}

func isHealth(path string) bool {
	return path == "/health" || path == "/healthz" || path == "/ready"
}

func max(a, b int) int {
	if a > b {
		return a
	}
	return b
}
