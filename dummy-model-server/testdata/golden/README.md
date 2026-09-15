<!-- Copyright (c) 2026 Metrum AI, Inc. SPDX-License-Identifier: Apache-2.0 -->

# Golden timing shape (appendix 8.3)

Configuration:

- `-latency=100ms`
- `-chunk-interval=20ms`
- `max_tokens=20`
- stream=true

Expected client-observed shape (wall clock vs first content SSE / stream end):

| Metric | Expected |
|--------|----------|
| TTFT | ~120 ms (100 ms latency + 20 ms first chunk sleep) |
| Response time | ~500 ms (100 + 20×20) |
| Content deltas | 20 × `"."` |
| Final | finish_reason=stop, usage.completion_tokens=20, then `data: [DONE]` |

Request fixture (`chat_stream_20.json`):

```json
{
  "model": "dummy",
  "messages": [{"role": "user", "content": "Hi"}],
  "max_tokens": 20,
  "stream": true,
  "stream_options": {"include_usage": true}
}
```
