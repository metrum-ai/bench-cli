// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

// Package media is the Metrum AI strict-media validation for the dummy server.
//
// When the dummy-model-server runs with -strict-media, the chat and audio
// handlers call into this package so that payloads a real inference server
// (vLLM, SGLang, OpenAI) would reject are also rejected here. The goal is to
// catch benchmark clients that ship placeholder media (1x1 images, zero-filled
// "MP3" stubs, truncated WAVs) before those numbers get published.
//
// The checks are deliberately lightweight and use only the Go standard
// library:
//
//   - Images: data: URLs are base64-decoded and their header is parsed with
//     image.DecodeConfig (PNG, JPEG, GIF) or a small built-in WebP header
//     reader (VP8, VP8L, VP8X). Only dimensions are read; pixel data is not
//     decoded. http(s) and other non-data references are not fetched.
//   - Audio: container magic is sniffed, WAV RIFF chunks are walked, and MP3
//     frame headers are parsed far enough to prove two consecutive frames
//     exist. The full audio stream is never decoded.
//
// Metrum AI maintains this package as part of the metrum-ai-bench-cli test
// harness; it is not a general-purpose media validator.
package media
