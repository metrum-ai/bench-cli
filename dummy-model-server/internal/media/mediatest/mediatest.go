// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

// Package mediatest is the Metrum AI fixture builder for strict-media tests.
// It synthesizes small valid and invalid image and audio payloads in memory so
// tests never depend on binary files checked into the repo.
package mediatest

import (
	"bytes"
	"encoding/base64"
	"encoding/binary"
	"image"
	"image/color"
	"image/gif"
	"image/jpeg"
	"image/png"
	"math"
)

func testImage(w, h int) *image.RGBA {
	img := image.NewRGBA(image.Rect(0, 0, w, h))
	for y := 0; y < h; y++ {
		for x := 0; x < w; x++ {
			img.Set(x, y, color.RGBA{R: uint8(40 * x), G: uint8(80 * y), B: 200, A: 255})
		}
	}
	return img
}

// PNG returns an encoded w x h PNG.
func PNG(w, h int) []byte {
	var buf bytes.Buffer
	_ = png.Encode(&buf, testImage(w, h))
	return buf.Bytes()
}

// JPEG returns an encoded w x h JPEG.
func JPEG(w, h int) []byte {
	var buf bytes.Buffer
	_ = jpeg.Encode(&buf, testImage(w, h), nil)
	return buf.Bytes()
}

// GIF returns an encoded w x h GIF.
func GIF(w, h int) []byte {
	var buf bytes.Buffer
	_ = gif.Encode(&buf, testImage(w, h), nil)
	return buf.Bytes()
}

// DataURL wraps raw bytes as data:<mime>;base64,<payload>.
func DataURL(mime string, raw []byte) string {
	return "data:" + mime + ";base64," + base64.StdEncoding.EncodeToString(raw)
}

// WAV builds a canonical 44-byte-header PCM WAV around samples.
func WAV(sampleRate uint32, channels, bits uint16, samples []byte) []byte {
	var buf bytes.Buffer
	blockAlign := channels * bits / 8
	buf.WriteString("RIFF")
	_ = binary.Write(&buf, binary.LittleEndian, uint32(36+len(samples)))
	buf.WriteString("WAVE")
	buf.WriteString("fmt ")
	_ = binary.Write(&buf, binary.LittleEndian, uint32(16))
	_ = binary.Write(&buf, binary.LittleEndian, uint16(1)) // PCM
	_ = binary.Write(&buf, binary.LittleEndian, channels)
	_ = binary.Write(&buf, binary.LittleEndian, sampleRate)
	_ = binary.Write(&buf, binary.LittleEndian, sampleRate*uint32(blockAlign))
	_ = binary.Write(&buf, binary.LittleEndian, blockAlign)
	_ = binary.Write(&buf, binary.LittleEndian, bits)
	buf.WriteString("data")
	_ = binary.Write(&buf, binary.LittleEndian, uint32(len(samples)))
	buf.Write(samples)
	return buf.Bytes()
}

// SineWAV is 1 s of a 440 Hz tone, 16 kHz mono 16-bit PCM.
func SineWAV() []byte {
	const sr = 16000
	samples := make([]byte, sr*2)
	for i := 0; i < sr; i++ {
		v := int16(0.5 * math.MaxInt16 * math.Sin(2*math.Pi*440*float64(i)/sr))
		binary.LittleEndian.PutUint16(samples[2*i:], uint16(v))
	}
	return WAV(sr, 1, 16, samples)
}

// SilentWAV is 1 s of digital silence, 16 kHz mono 16-bit PCM.
func SilentWAV() []byte {
	return WAV(16000, 1, 16, make([]byte, 32000))
}

// RepoDummyMP3 reproduces the legacy test-data/dummy.mp3 placeholder: one
// MPEG-1 Layer III header (0xFF 0xFB 0x90 0x00) followed by 100 zero bytes.
func RepoDummyMP3() []byte {
	return append([]byte{0xFF, 0xFB, 0x90, 0x00}, make([]byte, 100)...)
}

// LegacyFakeID3 is the old Rust test payload: an ID3v2.4 header with zero
// size followed by ASCII text and no MPEG frames.
func LegacyFakeID3() []byte {
	return []byte("ID3\x04\x00\x00\x00\x00\x00\x00fake mp3 payload")
}

// SynthMP3 builds n back-to-back MPEG-1 Layer III 128 kbps 44.1 kHz frames
// (417 bytes each, no padding, no CRC) with deterministic non-zero bodies.
// It is not decodable audio, but it has the frame structure decoders sync on.
func SynthMP3(n int) []byte {
	const frameLen = 417
	out := make([]byte, 0, n*frameLen)
	seed := uint32(0x9E3779B9)
	for f := 0; f < n; f++ {
		out = append(out, 0xFF, 0xFB, 0x90, 0x00)
		for i := 4; i < frameLen; i++ {
			seed = seed*1664525 + 1013904223
			out = append(out, byte(seed>>24)|1)
		}
	}
	return out
}
