// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

package media

import (
	"bytes"
	"encoding/base64"
	"encoding/binary"
	"errors"
	"fmt"
	"image"
	_ "image/gif"  // register GIF for image.DecodeConfig
	_ "image/jpeg" // register JPEG for image.DecodeConfig
	_ "image/png"  // register PNG for image.DecodeConfig
	"strings"
)

// MinImageDim is the smallest accepted width and height in pixels. Vision
// encoders in real servers (for example Qwen2-VL and LLaVA preprocessors)
// fail or produce degenerate patches for 1-pixel inputs.
const MinImageDim = 2

// ValidateImageURL checks one OpenAI chat image_url reference.
//
// For data: URLs it requires the form data:<mime>;base64,<payload>, decodes
// the payload (whitespace is ignored; standard padded base64 is tried first,
// then unpadded), parses the image header, and rejects images narrower or
// shorter than MinImageDim. Supported formats are PNG, JPEG, GIF, and WebP.
//
// Any other reference (http, https, file ids, and so on) returns nil because
// resolving it is the client's or the real server's job, not the dummy
// server's. Part of the Metrum AI strict-media checks.
func ValidateImageURL(url string) error {
	if len(url) < 5 || !strings.EqualFold(url[:5], "data:") {
		return nil
	}
	rest := url[5:]
	comma := strings.IndexByte(rest, ',')
	if comma < 0 {
		return errors.New("malformed data: URL: missing comma before payload")
	}
	meta, payload := rest[:comma], rest[comma+1:]

	semi := strings.IndexByte(meta, ';')
	if semi < 0 || !strings.EqualFold(strings.TrimSpace(meta[semi+1:]), "base64") {
		return errors.New("data: URL must be base64-encoded (data:<mime>;base64,<payload>)")
	}
	if strings.TrimSpace(meta[:semi]) == "" {
		return errors.New("data: URL is missing a MIME type")
	}

	raw, err := decodeBase64(payload)
	if err != nil {
		return errors.New("invalid base64 in data: URL")
	}
	if len(raw) == 0 {
		return errors.New("empty image payload in data: URL")
	}

	w, h, err := imageDims(raw)
	if err != nil {
		return err
	}
	if w < MinImageDim || h < MinImageDim {
		return fmt.Errorf("image too small: %dx%d (minimum %dx%d)", w, h, MinImageDim, MinImageDim)
	}
	return nil
}

// decodeBase64 strips all whitespace and decodes with padded std encoding,
// falling back to unpadded std encoding for clients that drop '=' padding.
func decodeBase64(s string) ([]byte, error) {
	clean := strings.Join(strings.Fields(s), "")
	if b, err := base64.StdEncoding.DecodeString(clean); err == nil {
		return b, nil
	}
	return base64.RawStdEncoding.DecodeString(strings.TrimRight(clean, "="))
}

// imageDims returns width and height from the image header.
func imageDims(raw []byte) (int, int, error) {
	if isWebP(raw) {
		return webpDims(raw)
	}
	cfg, _, err := image.DecodeConfig(bytes.NewReader(raw))
	if err != nil {
		if errors.Is(err, image.ErrFormat) {
			return 0, 0, errors.New("unsupported image format (expected PNG, JPEG, GIF, or WebP)")
		}
		return 0, 0, fmt.Errorf("corrupt image header: %v", err)
	}
	return cfg.Width, cfg.Height, nil
}

func isWebP(b []byte) bool {
	return len(b) >= 12 && string(b[0:4]) == "RIFF" && string(b[8:12]) == "WEBP"
}

// webpDims reads canvas dimensions from the first WebP chunk. It handles the
// three chunk types the WebP container spec allows first: VP8 (lossy), VP8L
// (lossless), and VP8X (extended). This avoids a golang.org/x/image
// dependency since only the header is needed.
func webpDims(b []byte) (int, int, error) {
	if len(b) < 20 {
		return 0, 0, errors.New("corrupt image header: truncated WebP")
	}
	fourcc := string(b[12:16])
	size := int(binary.LittleEndian.Uint32(b[16:20]))
	body := b[20:]
	if size > len(body) {
		return 0, 0, errors.New("corrupt image header: truncated WebP chunk")
	}
	body = body[:size]
	switch fourcc {
	case "VP8 ":
		// 3-byte frame tag, 3-byte start code 9d 01 2a, then 14-bit dims.
		if len(body) < 10 || body[3] != 0x9d || body[4] != 0x01 || body[5] != 0x2a {
			return 0, 0, errors.New("corrupt image header: bad VP8 start code")
		}
		w := int(binary.LittleEndian.Uint16(body[6:8]) & 0x3fff)
		h := int(binary.LittleEndian.Uint16(body[8:10]) & 0x3fff)
		return w, h, nil
	case "VP8L":
		// Signature 0x2f, then 14 bits (width-1) and 14 bits (height-1).
		if len(body) < 5 || body[0] != 0x2f {
			return 0, 0, errors.New("corrupt image header: bad VP8L signature")
		}
		bits := binary.LittleEndian.Uint32(body[1:5])
		w := int(bits&0x3fff) + 1
		h := int((bits>>14)&0x3fff) + 1
		return w, h, nil
	case "VP8X":
		// 4 bytes flags/reserved, then 24-bit (width-1) and 24-bit (height-1).
		if len(body) < 10 {
			return 0, 0, errors.New("corrupt image header: truncated VP8X")
		}
		w := int(uint32(body[4])|uint32(body[5])<<8|uint32(body[6])<<16) + 1
		h := int(uint32(body[7])|uint32(body[8])<<8|uint32(body[9])<<16) + 1
		return w, h, nil
	default:
		return 0, 0, fmt.Errorf("corrupt image header: unknown WebP chunk %q", fourcc)
	}
}
