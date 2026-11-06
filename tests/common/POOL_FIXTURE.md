<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Strategic connection-reuse fixture

`pool_fixture.rs` queues a fixed-Content-Length response whose final bytes are
`[DONE]`. There is no later HTTP transfer terminator to race pool return.
The response includes an eight-MiB valid SSE comment before the unchanged token
and usage events. The parser ignores the comment, but the transport must receive
and account for its real bytes. The test checks the exact complete body byte count
as well as the original minimum chunk count, reuse and socket assertions.

This is a bulk-response streaming test: `chunks_received` counts HTTP body reads,
not logical SSE events or server writes. The response exceeds the locked HTTP/1
reader's adaptive buffer by a wide margin. It therefore exercises multiple reads
without assuming that paced TCP writes become separate receive chunks. There
are no per-event sleeps or assumed packet boundaries. A wire/parser regression
checks Content-Length, ignored comment semantics, token count and final DONE.

Earlier paced small fixed-length writes failed the unchanged chunk assertion
under CI; an explicit HTTP-transfer-chunk experiment subsequently failed reuse.
Those failures remain failures. The bulk fixed-length design must pass its own
full repeated-suite validation and fresh CI before being treated as qualified.

The pooled control handles all three sequential requests on one accepted socket;
the existing CSV/summary reuse assertion remains at least two reused requests.
The independent socket-accept count must be exactly one. The negative control
sends Connection: close, disabling connection persistence; all three records
must report fresh connections and the server must accept three sockets.
Both controls retain the existing DNS/body-byte/chunk/receive-time checks.
This fixture is self-contained and does not depend on the Go dummy server.

Repeat locally with:

```sh
for i in $(seq 1 20); do
  METRUM_BENCH_REQUIRE_DUMMY=1 cargo test --locked --target-dir target --test e2e_http_trace || exit
done
```

A failed attempt remains a failure. Restart the complete sequence only after
fixing its cause; never count nonconsecutive reruns as a consecutive pass set.
