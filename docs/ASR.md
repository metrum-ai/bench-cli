<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# ASR benchmark

`metrum-ai-bench-cli-asr` benchmarks OpenAI-compatible `/audio/transcriptions`
endpoints. Its input is JSONL:

```json
{"id":"1089-134686-0030","path":"test-data/asr/1089-134686-0030.wav","format":"wav","duration":2.715}
```

## Serving frameworks

Run the bench against any server that implements OpenAI-compatible
`POST /v1/audio/transcriptions` (multipart `file`, `model`, optional
`language` and `response_format`).

### Recommended: vLLM speech-to-text

```bash
scripts/live/serve/asr.sh start      # vllm/vllm-openai:v0.30.0, openai/whisper-large-v3-turbo, --max-model-len 448
metrum-ai-bench-cli-asr --url http://127.0.0.1:8000/v1/audio/transcriptions --api-key dummy \
  --model openai/whisper-large-v3-turbo --scenario asr-smoke \
  --input test-data/asr/input.jsonl --ground-truth test-data/asr/truth.jsonl \
  --num-requests 6 --concurrency 1 --data-log asr.jsonl \
  --sut live-results/serve-asr/sut.json --require-sut
scripts/live/assert_headline.sh asr asr.jsonl
```

[`scripts/live/serve/asr.sh`](../scripts/live/serve/asr.sh) pins
`vllm/vllm-openai:v0.30.0` and writes a SUT with the exact launch command.
Upstream references:

- Endpoint, request fields, and supported models:
  [vLLM speech-to-text](https://docs.vllm.ai/en/latest/serving/online_serving/speech_to_text/)
- Audio inputs, decoder backends, and `--media-io-kwargs`:
  [vLLM multimodal inputs](https://docs.vllm.ai/en/latest/features/multimodal_inputs.html)
- Model: [`openai/whisper-large-v3-turbo`](https://huggingface.co/openai/whisper-large-v3-turbo)
  (transcription only; turbo does not translate)
- Release we pin: [vLLM v0.30.0](https://github.com/vllm-project/vllm/releases/tag/v0.30.0)

### Whisper on vLLM 0.30.0: `--max-model-len 448`

Serve Whisper with `--max-model-len 448`:

```bash
vllm serve openai/whisper-large-v3-turbo --max-model-len 448
```

448 is the Whisper decoder's position limit (`max_target_positions`). vLLM
0.30.0 derives it on its own and its examples pass it explicitly; setting it
records the value in the launch command and rules out a larger value. Sources:
[vLLM speech-to-text docs](https://docs.vllm.ai/en/latest/serving/online_serving/speech_to_text/)
and `examples/generate/multimodal/audio_language_offline.py` in vLLM v0.30.0.
`scripts/live/serve/asr.sh` uses this configuration.

If valid WAV files still return `Invalid or unsupported audio file`, the
image may be missing an audio decoder (vLLM reports a missing `soundfile`
or `av` library with the same message). Check with
`python -c "import soundfile, av"` inside the container, or force a backend
with `--media-io-kwargs '{"audio": {"audio_backend": "soundfile"}}'`.

### Other OpenAI-compatible backends

Any server that speaks `/v1/audio/transcriptions` works with the same flags.
For example, [speaches](https://github.com/speaches-ai/speaches) (faster-whisper)
describes itself as OpenAI API compatible. We have not run it. If you use a
backend other than vLLM, record its name, version, and launch command in the
SUT, and check that its `response_format` values include `json` or
`verbose_json` (`--response-format`). Results from different backends compare
serving stacks, not models.

Live verification status for ASR is tracked in
[CLAIMS_LEDGER.md](CLAIMS_LEDGER.md). It is functional only until a live run
reports WER. [SERVING.md](SERVING.md) lists the stack for every modality.

## Audio must be real, decodable audio

Real servers decode the upload before transcribing. vLLM answers HTTP 400
`Invalid or unsupported audio file` for bytes it cannot decode, and an
all-error run has no throughput or WER to report. Header-only or zero-filled
files, which pass a dummy server that never decodes audio, fail here. For
the containers and codecs vLLM accepts, see the
[speech-to-text](https://docs.vllm.ai/en/latest/serving/online_serving/speech_to_text/)
and [multimodal inputs](https://docs.vllm.ai/en/latest/features/multimodal_inputs.html)
docs; 16 kHz mono PCM WAV (as in `test-data/asr/`) is the safe default.

`test-data/asr/` ships three LibriSpeech `test-clean` utterances (CC BY 4.0)
as 16 kHz mono 16-bit WAV with an `input.jsonl` manifest and exact
transcripts in `truth.jsonl`; see [test-data/README.md](../test-data/README.md).
`test-data/negative/header-only-invalid.mp3` is a negative fixture that every
real server rejects. Use it only to test error handling.

`dummy-model-server -strict-media` approximates this check (container magic,
WAV chunks, MPEG frame headers, non-silent body) so CI catches fake audio. It
does not decode audio; see [LIMITATIONS.md](LIMITATIONS.md).

## Ground truth, WER, and CER

Ground truth is JSONL with matching `id` and `transcript` fields, passed with
`--ground-truth`. Every request record whose sample has a reference carries
`modality_metrics.wer` and `modality_metrics.cer`. The summary line has no
WER aggregate; average the request records, or use
`scripts/live/assert_headline.sh asr <data_log>`, which prints the means and
fails if a successful record lacks WER or CER.

```bash
metrum-ai-bench-cli-asr \
  --scenario asr-wer --url http://127.0.0.1:8000/v1/audio/transcriptions \
  --api-key dummy --num-requests 3 --concurrency 1 \
  --input test-data/asr/input.jsonl --ground-truth test-data/asr/truth.jsonl \
  --model openai/whisper-large-v3-turbo --data-log asr.jsonl
```

The ground truth is matched by `id`; samples without a reference get no WER.
WER and CER normalize reference and hypothesis with the same normalizer,
chosen with `--normalizer`:

| Value | Behavior |
|-------|----------|
| `whisper-english` (default) | Case, punctuation and bracketed fillers folded, then contractions expanded and small numerals digitized. Comparable to published Whisper-normalizer WER. |
| `whisper-basic` | Case, punctuation and bracketed fillers only. |
| `none` | Raw string comparison. |

The selected value is echoed in the run record's `config.normalizer`; scores
from different settings are not comparable. The request record distinguishes
server-reported inference seconds from client-measured inference seconds and
records `rtfx_client = audio_seconds / client_seconds`.

```bash
metrum-ai-bench-cli-asr \
  --scenario smoke --url http://127.0.0.1:8000/v1/audio/transcriptions \
  --api-key dummy --num-requests 20 --concurrency 4 \
  --input samples.jsonl --ground-truth truth.jsonl --model whisper-1 \
  --response-format verbose-json --data-log asr.jsonl --seed 7
```

Run `metrum-ai-bench-cli-asr --help` for the complete, authoritative argument list.
