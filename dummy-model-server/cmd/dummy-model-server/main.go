// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

package main

import (
	"fmt"
	"log"
	"net/http"
	"os"

	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/config"
	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/server"
)

func main() {
	cfg, err := config.ParseFlags(os.Args[1:])
	if err != nil {
		log.Fatal(err)
	}
	h := server.New(cfg)
	addr := fmt.Sprintf(":%d", cfg.Port)
	log.Printf("dummy-model-server listening on %s compat=%s model=%q latency=%v chunk=%v req/s=%.2f tokens/s=%.2f",
		addr, cfg.Compat, cfg.Model, cfg.Latency, cfg.ChunkInterval, cfg.ReqPerSec, cfg.TokensPerSec)
	if err := http.ListenAndServe(addr, h); err != nil {
		log.Fatal(err)
	}
}
