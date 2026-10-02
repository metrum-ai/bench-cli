<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# ASR benchmark

`metrum-ai-bench-cli-asr` benchmarks OpenAI-compatible
`/v1/audio/transcriptions` endpoints. To start: pick a server under
[Serving frameworks](#serving-frameworks), serve one of the real speech
fixtures in `test-data/asr/` with `--ground-truth`, and gate the cell with
`scripts/live/assert_headline.sh asr`. Before a real run, web-search the
current vendor docs for your exact ASR model and engine version, and record
the launch arguments and sources in the SUT (see [SERVING.md](SERVING.md)).

Its input is JSONL:

```json
{"id":"1089-134686-0030","path":"test-data/asr/1089-134686-0030.wav","format":"wav","duration":2.715}
```

## Serving frameworks

Run the bench against any server that implements OpenAI-compatible
`POST /v1/audio/transcriptions` (multipart `file`, `model`, optional
`language` and `response_format`).

| Stack | Status for this CLI (checked 2026-10-02) | Launcher |
|---|---|---|
| **vLLM-Omni** | Intended ASR stack. **Blocked in v0.30.0**: with `--omni` the engine reports only the `generate` and `speech` tasks, so the server never mounts `/v1/audio/transcriptions` ([vllm-omni#5722](https://github.com/vllm-project/vllm-omni/issues/5722), open). Use it only to re-validate once that issue is closed. | `ASR_STACK=omni scripts/live/serve/asr.sh start` (`vllm/vllm-omni:v0.30.0`) |
| **Regular vLLM speech-to-text** | Works today and is the launcher default. vLLM documents Whisper on this endpoint ([speech-to-text](https://docs.vllm.ai/en/latest/serving/online_serving/speech_to_text/)). The 2026-10-02 widen ASR cells ran on it. | `scripts/live/serve/asr.sh start` (`vllm/vllm-openai:v0.30.0`) |
| Other OpenAI-compatible servers | Possible; not run by us. | none |

```bash
scripts/live/serve/asr.sh start      # ASR_STACK=vllm: vllm/vllm-openai:v0.30.0, whisper-large-v3-turbo, --max-model-len 448
metrum-ai-bench-cli-asr --url http://127.0.0.1:8000/v1/audio/transcriptions --api-key dummy \
  --model openai/whisper-large-v3-turbo --scenario asr-smoke \
  --input test-data/asr/input.jsonl --ground-truth test-data/asr/truth.jsonl \
  --num-requests 6 --concurrency 1 --data-log asr.jsonl \
  --sut live-results/serve-asr/sut.json --require-sut
scripts/live/assert_headline.sh asr asr.jsonl
```

[`scripts/live/serve/asr.sh`](../scripts/live/serve/asr.sh) writes a SUT with
the exact launch command and the stack's sources. Results from different
stacks compare serving stacks, not models: record the stack in the SUT and do
not mix them in one comparison. Upstream references:

- vLLM-Omni: [docs](https://docs.vllm.ai/projects/vllm-omni/en/latest/),
  [v0.30.0 release](https://github.com/vllm-project/vllm-omni/releases/tag/v0.30.0),
  [transcription RFC #5722](https://github.com/vllm-project/vllm-omni/issues/5722)
- vLLM: [speech-to-text](https://docs.vllm.ai/en/latest/serving/online_serving/speech_to_text/),
  [multimodal inputs](https://docs.vllm.ai/en/latest/features/multimodal_inputs.html)
  (audio decoders, `--media-io-kwargs`),
  [v0.30.0 release](https://github.com/vllm-project/vllm/releases/tag/v0.30.0)
- Model: [`openai/whisper-large-v3-turbo`](https://huggingface.co/openai/whisper-large-v3-turbo)
  (transcription only; turbo does not translate)

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

#### Troubleshooting audio decoding

If valid WAV files return `Invalid or unsupported audio file`, or every
upload fails, suspect the server's audio decoder. vLLM 0.30.0 chooses the
decoder with `--media-io-kwargs '{"audio": {"audio_backend": ...}}'`. The
default, `auto`, tries soundfile, then torchcodec, then PyAV. Any other value
uses only that backend, with no fallback
([`vllm/multimodal/media/audio.py`](https://github.com/vllm-project/vllm/blob/v0.30.0/vllm/multimodal/media/audio.py)).

The stock `vllm/vllm-openai:v0.30.0` image does not include the
`vllm[audio]` extra. `soundfile` and `av` are not installed, so
`python -c "import soundfile, av"` fails even on a working server. The image
ships torchcodec, and torchcodec decodes the uploads. As a result, each
upload logs this line once and still succeeds:

`ERROR ... Failed to load audio via soundfile: ImportError('Please install vllm[audio] for audio support')`

On this image that ERROR is expected. In the 2026-10-02 widen ASR run on
1x H100, all 1031 uploads logged it once each and returned HTTP 200.

To check decoding:

1. Send a real fixture straight to the server, without the bench:

   ```bash
   curl -sS http://127.0.0.1:8000/v1/audio/transcriptions \
     -H 'Authorization: Bearer dummy' \
     -F file=@test-data/asr/1089-134686-0030.wav \
     -F model=openai/whisper-large-v3-turbo -F language=en -F response_format=json
   ```

   A working server answers HTTP 200 with the transcript
   `Beware of making that mistake.`
2. If that fails, search the server log (`docker logs metrum-live-asr` for
   the launcher's container) for
   `torchcodec unavailable (...); falling back to PyAV`. That WARNING, not the
   soundfile ERROR, means torchcodec cannot decode. torchcodec needs both its
   Python package and a system FFmpeg. `import torchcodec` alone is not a
   check, because torchcodec loads FFmpeg only when it opens a file. The
   stock image has no PyAV, so there is nothing left to fall back to.

To fix a missing decoder:

- Install the audio extra in a derived image, pinned to the installed
  version so pip adds only the decoders: `pip install 'vllm[audio]==0.30.0'`.
  This adds `av`, `scipy`, `soundfile`, `soxr`, and `mistral_common[audio]`.
- Or select torchcodec explicitly:
  `--media-io-kwargs '{"audio": {"audio_backend": "torchcodec"}}'`. This
  also skips the soundfile attempt and its ERROR line. We have not measured
  this setting.

Do not set `audio_backend` to `soundfile` or `pyav` on the stock image. An
explicit backend gets no fallback, so every upload fails while that library
is missing. This comes from the v0.30.0 source; it was not run live.

### Other OpenAI-compatible backends

Any server that speaks `/v1/audio/transcriptions` works with the same flags.
For example, [speaches](https://github.com/speaches-ai/speaches)
(faster-whisper) describes itself as OpenAI API compatible; we have not run
it. Record the backend name, version, and launch command in the SUT. Check
that it accepts `--response-format json` or `verbose-json`.

Live verification status for ASR is tracked in
[CLAIMS_LEDGER.md](CLAIMS_LEDGER.md). [SERVING.md](SERVING.md) lists the
stack for every modality.

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
