#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Metrum AI Bench CLI parity harness: replay a Metrum all-smi fork /metrics page.

Telemetry counts need the same exporter page in front of both tools, without a
GPU on the harness host. This Metrum AI helper has two subcommands:

  capture SRC OUT   Fetch a live fork page (http URL) or read a saved one (path),
                    redact host identity (gpu_uuid, hostname, instance, host),
                    and write OUT with the Metrum AI header and a provenance line.
  serve             Serve a captured page on GET /metrics. Each scrape moves the
                    values the way a live exporter does: gauges jitter around the
                    captured value (within their natural bounds), counters grow
                    with wall time, and *_info / build metadata stay fixed.
                    GET /metric returns 404, as on all-smi v0.26.3-metrum.4.

The series set is whatever the page holds; nothing is invented. The default
page, fixtures/all-smi-fork-h100.prom, is a redacted capture from a Shadeform
H100 PCIe host running all-smi v0.26.3 (Metrum fork).

Usage: fork_page.py serve [--page FILE] [--host 127.0.0.1] [--port 9090]
                          [--jitter 0.05] [--seed 0]
       fork_page.py capture SRC OUT [--source-note TEXT]
Standard library only.
"""

from __future__ import annotations

import argparse
import random
import re
import sys
import threading
import time
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEFAULT_PAGE = HERE / "fixtures" / "all-smi-fork-h100.prom"
HEADER = ("# Copyright (c) 2026 Metrum AI, Inc.\n"
          "# SPDX-License-Identifier: Apache-2.0\n")
SAMPLE_RE = re.compile(r"^([a-zA-Z_:][a-zA-Z0-9_:]*)(\{.*\})?\s+(\S+)(\s+\d+)?$")
HOST_LABELS = ("hostname", "instance", "host")


def redact(text: str) -> str:
    """Replace per-host identity labels with stable placeholders."""
    uuids: dict[str, str] = {}

    def uuid_sub(m: re.Match) -> str:
        real = m.group(1)
        if real not in uuids:
            uuids[real] = f"GPU-00000000-0000-0000-0000-{len(uuids):012d}"
        return f'gpu_uuid="{uuids[real]}"'

    text = re.sub(r'gpu_uuid="([^"]*)"', uuid_sub, text)
    for label in HOST_LABELS:
        text = re.sub(rf'(?<![a-z_]){label}="[^"]*"', f'{label}="parity-host"', text)
    return text


def capture(args: argparse.Namespace) -> int:
    src = args.src
    if src.startswith(("http://", "https://")):
        with urllib.request.urlopen(src, timeout=10) as resp:
            body = resp.read().decode("utf-8", "replace")
    else:
        body = Path(src).read_text(encoding="utf-8")
    # Drop any earlier header/provenance so re-captures stay idempotent.
    kept = [ln for ln in body.splitlines()
            if ln.startswith("# HELP") or ln.startswith("# TYPE") or not ln.startswith("#")]
    note = args.source_note or f"captured from {src}"
    out = (HEADER + f"# Metrum AI parity fixture, {note}; host identity redacted by fork_page.py.\n"
           + redact("\n".join(kept)).rstrip("\n") + "\n")
    Path(args.out).write_text(out, encoding="utf-8")
    names = {m.group(1) for ln in kept if (m := SAMPLE_RE.match(ln))}
    print(f"wrote {args.out}: {len(names)} metric names, "
          f"{sum(1 for n in names if n.startswith('all_smi_gpu_'))} all_smi_gpu_*", file=sys.stderr)
    return 0


class Page:
    """Parsed page plus per-sample state so successive scrapes move."""

    def __init__(self, path: Path, jitter: float, seed: int) -> None:
        self.lines = path.read_text(encoding="utf-8").splitlines()
        self.types: dict[str, str] = {}
        for ln in self.lines:
            if ln.startswith("# TYPE "):
                _, _, name, kind = ln.split(maxsplit=3)
                self.types[name] = kind.strip()
        self.jitter = jitter
        self.rng = random.Random(seed)
        self.lock = threading.Lock()
        self.t0 = time.monotonic()

    def kind(self, name: str) -> str:
        return self.types.get(name, "counter" if name.endswith("_total") else "gauge")

    def move(self, name: str, value: float) -> float:
        if name.endswith("_info") or name in ("all_smi_up", "all_smi_build_info") or value == 0:
            return value
        if self.kind(name) == "counter":
            # Grow about 1% of the captured value per second; stays monotonic.
            return value * (1.0 + 0.01 * (time.monotonic() - self.t0))
        moved = value * (1.0 + self.rng.uniform(-self.jitter, self.jitter))
        if "ratio" in name or "utilization" in name:
            ceiling = 1.0 if value <= 1.0 else 100.0
            moved = min(ceiling, moved)
        return max(0.0, moved)

    def render(self) -> str:
        out = []
        with self.lock:
            for ln in self.lines:
                m = None if ln.startswith("#") else SAMPLE_RE.match(ln)
                if not m:
                    if not ln.startswith("#") or ln.startswith(("# HELP", "# TYPE")):
                        out.append(ln)
                    continue
                name, labels, raw = m.group(1), m.group(2) or "", m.group(3)
                try:
                    value = float(raw)
                except ValueError:
                    out.append(ln)
                    continue
                moved = self.move(name, value)
                text = str(int(round(moved))) if value.is_integer() and self.kind(name) == "counter" else f"{moved:.6g}"
                out.append(f"{name}{labels} {text}")
        return "\n".join(out) + "\n"


def serve(args: argparse.Namespace) -> int:
    page = Page(Path(args.page), args.jitter, args.seed)

    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, fmt: str, *a) -> None:
            pass

        def do_GET(self) -> None:
            if self.path.split("?", 1)[0] != "/metrics":
                body, code, ctype = b"404 page not found\n", 404, "text/plain"
            else:
                body, code, ctype = page.render().encode(), 200, "text/plain; version=0.0.4"
            self.send_response(code)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    server = ThreadingHTTPServer((args.host, args.port), Handler)
    server.daemon_threads = True
    print(f"fork page {args.page} on http://{args.host}:{args.port}/metrics", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description="Metrum AI all-smi fork page replay for the parity harness")
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("serve", help="serve a captured page on /metrics")
    s.add_argument("--page", default=str(DEFAULT_PAGE))
    s.add_argument("--host", default="127.0.0.1")
    s.add_argument("--port", type=int, default=9090)
    s.add_argument("--jitter", type=float, default=0.05, help="gauge jitter as a fraction")
    s.add_argument("--seed", type=int, default=0)
    c = sub.add_parser("capture", help="capture and redact a live or saved page")
    c.add_argument("src", help="http(s) URL of a fork /metrics page, or a saved page path")
    c.add_argument("out")
    c.add_argument("--source-note", default="", help="provenance text for the fixture header")
    args = ap.parse_args()
    return serve(args) if args.cmd == "serve" else capture(args)


if __name__ == "__main__":
    raise SystemExit(main())
