<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# ASR benchmark

`metrum-ai-bench-asr` benchmarks OpenAI-compatible `/audio/transcriptions`
endpoints. Its input is JSONL:

```json
{"id":"sample-1","path":"test-data/dummy.mp3","format":"mp3","duration":2.0}
```

Optional ground truth is JSONL with matching `id` and `transcript` fields.
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
metrum-ai-bench-asr \
  --scenario smoke --url http://127.0.0.1:8000/v1/audio/transcriptions \
  --api-key dummy --num-requests 20 --concurrency 4 \
  --input samples.jsonl --ground-truth truth.jsonl --model whisper-1 \
  --response-format verbose-json --data-log asr.jsonl --seed 7
```

Run `metrum-ai-bench-asr --help` for the complete, authoritative argument list.
