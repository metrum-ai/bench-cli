// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

package media

import (
	"encoding/binary"
	"errors"
	"fmt"
)

// MinAudioBytes is the smallest accepted audio upload. Anything shorter
// cannot hold a meaningful amount of speech in any common codec, and real
// servers (vLLM via librosa/soundfile, faster-whisper via ffmpeg) fail to
// decode the stub payloads that benchmark clients tend to ship.
const MinAudioBytes = 1024

const (
	wavHeaderBytes = 44 // canonical RIFF/WAVE header with fmt and data chunk headers
	mp3HeaderBytes = 4  // one MPEG audio frame header
	otherSkipBytes = 64 // container header region skipped for FLAC/OGG/MP4/WebM
)

// ValidateAudio checks an uploaded audio file the way a real ASR server
// would fail on it, without decoding the audio stream.
//
// Parsing choices (Metrum AI strict-media):
//
//   - Size: payloads under MinAudioBytes are rejected outright.
//   - WAV (RIFF....WAVE): RIFF chunks are walked from offset 12. A "fmt "
//     chunk must declare a known format tag (PCM, IEEE float, A-law, mu-law,
//     or WAVE_FORMAT_EXTENSIBLE), 1..8 channels, a sample rate in
//     1000..384000 Hz, and a bits-per-sample value consistent with the tag. A
//     "data" chunk must follow with nonzero length, and its samples must not
//     all be silence (all zero bytes; for 8-bit formats, all one value). A
//     data size larger than the file is clamped, matching how streaming
//     writers leave 0xFFFFFFFF placeholders that decoders tolerate.
//   - MP3 (ID3 tag or MPEG frame sync 0xFFE0): an ID3v2 tag is skipped using
//     its synchsafe size (plus the footer if flagged). The remainder is
//     scanned for an MPEG audio frame header (version, layer, bitrate index,
//     sample-rate index) whose computed frame length lands exactly on a
//     second valid header with the same version, layer, and sample rate. Two
//     back-to-back headers is the same heuristic decoders such as mpg123 and
//     ffmpeg use to lock sync, so random bytes after a bare 0xFFFB do not
//     pass. The first frame body must not be all zeros. Free-format bitrate
//     (index 0) is rejected because its frame length cannot be computed from
//     the header.
//   - FLAC ("fLaC"), OGG ("OggS"), MP4/M4A ("ftyp" at offset 4), and WebM or
//     Matroska (EBML 0x1A45DFA3): accepted when at least MinAudioBytes long
//     and the bytes after the first 64 are not all zero. These are not
//     parsed further.
//   - Anything else: rejected as an unrecognized container.
//
// Independently of format, payloads whose bytes after the minimal header (44
// bytes for WAV, 4 bytes for MP3) are all zero are rejected.
func ValidateAudio(b []byte) error {
	if len(b) < MinAudioBytes {
		return fmt.Errorf("audio payload too small: %d bytes (minimum %d)", len(b), MinAudioBytes)
	}
	return validateContainer(b)
}

// validateContainer dispatches on container magic. It is also used for the
// bytes after an ID3v2 tag, because some FLAC, OGG, and WAV writers prepend
// one and ffmpeg and libFLAC still decode those files.
func validateContainer(b []byte) error {
	if len(b) < 12 {
		return errors.New("audio payload truncated after header")
	}
	switch {
	case len(b) >= 12 && string(b[0:4]) == "RIFF" && string(b[8:12]) == "WAVE":
		if allZero(b[wavHeaderBytes:]) {
			return errors.New("WAV payload is all zeros after header")
		}
		return validateWAV(b)
	case string(b[0:3]) == "ID3" || isMPEGSync(b):
		if allZero(b[mp3HeaderBytes:]) {
			return errors.New("MP3 payload is all zeros after header")
		}
		return validateMP3(b)
	case string(b[0:4]) == "fLaC",
		string(b[0:4]) == "OggS",
		string(b[4:8]) == "ftyp",
		b[0] == 0x1A && b[1] == 0x45 && b[2] == 0xDF && b[3] == 0xA3:
		if allZero(b[otherSkipBytes:]) {
			return errors.New("audio payload is all zeros after container header")
		}
		return nil
	default:
		return errors.New("unrecognized audio container (expected WAV, MP3, FLAC, OGG, M4A, or WebM)")
	}
}

// isChunkID reports whether b[off:off+4] is a printable-ASCII RIFF chunk id.
func isChunkID(b []byte, off int) bool {
	if off < 0 || off+4 > len(b) {
		return false
	}
	for _, c := range b[off : off+4] {
		if c < 0x20 || c > 0x7e {
			return false
		}
	}
	return true
}

func allZero(b []byte) bool {
	for _, c := range b {
		if c != 0 {
			return false
		}
	}
	return true
}

func allSame(b []byte) bool {
	for _, c := range b {
		if c != b[0] {
			return false
		}
	}
	return true
}

// WAV format tags from mmreg.h.
const (
	wavFormatPCM        = 0x0001
	wavFormatIEEEFloat  = 0x0003
	wavFormatALaw       = 0x0006
	wavFormatMuLaw      = 0x0007
	wavFormatExtensible = 0xFFFE
)

type wavFmt struct {
	tag        uint16
	channels   uint16
	sampleRate uint32
	blockAlign uint16
	bits       uint16
}

func validateWAV(b []byte) error {
	var (
		fmtChunk *wavFmt
		off      = 12
	)
	for off+8 <= len(b) {
		id := string(b[off : off+4])
		size := int64(binary.LittleEndian.Uint32(b[off+4 : off+8]))
		bodyStart := off + 8
		remaining := int64(len(b) - bodyStart)

		switch id {
		case "fmt ":
			if size < 16 || size > remaining {
				return errors.New("WAV fmt chunk is truncated or too short")
			}
			f, err := parseWAVFmt(b[bodyStart : bodyStart+int(size)])
			if err != nil {
				return err
			}
			fmtChunk = f
		case "data":
			if fmtChunk == nil {
				return errors.New("WAV data chunk appears before fmt chunk")
			}
			if size == 0 {
				return errors.New("WAV data chunk is empty")
			}
			if size > remaining {
				size = remaining
			}
			data := b[bodyStart : bodyStart+int(size)]
			if len(data) < int(fmtChunk.blockAlign) {
				return errors.New("WAV data chunk is shorter than one sample frame")
			}
			if allZero(data) || (fmtChunk.bits == 8 && allSame(data)) {
				return errors.New("WAV data chunk is silent (all samples identical)")
			}
			return nil
		}
		if size > remaining {
			break
		}
		// Chunks are word-aligned: odd sizes carry one pad byte. Some
		// writers omit it (libsndfile tolerates that), so fall back to the
		// unpadded offset when the padded one does not start a chunk id.
		next := bodyStart + int(size)
		if size&1 == 1 && !isChunkID(b, next+1) && isChunkID(b, next) {
			off = next
		} else {
			off = next + int(size&1)
		}
	}
	if fmtChunk == nil {
		return errors.New("WAV is missing fmt chunk")
	}
	return errors.New("WAV is missing data chunk")
}

func parseWAVFmt(c []byte) (*wavFmt, error) {
	f := &wavFmt{
		tag:        binary.LittleEndian.Uint16(c[0:2]),
		channels:   binary.LittleEndian.Uint16(c[2:4]),
		sampleRate: binary.LittleEndian.Uint32(c[4:8]),
		blockAlign: binary.LittleEndian.Uint16(c[12:14]),
		bits:       binary.LittleEndian.Uint16(c[14:16]),
	}
	if f.channels < 1 || f.channels > 8 {
		return nil, fmt.Errorf("WAV channel count %d out of range 1..8", f.channels)
	}
	if f.sampleRate < 1000 || f.sampleRate > 384000 {
		return nil, fmt.Errorf("WAV sample rate %d out of range 1000..384000", f.sampleRate)
	}
	var bitsOK bool
	switch f.tag {
	case wavFormatPCM:
		bitsOK = f.bits == 8 || f.bits == 16 || f.bits == 24 || f.bits == 32
	case wavFormatIEEEFloat:
		bitsOK = f.bits == 32 || f.bits == 64
	case wavFormatALaw, wavFormatMuLaw:
		bitsOK = f.bits == 8
	case wavFormatExtensible:
		bitsOK = f.bits >= 8 && f.bits <= 64 && f.bits%8 == 0
	default:
		return nil, fmt.Errorf("unsupported WAV format tag 0x%04x", f.tag)
	}
	if !bitsOK {
		return nil, fmt.Errorf("WAV bits per sample %d invalid for format tag 0x%04x", f.bits, f.tag)
	}
	if want := f.channels * (f.bits / 8); f.blockAlign != want {
		return nil, fmt.Errorf("WAV block align %d does not match channels*bytes %d", f.blockAlign, want)
	}
	return f, nil
}

func isMPEGSync(b []byte) bool {
	return len(b) >= 2 && b[0] == 0xFF && b[1]&0xE0 == 0xE0
}

// mpegHeader is the subset of an MPEG audio frame header needed to compute
// frame length and check consistency across frames.
type mpegHeader struct {
	version    int // 3 = MPEG-1, 2 = MPEG-2, 0 = MPEG-2.5
	layer      int // 1, 2, or 3
	sampleRate int
	frameLen   int
}

// Bitrates in kbps indexed by [table][bitrate index]; index 0 is free format
// and 15 is invalid, both rejected.
var mpegBitrates = [5][16]int{
	{0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448, -1}, // V1 L1
	{0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, -1},    // V1 L2
	{0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, -1},     // V1 L3
	{0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256, -1},    // V2/2.5 L1
	{0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, -1},         // V2/2.5 L2, L3
}

var mpegSampleRates = map[int][3]int{
	3: {44100, 48000, 32000},
	2: {22050, 24000, 16000},
	0: {11025, 12000, 8000},
}

// parseMPEGHeader decodes 4 header bytes; ok is false for any reserved or
// unsupported field value.
func parseMPEGHeader(h []byte) (mpegHeader, bool) {
	if len(h) < 4 || !isMPEGSync(h) {
		return mpegHeader{}, false
	}
	version := int(h[1]>>3) & 3
	layerBits := int(h[1]>>1) & 3
	brIdx := int(h[2] >> 4)
	srIdx := int(h[2]>>2) & 3
	padding := int(h[2]>>1) & 1
	emphasis := int(h[3]) & 3
	if version == 1 || layerBits == 0 || brIdx == 0 || brIdx == 15 || srIdx == 3 || emphasis == 2 {
		return mpegHeader{}, false
	}
	layer := 4 - layerBits

	var table int
	switch {
	case version == 3:
		table = layer - 1
	case layer == 1:
		table = 3
	default:
		table = 4
	}
	bitrate := mpegBitrates[table][brIdx] * 1000
	sr := mpegSampleRates[version][srIdx]

	var frameLen int
	switch {
	case layer == 1:
		frameLen = (12*bitrate/sr + padding) * 4
	case layer == 3 && version != 3:
		frameLen = 72*bitrate/sr + padding
	default:
		frameLen = 144*bitrate/sr + padding
	}
	if frameLen <= mp3HeaderBytes {
		return mpegHeader{}, false
	}
	return mpegHeader{version: version, layer: layer, sampleRate: sr, frameLen: frameLen}, true
}

func validateMP3(b []byte) error {
	start := 0
	if string(b[0:3]) == "ID3" {
		if len(b) < 10 {
			return errors.New("truncated ID3v2 tag")
		}
		var size int
		for _, c := range b[6:10] {
			if c&0x80 != 0 {
				return errors.New("invalid ID3v2 tag size (not synchsafe)")
			}
			size = size<<7 | int(c)
		}
		start = 10 + size
		if b[5]&0x10 != 0 { // footer present
			start += 10
		}
		if start >= len(b) {
			return errors.New("ID3v2 tag has no audio frames after it")
		}
		if rest := b[start:]; len(rest) >= 4 && (string(rest[0:4]) == "fLaC" ||
			string(rest[0:4]) == "OggS" || string(rest[0:4]) == "RIFF") {
			return validateContainer(rest)
		}
	}

	// Scan for the first offset where two consecutive frame headers agree.
	for off := start; off+mp3HeaderBytes <= len(b); off++ {
		first, ok := parseMPEGHeader(b[off:])
		if !ok {
			continue
		}
		next := off + first.frameLen
		if next+mp3HeaderBytes > len(b) {
			continue
		}
		second, ok := parseMPEGHeader(b[next:])
		if !ok || second.version != first.version || second.layer != first.layer ||
			second.sampleRate != first.sampleRate {
			continue
		}
		if allZero(b[off+mp3HeaderBytes : next]) {
			return errors.New("MP3 frame body is all zeros")
		}
		return nil
	}
	return errors.New("no two consecutive valid MPEG audio frames found")
}
