<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Strategic connection-reuse fixture

`pool_fixture.rs` gives each SSE response a fixed Content-Length. The final
write contains both the finish/usage event and `[DONE]`, with no outstanding
HTTP chunk terminator after the parser sees DONE. This makes HTTP body drain
and eligibility for pool return coincide with the final response bytes rather
than racing the server's later chunked-transfer termination. TCP_NODELAY avoids
Nagle/delayed-ACK coalescing of the deliberately paced payload writes.

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
  cargo test --test e2e_http_trace strategic_points_report_http_phase_trace -- --exact || exit
done
```

A failed attempt remains a failure. Restart the complete sequence only after
fixing its cause; never count nonconsecutive reruns as a consecutive pass set.
