// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

// Metrum AI strict-media validation tests.
package media_test

import (
	"bytes"
	"encoding/base64"
	"encoding/binary"
	"strings"
	"testing"

	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/media"
	"github.com/metrum-ai/bench-cli/dummy-model-server/internal/media/mediatest"
)

// webpVP8L builds a minimal lossless WebP header with the given dimensions.
func webpVP8L(w, h int) []byte {
	bits := uint32(w-1) | uint32(h-1)<<14
	body := []byte{0x2f, 0, 0, 0, 0, 0}
	binary.LittleEndian.PutUint32(body[1:5], bits)
	return riffWebP("VP8L", body)
}

// webpVP8X builds a minimal extended WebP header with the given canvas size.
func webpVP8X(w, h int) []byte {
	body := make([]byte, 10)
	w--
	h--
	body[4], body[5], body[6] = byte(w), byte(w>>8), byte(w>>16)
	body[7], body[8], body[9] = byte(h), byte(h>>8), byte(h>>16)
	return riffWebP("VP8X", body)
}

// webpVP8 builds a minimal lossy WebP header with the given dimensions.
func webpVP8(w, h int) []byte {
	body := []byte{0, 0, 0, 0x9d, 0x01, 0x2a, 0, 0, 0, 0}
	binary.LittleEndian.PutUint16(body[6:8], uint16(w))
	binary.LittleEndian.PutUint16(body[8:10], uint16(h))
	return riffWebP("VP8 ", body)
}

func riffWebP(fourcc string, body []byte) []byte {
	out := []byte("RIFF\x00\x00\x00\x00WEBP" + fourcc)
	out = binary.LittleEndian.AppendUint32(out, uint32(len(body)))
	out = append(out, body...)
	binary.LittleEndian.PutUint32(out[4:8], uint32(len(out)-8))
	return out
}

func TestValidateImageURL(t *testing.T) {
	png2 := mediatest.PNG(2, 2)
	unpadded := "data:image/png;base64," + base64.RawStdEncoding.EncodeToString(png2)
	wrapped := mediatest.DataURL("image/png", png2)
	wrapped = wrapped[:40] + "\n  " + wrapped[40:]

	cases := []struct {
		name    string
		url     string
		wantErr string // empty means accept
	}{
		{"png 2x2", mediatest.DataURL("image/png", png2), ""},
		{"jpeg 2x2", mediatest.DataURL("image/jpeg", mediatest.JPEG(2, 2)), ""},
		{"gif 2x2", mediatest.DataURL("image/gif", mediatest.GIF(2, 2)), ""},
		{"png 64x32", mediatest.DataURL("image/png", mediatest.PNG(64, 32)), ""},
		{"webp vp8l 2x2", mediatest.DataURL("image/webp", webpVP8L(2, 2)), ""},
		{"webp vp8x 3x5", mediatest.DataURL("image/webp", webpVP8X(3, 5)), ""},
		{"webp vp8 16x16", mediatest.DataURL("image/webp", webpVP8(16, 16)), ""},
		{"unpadded base64", unpadded, ""},
		{"whitespace in payload", wrapped, ""},
		{"uppercase scheme", "DATA:image/png;BASE64," + base64.StdEncoding.EncodeToString(png2), ""},
		{"http url", "http://example.com/cat.png", ""},
		{"https url", "https://example.com/cat.png", ""},
		{"other ref", "file-abc123", ""},
		{"empty string", "", ""},

		{"png 1x1", mediatest.DataURL("image/png", mediatest.PNG(1, 1)), "image too small: 1x1"},
		{"png 1x8", mediatest.DataURL("image/png", mediatest.PNG(1, 8)), "image too small: 1x8"},
		{"webp 1x1", mediatest.DataURL("image/webp", webpVP8L(1, 1)), "image too small: 1x1"},
		{"missing comma", "data:image/png;base64", "missing comma"},
		{"not base64", "data:image/png,raw", "must be base64-encoded"},
		{"missing mime", "data:;base64," + base64.StdEncoding.EncodeToString(png2), "missing a MIME type"},
		{"corrupt base64", "data:image/png;base64,!!!not*base64!!!", "invalid base64 in data: URL"},
		{"empty payload", "data:image/png;base64,", "empty image payload"},
		{"unsupported format", mediatest.DataURL("image/bmp", []byte("BM\x00\x00\x00\x00 not a real image")), "unsupported image format"},
		{"old test stub abc", "data:image/jpeg;base64,abc", "unsupported image format"},
		{"truncated png", mediatest.DataURL("image/png", png2[:20]), "corrupt image header"},
		{"bad webp chunk", mediatest.DataURL("image/webp", riffWebP("ZZZZ", make([]byte, 10))), "unknown WebP chunk"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			err := media.ValidateImageURL(tc.url)
			if tc.wantErr == "" {
				if err != nil {
					t.Fatalf("want accept, got %v", err)
				}
				return
			}
			if err == nil {
				t.Fatalf("want error containing %q, got nil", tc.wantErr)
			}
			if !strings.Contains(err.Error(), tc.wantErr) {
				t.Fatalf("error %q does not contain %q", err, tc.wantErr)
			}
		})
	}
}

func TestValidateAudio(t *testing.T) {
	sine := mediatest.SineWAV()

	// WAV with a LIST chunk between fmt and data, plus an odd-sized pad.
	withList := append([]byte{}, sine[:36]...)
	withList = append(withList, []byte("LIST\x03\x00\x00\x00abc\x00")...)
	withList = append(withList, sine[36:]...)

	// Streaming-style WAV whose data size is a 0xFFFFFFFF placeholder.
	streaming := append([]byte{}, sine...)
	binary.LittleEndian.PutUint32(streaming[40:44], 0xFFFFFFFF)

	badChannels := append([]byte{}, sine...)
	binary.LittleEndian.PutUint16(badChannels[22:24], 9)

	badRate := append([]byte{}, sine...)
	binary.LittleEndian.PutUint32(badRate[24:28], 500)

	badTag := append([]byte{}, sine...)
	binary.LittleEndian.PutUint16(badTag[20:22], 0x0055) // MP3-in-WAV, not accepted

	badBits := append([]byte{}, sine...)
	binary.LittleEndian.PutUint16(badBits[34:36], 12)

	emptyData := append([]byte{}, sine...)
	binary.LittleEndian.PutUint32(emptyData[40:44], 0)

	noData := append([]byte{}, sine...)
	copy(noData[36:40], "junk")

	// ID3v2 tag (synchsafe size 300) followed by valid frames.
	id3 := []byte("ID3\x04\x00\x00\x00\x00\x02\x2c") // 2<<7 | 0x2c = 300
	id3 = append(id3, make([]byte, 300)...)
	id3 = append(id3, []byte("TIT2")...)
	id3Synth := append(id3, mediatest.SynthMP3(3)...)

	// ID3 tag with a non-synchsafe size byte.
	badID3 := append([]byte("ID3\x04\x00\x00\x00\x00\x80\x00"), mediatest.SynthMP3(3)...)

	// A lone frame header followed by non-zero junk that never re-syncs.
	loneHeader := append([]byte{0xFF, 0xFB, 0x90, 0x00}, []byte(strings.Repeat("x", 2000))...)

	// Zero-bodied frames: headers line up but the audio payload is empty.
	zeroBodies := make([]byte, 0, 417*3)
	for i := 0; i < 3; i++ {
		zeroBodies = append(zeroBodies, 0xFF, 0xFB, 0x90, 0x00)
		zeroBodies = append(zeroBodies, make([]byte, 413)...)
	}

	other := func(magic string, at int, fill byte) []byte {
		b := make([]byte, 2048)
		copy(b[at:], magic)
		if fill != 0 {
			for i := 64; i < len(b); i++ {
				b[i] = fill
			}
		}
		return b
	}

	cases := []struct {
		name    string
		data    []byte
		wantErr string
	}{
		{"wav sine 16k mono 16-bit", sine, ""},
		{"wav with LIST chunk", withList, ""},
		{"wav streaming data size", streaming, ""},
		{"mp3 synthesized frames", mediatest.SynthMP3(3), ""},
		{"mp3 with ID3v2 tag", id3Synth, ""},
		{"flac", other("fLaC", 0, 0x11), ""},
		{"ogg", other("OggS", 0, 0x22), ""},
		{"m4a", other("ftypM4A ", 4, 0x33), ""},
		{"webm", other("\x1A\x45\xDF\xA3", 0, 0x44), ""},

		{"repo dummy.mp3 exact bytes", mediatest.RepoDummyMP3(), "too small"},
		{"legacy fake ID3 payload", mediatest.LegacyFakeID3(), "too small"},
		{"short payload", []byte("RIFF....WAVE"), "too small"},
		{"empty", nil, "too small"},
		{"wav silent", mediatest.SilentWAV(), "all zeros after header"},
		{"wav bad channels", badChannels, "channel count 9"},
		{"wav bad sample rate", badRate, "sample rate 500"},
		{"wav bad format tag", badTag, "unsupported WAV format tag"},
		{"wav bad bits", badBits, "bits per sample 12"},
		{"wav empty data chunk", emptyData, "data chunk is empty"},
		{"wav missing data chunk", noData, "missing data chunk"},
		{"mp3 dummy header padded to 2 KiB with zeros", append(mediatest.RepoDummyMP3(), make([]byte, 2048)...), "all zeros after header"},
		{"mp3 lone header", loneHeader, "no two consecutive valid MPEG audio frames"},
		{"mp3 zero frame bodies", zeroBodies, "frame body is all zeros"},
		{"mp3 bad ID3 size", badID3, "not synchsafe"},
		{"fake ID3 padded with text", append(mediatest.LegacyFakeID3(), []byte(strings.Repeat("fake ", 300))...), "no two consecutive valid MPEG audio frames"},
		{"flac all zeros", other("fLaC", 0, 0), "all zeros after container header"},
		{"unknown magic", other("JUNK", 0, 0x55), "unrecognized audio container"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			err := media.ValidateAudio(tc.data)
			if tc.wantErr == "" {
				if err != nil {
					t.Fatalf("want accept, got %v", err)
				}
				return
			}
			if err == nil {
				t.Fatalf("want error containing %q, got nil", tc.wantErr)
			}
			if !strings.Contains(err.Error(), tc.wantErr) {
				t.Fatalf("error %q does not contain %q", err, tc.wantErr)
			}
		})
	}
}

func TestRepoDummyMP3Shape(t *testing.T) {
	b := mediatest.RepoDummyMP3()
	if len(b) != 104 || b[0] != 0xFF || b[1] != 0xFB || b[2] != 0x90 || b[3] != 0x00 {
		t.Fatalf("fixture drifted from test-data/dummy.mp3: len=%d head=% x", len(b), b[:4])
	}
}

// id3Prefix returns an ID3v2.4 tag header with an empty body of n bytes.
func id3Prefix(n int) []byte {
	h := []byte{'I', 'D', '3', 4, 0, 0, byte(n >> 21 & 0x7f), byte(n >> 14 & 0x7f), byte(n >> 7 & 0x7f), byte(n & 0x7f)}
	return append(h, make([]byte, n)...)
}

func TestValidateAudioAcceptsID3PrefixedContainers(t *testing.T) {
	flac := append([]byte("fLaC"), bytes.Repeat([]byte{0x12, 0x34}, 1024)...)
	cases := map[string][]byte{
		"id3+flac": append(id3Prefix(32), flac...),
		"id3+wav":  append(id3Prefix(16), mediatest.SineWAV()...),
	}
	for name, b := range cases {
		if err := media.ValidateAudio(b); err != nil {
			t.Errorf("%s: unexpected reject: %v", name, err)
		}
	}
	silentFLAC := append(id3Prefix(32), append([]byte("fLaC"), make([]byte, 2048)...)...)
	if err := media.ValidateAudio(silentFLAC); err == nil {
		t.Error("id3+all-zero flac: expected reject")
	}
}

func TestValidateAudioToleratesMissingWAVPadByte(t *testing.T) {
	wav := mediatest.SineWAV()
	// Insert an odd-sized LIST chunk without its pad byte after fmt.
	list := append([]byte("LIST"), 3, 0, 0, 0, 'a', 'b', 'c')
	fmtEnd := 12 + 8 + 16
	b := append(append(append([]byte{}, wav[:fmtEnd]...), list...), wav[fmtEnd:]...)
	if err := media.ValidateAudio(b); err != nil {
		t.Fatalf("unexpected reject: %v", err)
	}
}
