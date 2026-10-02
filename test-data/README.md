<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Test fixture provenance

- `tiny.png` is a programmatically generated 1x1 RGBA PNG. It is for unit
  tests only; real VLM servers and `dummy-model-server -strict-media` reject
  images smaller than 2x2.
- `llm-hi.jsonl` is a one-line LLM prompt fixture (`{"prompt":"Hi"}`).
- `vlm/shapes-512.png` is a 512x512 RGB PNG (red circle, blue square, green
  triangle, and the word BENCH) written by `scripts/gen_vlm_fixture.py`. The
  script is deterministic; rerun it to reproduce the file byte for byte.
  `vlm/prompts.jsonl` is a one-row VLM prompt file that points at it.
- `asr/` holds real speech for ASR runs that report WER and CER:

  | id (speaker-chapter-utterance) | Speaker | Chapter | Utterance | Seconds | Transcript |
  |---|---|---|---|---|---|
  | `1089-134686-0030` | 1089 | 134686 | 0030 | 2.715 | BEWARE OF MAKING THAT MISTAKE |
  | `2961-961-0016` | 2961 | 961 | 0016 | 7.815 | I WILL BRIEFLY DESCRIBE THEM TO YOU AND YOU SHALL READ THE ACCOUNT OF THEM AT YOUR LEISURE IN THE SACRED REGISTERS |
  | `6930-81414-0005` | 6930 | 81414 | 0005 | 1.815 | WHAT WAS THAT |

  Source: LibriSpeech ASR corpus, `test-clean` split,
  https://www.openslr.org/12 (`test-clean.tar.gz`, MD5
  `32fa31d27d2e1cad72775fee3f4849a9`), CC BY 4.0. `asr/truth.jsonl` holds the
  exact LibriSpeech transcripts; `asr/input.jsonl` is the
  `metrum-ai-bench-cli-asr` manifest (paths are relative to the repository or
  archive root). `scripts/fetch_asr_fixtures.sh` regenerates all five files.
  Each clip was converted with:

  ```bash
  ffmpeg -i <utterance>.flac -map_metadata -1 -fflags +bitexact -flags:a +bitexact \
    -ac 1 -ar 16000 -sample_fmt s16 -c:a pcm_s16le <utterance>.wav
  ```

- `negative/header-only-invalid.mp3` is a negative fixture: 104 bytes, an
  MPEG frame header (`ff fb 90 00`) followed by 100 zero bytes. It is not
  audio. vLLM Whisper returns HTTP 400 `Invalid or unsupported audio file` for
  it, and so does `dummy-model-server -strict-media`. Use it only to test
  error handling. It was `test-data/dummy.mp3` in 1.5.2 and earlier.
- endpoint YAML files are authored for this repository and contain only
  loopback/example configuration.
- `reference-result.json` is generated from the repository's deterministic
  dummy model server using `docs/REPRODUCING.md`.

Apart from `asr/` (LibriSpeech, CC BY 4.0, attributed above and in `NOTICE`),
these fixtures contain no third-party creative work or production data.
