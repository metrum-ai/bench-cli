// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

package main

import (
	"context"
	"fmt"
	"log"
	"net"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/config"
	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/server"
)

func main() {
	cfg, err := config.ParseFlags(os.Args[1:])
	if err != nil {
		log.Fatal(err)
	}
	// Bind before building the handler so -port 0 picks a free port and
	// cfg.Port holds the real one (image URLs embed it). Tests read the
	// "listening on" line below to learn the port without a reserve/drop race.
	ln, err := net.Listen("tcp", fmt.Sprintf(":%d", cfg.Port))
	if err != nil {
		log.Fatal(err)
	}
	cfg.Port = ln.Addr().(*net.TCPAddr).Port
	addr := fmt.Sprintf(":%d", cfg.Port)
	h := server.New(cfg)
	srv := &http.Server{
		Handler:           h,
		ReadHeaderTimeout: 5 * time.Second,
		ReadTimeout:       30 * time.Second,
		WriteTimeout:      60 * time.Second,
		IdleTimeout:       60 * time.Second,
		MaxHeaderBytes:    1 << 20, // 1 MiB
	}
	log.Printf("dummy-model-server listening on %s compat=%s model=%q latency=%v chunk=%v req/s=%.2f tokens/s=%.2f",
		addr, cfg.Compat, cfg.Model, cfg.Latency, cfg.ChunkInterval, cfg.ReqPerSec, cfg.TokensPerSec)

	go func() {
		if err := srv.Serve(ln); err != nil && err != http.ErrServerClosed {
			log.Fatal(err)
		}
	}()

	stop := make(chan os.Signal, 1)
	signal.Notify(stop, syscall.SIGINT, syscall.SIGTERM)
	<-stop

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := srv.Shutdown(ctx); err != nil {
		log.Printf("shutdown: %v", err)
	}
}
