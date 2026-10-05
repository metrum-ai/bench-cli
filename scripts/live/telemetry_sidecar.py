#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Metrum AI Bench CLI telemetry sidecar for modality cells.

DEPRECATED (#196): metrum-ai-bench-cli-{llm,vlm,asr,imagegen} now take
--ndjson PATH --telemetry YAML [--require-telemetry], like the strategic
binary. They write telemetry.v1 rows with units and stage windows on the
same monotonic clock as their request rows. See docs/TELEMETRY.md. This
script stays only for binaries older than that change; it will be removed
once live scripts stop calling it.

Older metrum-ai-bench-cli-{llm,vlm,asr,imagegen} have no --telemetry flag
(only the strategic binary scrapes). This sidecar polls a Prometheus text endpoint, by
default the Metrum all-smi fork at http://127.0.0.1:9090/metrics, and writes
one NDJSON row per sample with a wall-clock timestamp, so a cell's data log
can be joined to telemetry by time:

  {"kind":"telemetry","t_wall":"...","t_unix_ns":...,"src":"all-smi",
   "metric":"all_smi_gpu_power_consumption_watts","labels":{...},"value":312.5}

Usage: telemetry_sidecar.py OUT.ndjson [--url URL] [--src NAME]
           [--interval-ms 500] [--include REGEX ...]
Runs until SIGTERM/SIGINT. Scrape errors become kind=scrape_error rows.
Standard library only.
"""

import argparse
import datetime
import json
import re
import signal
import sys
import time
import urllib.request

LINE = re.compile(r'^([a-zA-Z_:][a-zA-Z0-9_:]*)(\{(.*)\})?\s+(\S+)')
LABEL = re.compile(r'([a-zA-Z_][a-zA-Z0-9_]*)="((?:[^"\\]|\\.)*)"')


def parse(text, include):
    for line in text.splitlines():
        if not line or line.startswith("#"):
            continue
        m = LINE.match(line)
        if not m:
            continue
        name, labels, value = m.group(1), m.group(3) or "", m.group(4)
        if include and not any(r.search(name) for r in include):
            continue
        try:
            v = float(value)
        except ValueError:
            continue
        yield name, dict(LABEL.findall(labels)), v


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("--url", default="http://127.0.0.1:9090/metrics")
    ap.add_argument("--src", default="all-smi")
    ap.add_argument("--interval-ms", type=int, default=500)
    ap.add_argument("--include", action="append", default=[])
    a = ap.parse_args()
    print("telemetry_sidecar.py is deprecated (#196): pass --ndjson and --telemetry to the bench binary; see docs/TELEMETRY.md", file=sys.stderr)
    include = [re.compile(p) for p in a.include]
    stop = []
    signal.signal(signal.SIGTERM, lambda *_: stop.append(1))
    signal.signal(signal.SIGINT, lambda *_: stop.append(1))
    with open(a.out, "a", buffering=1) as f:
        while not stop:
            t0 = time.time()
            stamp = datetime.datetime.fromtimestamp(t0, datetime.timezone.utc).isoformat()
            base = {"t_wall": stamp, "t_unix_ns": int(t0 * 1e9), "src": a.src}
            try:
                with urllib.request.urlopen(a.url, timeout=max(a.interval_ms / 1000, 0.5)) as r:
                    body = r.read().decode("utf-8", "replace")
                for name, labels, v in parse(body, include):
                    f.write(json.dumps({"kind": "telemetry", **base, "metric": name,
                                        "labels": labels, "value": v}) + "\n")
            except Exception as e:  # keep polling; record the failure
                f.write(json.dumps({"kind": "scrape_error", **base, "error": str(e)}) + "\n")
            time.sleep(max(0.0, a.interval_ms / 1000 - (time.time() - t0)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
