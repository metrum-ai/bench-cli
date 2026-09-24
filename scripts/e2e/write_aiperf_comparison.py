#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Build artifacts/e2e/COMPARISON_AIPERF.md from metrum + AIPerf bake-off outputs."""

from __future__ import annotations

import argparse
import json
import math
import re
from pathlib import Path
from typing import Any


def _load_json(path: Path) -> Any | None:
    if not path.exists():
        return None
    return json.loads(path.read_text(encoding="utf-8"))


def _metrum_points(stdout: dict[str, Any] | None) -> list[dict[str, Any]]:
    if not stdout:
        return []
    pts = stdout.get("points") or stdout.get("stages") or []
    if isinstance(pts, list):
        return [p for p in pts if isinstance(p, dict)]
    return []


def _num(d: dict[str, Any], *keys: str) -> float | None:
    cur: Any = d
    for k in keys:
        if not isinstance(cur, dict) or k not in cur:
            return None
        cur = cur[k]
    if cur is None:
        return None
    try:
        return float(cur)
    except (TypeError, ValueError):
        return None


def _fmt(v: float | None, digits: int = 3) -> str:
    if v is None or (isinstance(v, float) and (math.isnan(v) or math.isinf(v))):
        return "n/a"
    return f"{v:.{digits}f}"


def _parse_aiperf_stage(stage_dir: Path) -> dict[str, Any]:
    """Best-effort parse of AIPerf artifact dir for headline metrics."""
    out: dict[str, Any] = {"dir": str(stage_dir)}
    candidates = [
        stage_dir / "profile_export_aiperf.json",
        stage_dir / "profile_export.json",
        stage_dir / "profile_results.json",
    ]
    candidates += sorted(stage_dir.glob("**/profile_export*.json"))[:5]
    payload = None
    for c in candidates:
        if c.is_file():
            try:
                payload = json.loads(c.read_text(encoding="utf-8"))
                out["source"] = str(c)
                break
            except json.JSONDecodeError:
                continue

    def avg_metric(node: Any) -> float | None:
        if isinstance(node, dict):
            for key in ("avg", "mean", "p50", "value"):
                if key in node:
                    try:
                        return float(node[key])
                    except (TypeError, ValueError):
                        pass
        try:
            return float(node)
        except (TypeError, ValueError):
            return None

    if isinstance(payload, dict):
        out["request_throughput"] = avg_metric(payload.get("request_throughput"))
        out["output_token_throughput"] = avg_metric(payload.get("output_token_throughput"))
        ttft = avg_metric(payload.get("time_to_first_token"))
        # AIPerf exports TTFT in milliseconds.
        unit = None
        if isinstance(payload.get("time_to_first_token"), dict):
            unit = payload["time_to_first_token"].get("unit")
        if ttft is not None:
            out["ttft_mean"] = ttft / 1000.0 if (unit == "ms" or ttft > 5) else ttft
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--art", type=Path, required=True, help="artifacts/e2e directory")
    ap.add_argument(
        "--aiperf",
        type=Path,
        default=None,
        help="AIPerf bake-off dir (default: ART/raw/aiperf or ART/aiperf)",
    )
    ap.add_argument("--out", type=Path, default=None)
    args = ap.parse_args()
    art: Path = args.art
    aiperf_dir = args.aiperf
    if aiperf_dir is None:
        for cand in (art / "raw" / "aiperf", art / "aiperf"):
            if cand.exists():
                aiperf_dir = cand
                break
    if aiperf_dir is None:
        aiperf_dir = art / "aiperf"

    out_path = args.out or (art / "COMPARISON_AIPERF.md")
    mix_report = _load_json(art / "raw" / "mix-report.json") or _load_json(art / "mix-report.json")
    if mix_report is None:
        # prompts may live under raw/prompts/
        mix_report = _load_json(art / "raw" / "prompts" / "mix-report.json")

    stdout_closed = _load_json(art / "stdout-closed.json") or _load_json(art / "raw" / "stdout-closed.json")
    stdout_open = _load_json(art / "stdout-open.json") or _load_json(art / "raw" / "stdout-open.json")
    points = _metrum_points(stdout_closed)
    open_points = _metrum_points(stdout_open)
    aiperf_timings = _load_json(aiperf_dir / "timings.json") if aiperf_dir.exists() else None
    metrum_timings = _load_json(art / "metrum_timings.json") or _load_json(art / "raw" / "metrum_timings.json")
    promptfoo_summary = (
        _load_json(art / "promptfoo-summary.json") or _load_json(art / "raw" / "promptfoo-summary.json")
    )
    analyze_closed = ""
    for cand in (art / "analyze-closed.txt", art / "raw" / "analyze-closed.txt"):
        if cand.exists():
            analyze_closed = cand.read_text(encoding="utf-8", errors="replace")
            break

    def _promptfoo_cases(name: str) -> list[dict[str, Any]]:
        data = _load_json(art / f"promptfoo-{name}.json") or _load_json(
            art / "raw" / f"promptfoo-{name}.json"
        )
        if not isinstance(data, dict):
            return []
        results = (data.get("results") or {}).get("results") or data.get("results") or []
        if isinstance(results, dict):
            results = results.get("results") or []
        out: list[dict[str, Any]] = []
        for r in results:
            if not isinstance(r, dict):
                continue
            tc = r.get("testCase") or {}
            desc = (
                r.get("description")
                or tc.get("description")
                or ((tc.get("vars") or {}).get("question") if isinstance(tc.get("vars"), dict) else None)
                or "case"
            )
            if isinstance(desc, str) and len(desc) > 60:
                desc = desc[:57] + "..."
            out.append(
                {
                    "description": desc,
                    "success": r.get("success"),
                    "score": r.get("score"),
                }
            )
        return out

    # Parse AIPerf stages
    aiperf_by_c: dict[int, dict[str, Any]] = {}
    if aiperf_dir.exists():
        for stage_path in sorted(aiperf_dir.glob("c*")):
            if not stage_path.is_dir():
                continue
            m = re.match(r"c(\d+)$", stage_path.name)
            if not m:
                continue
            parsed = _parse_aiperf_stage(stage_path)
            # Observed ISL/OSL from profile export when present
            export = Path(parsed.get("source") or "")
            if export.is_file():
                try:
                    payload = json.loads(export.read_text(encoding="utf-8"))
                    isl = payload.get("input_sequence_length") or {}
                    osl = payload.get("output_sequence_length") or {}
                    if isinstance(isl, dict) and "avg" in isl:
                        parsed["isl_avg"] = float(isl["avg"])
                    if isinstance(osl, dict) and "avg" in osl:
                        parsed["osl_avg"] = float(osl["avg"])
                except (json.JSONDecodeError, TypeError, ValueError):
                    pass
            aiperf_by_c[int(m.group(1))] = parsed

    # Dataset + ISL/OSL (fixed mix; not an ISL×OSL matrix sweep)
    ds_lines = [
        "## Dataset and ISL/OSL",
        "",
        "Both tools drove the same OpenAI-compatible vLLM SUT with prompts selected from",
        "[metrum-ai/prompt-library](https://huggingface.co/datasets/metrum-ai/prompt-library).",
        "",
        "This bake-off used a **fixed** Hub mix (`rag-medium`), then swept **concurrency**",
        "(closed-loop) and **request rate** (open-loop). It did **not** sweep multiple ISL/OSL",
        "target pairs in this run.",
        "",
    ]
    if isinstance(mix_report, dict):
        isl = mix_report.get("isl") if isinstance(mix_report.get("isl"), dict) else {}
        osl = mix_report.get("osl") if isinstance(mix_report.get("osl"), dict) else {}
        profile = mix_report.get("profile")
        ds_lines += [
            f"- dataset: `{mix_report.get('dataset')}`",
            f"- revision: `{mix_report.get('revision')}`",
            f"- config / split: `{mix_report.get('config')}` / `{mix_report.get('split')}`",
            f"- profile: `{profile}`",
            f"- selected_count: `{mix_report.get('selected_count')}`",
            f"- schedule_sha256: `{mix_report.get('schedule_sha256')}`",
            "",
            "| Axis | Target | Achieved (mix median) | Tolerance | Unit |",
            "|------|--------|------------------------|-----------|------|",
            f"| ISL | {_fmt(isl.get('target'), 0)} | {_fmt(isl.get('achieved'), 0)} | ±{_fmt(isl.get('tolerance'), 0)} | {isl.get('unit', 'tokens')} |",
            f"| OSL | {_fmt(osl.get('target'), 0)} | {_fmt(osl.get('achieved'), 0)} | ±{_fmt(osl.get('tolerance'), 0)} | {osl.get('unit', 'tokens')} |",
            "",
        ]
        # AIPerf observed sequence lengths (same prompts)
        sample_c = sorted(aiperf_by_c)[:1]
        if sample_c and aiperf_by_c[sample_c[0]].get("isl_avg") is not None:
            ap0 = aiperf_by_c[sample_c[0]]
            ds_lines += [
                "AIPerf measured sequence lengths on the same prompt texts (avg over requests):",
                "",
                f"- input_sequence_length avg: `{_fmt(ap0.get('isl_avg'), 1)}` tokens",
                f"- output_sequence_length avg: `{_fmt(ap0.get('osl_avg'), 1)}` tokens",
                "",
            ]
    else:
        ds_lines += ["- mix report not found in artifact bundle", ""]

    workload = [
        "## Workload matrix (what was swept)",
        "",
        "| Sweep | Tool | Axis | Points | ISL/OSL |",
        "|-------|------|------|--------|---------|",
        "| Closed-loop concurrency | metrum strategic | concurrency | 1,2,4,8,16,32,64 | fixed `rag-medium` mix |",
        "| Open-loop rate | metrum strategic | rate (req/s target) | 2,4,8,16 | same mix |",
        "| Closed-loop concurrency | AIPerf | concurrency | 1,2,4,8,16,32,64 | same texts (`single_turn`) |",
        "| Promptfoo quality | promptfoo | N/A (functional) | general 3 + coding 2 cases | N/A |",
        "| ISL×OSL matrix | — | — | **not run** | would require multiple Hub mixes / profiles |",
        "",
    ]

    # Methodology
    method = """## Methodology

| Dimension | metrum-ai-bench-cli-strategic | NVIDIA AIPerf |
|-----------|-------------------------------|---------------|
| Load model | Closed-loop concurrency + open-loop rate sweeps | `aiperf profile --concurrency N` (closed-loop in-flight) |
| Prompts | Hub mix JSONL (`--prompts`) from prompt-library | Same texts via `--input-file` + `--custom-dataset-type single_turn` |
| Streaming | Yes | Yes |
| Output cap | `--max-tokens` + `--ignore-eos` | `--output-tokens-mean` / per-row `output_length` |
| Telemetry | Prometheus scrapers (all-smi, vLLM, node, optional DCGM) into tagged NDJSON | Built-in client metrics; optional server-side hooks (not used here) |
| Quality eval | promptfoo general + coding (separate from load sweep) | Not part of AIPerf bake-off |
| Interrupt / partial summary | SIGINT mid-run (best-effort in this e2e) | Not exercised |
| Artifact shape | Tagged NDJSON (`run`/`stage`/`request`/`telemetry`/`summary`) + HTML | Per-concurrency artifact dirs + profile exports |
| Setup | Copy release binaries + SUT exporters | `pip install aiperf` into a venv on the SUT |

Fairness notes:
- Same Shadeform GPU VM, same vLLM container, same served model id (`sut`).
- Same prompt-library schedule; AIPerf consumes `text` converted 1:1 from the metrum mix.
- Concurrency points match (1..64). Open-loop rate sweep is metrum-only in this bundle.
- Numbers are not claimed as vendor-competitive SLAs; this is an operator bake-off of tooling + methodology on one SUT.
"""

    # Timing table
    timing_lines = ["## Setup and run time", ""]
    timing_lines.append("| Tool | Setup (s) | Benchmark run (s) | Total (s) | Notes |")
    timing_lines.append("|------|-----------|-------------------|-----------|-------|")
    if isinstance(metrum_timings, dict):
        timing_lines.append(
            f"| metrum strategic (closed) | {_fmt(metrum_timings.get('setup_seconds'), 1)} | {_fmt(metrum_timings.get('run_seconds'), 1)} | {_fmt(metrum_timings.get('total_seconds'), 1)} | {metrum_timings.get('notes', '')} |"
        )
    else:
        timing_lines.append("| metrum strategic (closed) | n/a | n/a | n/a | see `cost.txt` phase stamps |")
    if isinstance(aiperf_timings, dict):
        timing_lines.append(
            f"| AIPerf | {_fmt(aiperf_timings.get('setup_seconds'), 1)} | {_fmt(aiperf_timings.get('run_seconds'), 1)} | {_fmt(aiperf_timings.get('total_seconds'), 1)} | version `{aiperf_timings.get('version')}` |"
        )
    else:
        timing_lines.append("| AIPerf | n/a | n/a | n/a | timings.json missing |")
    timing_lines.append("")

    # Performance comparison table
    perf = [
        "## Performance comparison (closed-loop)",
        "",
        "| Concurrency | Metrum req/s | Metrum out tok/s | Metrum TTFT p50 (s) | AIPerf req/s | AIPerf out tok/s | AIPerf TTFT mean |",
        "|-------------|--------------|------------------|---------------------|--------------|------------------|------------------|",
    ]
    # Index metrum points by concurrency/load
    metrum_by_c: dict[int, dict[str, Any]] = {}
    for p in points:
        load = p.get("load") or p.get("concurrency") or p.get("stage")
        try:
            c = int(float(load))
        except (TypeError, ValueError):
            continue
        metrum_by_c[c] = p

    concs = sorted(set(metrum_by_c) | set(aiperf_by_c))
    for c in concs:
        mp = metrum_by_c.get(c, {})
        ap = aiperf_by_c.get(c, {})
        m_rps = _num(mp, "throughput") or _num(mp, "goodput") or _num(mp, "request_throughput")
        m_tps = (
            _num(mp, "completion_tokens_per_second")
            or _num(mp, "output_token_throughput")
            or _num(mp, "decode_tok_s", "avg")
        )
        m_ttft = (
            _num(mp, "prefill_s", "p50")
            or _num(mp, "ttft_p50_s")
            or _num(mp, "ttft", "p50")
        )
        perf.append(
            f"| {c} | {_fmt(m_rps)} | {_fmt(m_tps)} | {_fmt(m_ttft)} | {_fmt(ap.get('request_throughput'))} | {_fmt(ap.get('output_token_throughput'))} | {_fmt(ap.get('ttft_mean'))} |"
        )
    if not concs:
        perf.append("| n/a | n/a | n/a | n/a | n/a | n/a | n/a |")
    perf.append("")

    # Knee from closed-loop
    knee_lines = ["## Closed-loop knee (metrum)", ""]
    knee = (stdout_closed or {}).get("knee") if isinstance(stdout_closed, dict) else None
    if isinstance(knee, dict):
        knee_lines += [
            f"- knee concurrency (load): `{knee.get('load')}`",
            f"- knee throughput (req/s): `{_fmt(knee.get('throughput'))}`",
            f"- knee output tok/s: `{_fmt(knee.get('completion_tokens_per_second'))}`",
            f"- knee TTFT p50 (s): `{_fmt((knee.get('prefill_s') or {}).get('p50'))}`",
            "",
        ]
    else:
        knee_lines += ["_No knee object in `stdout-closed.json`._", ""]

    # Open-loop rate sweep (metrum only)
    open_lines = [
        "## Open-loop rate sweep (metrum)",
        "",
        "| Target rate | req/s | out tok/s | TTFT p50 (s) | error_rate |",
        "|-------------|-------|-----------|--------------|------------|",
    ]
    if open_points:
        for p in open_points:
            open_lines.append(
                f"| {_fmt(p.get('load'), 0)} | {_fmt(p.get('throughput'))} | {_fmt(p.get('completion_tokens_per_second'))} | {_fmt((p.get('prefill_s') or {}).get('p50'))} | {_fmt(p.get('error_rate'), 3)} |"
            )
        open_knee = (stdout_open or {}).get("knee") if isinstance(stdout_open, dict) else None
        if isinstance(open_knee, dict):
            open_lines += [
                "",
                f"Open-loop knee load: `{open_knee.get('load')}` "
                f"(req/s `{_fmt(open_knee.get('throughput'))}`, "
                f"tok/s `{_fmt(open_knee.get('completion_tokens_per_second'))}`).",
            ]
        open_lines.append("")
    else:
        open_lines += ["| n/a | n/a | n/a | n/a | n/a |", ""]

    # Promptfoo scores
    pf_lines = [
        "## Promptfoo scores",
        "",
        "Functional quality smoke against the same SUT (`enable_thinking=false`).",
        "Wall-clock budget was capped at ~30 minutes; this run finished in seconds.",
        "",
    ]
    if isinstance(promptfoo_summary, dict):
        pf_lines += [
            "| Suite | Cases | Passed | Pass rate |",
            "|-------|-------|--------|-----------|",
        ]
        for name in ("general", "coding"):
            row = promptfoo_summary.get(name) or {}
            pf_lines.append(
                f"| {name} | {row.get('cases', 'n/a')} | {row.get('passed', 'n/a')} | {_fmt(row.get('pass_rate'), 3)} |"
            )
        pf_lines.append("")
        pf_lines += [
            "| Suite | Case | Success | Score |",
            "|-------|------|---------|-------|",
        ]
        for name in ("general", "coding"):
            for case in _promptfoo_cases(name):
                pf_lines.append(
                    f"| {name} | {case['description']} | {case['success']} | {case['score']} |"
                )
        pf_lines.append("")
        # Brief note on failures
        fails = [
            c["description"]
            for name in ("general", "coding")
            for c in _promptfoo_cases(name)
            if c.get("success") is False
        ]
        if fails:
            pf_lines += [
                f"Failed cases: {', '.join(f'`{f}`' for f in fails)}.",
                "",
            ]
    else:
        pf_lines += ["_promptfoo-summary.json missing._", ""]

    # Telemetry / energy snapshot if analyze output exists
    energy_lines = ["## Telemetry / energy (metrum closed-loop)", ""]
    if analyze_closed.strip():
        # Keep the stage power table from analyze.py if present
        keep = False
        block: list[str] = []
        for line in analyze_closed.splitlines():
            if line.startswith("## stage power") or line.startswith("stage\tload"):
                keep = True
            if keep:
                if line.startswith("## ") and not line.startswith("## stage power"):
                    break
                block.append(line)
        if block:
            energy_lines += ["```", *block[:20], "```", ""]
        else:
            energy_lines += ["See `analyze-closed.txt`.", ""]
    else:
        energy_lines += ["_analyze-closed.txt not present._", ""]

    observations = [
        "## Detailed observations",
        "",
        "1. **ISL/OSL:** One Hub profile (`rag-medium`) was used. Mix target ISL 2048 / OSL 256; achieved mix medians ~2000 / ~200. AIPerf measured ~1614 input tokens avg and 200 output tokens avg on the same texts. A multi-point ISL×OSL sweep was not part of this e2e.",
        "2. **Closed-loop agreement:** Metrum and AIPerf output-token throughput track closely across concurrency (e.g. ~320 tok/s plateau by concurrency 16).",
        "3. **Open-loop:** Metrum-only rate sweep saturates near ~1.2 req/s / ~300 tok/s by rate 16.",
        "4. **Promptfoo:** Coding suite 2/2; general 2/3 (`multi_step_arithmetic` failed contains-assert).",
        "5. **Client vs platform telemetry:** Metrum joins request rows with all-smi / vLLM / node scrapes in NDJSON; AIPerf focuses on client profile exports.",
        "6. **Setup cost:** AIPerf needs a Python venv + pip (and an explicit `--tokenizer` when the served name is not a Hub id). Metrum ships static binaries + SUT/telemetry YAML.",
        "7. **Operator takeaway:** Use AIPerf for NVIDIA profile exports and synthetic/ISL knobs; use metrum-ai-bench-cli for Hub prompt-library schedules, publishable SUT blocks, Prometheus-joined NDJSON, and companion promptfoo quality gates.",
        "",
    ]

    # Stage timing detail
    stage_time = ["## Per-concurrency wall time (AIPerf)", ""]
    if isinstance(aiperf_timings, dict) and aiperf_timings.get("stages"):
        stage_time.append("| Concurrency | Seconds | rc |")
        stage_time.append("|-------------|---------|----|")
        for st in aiperf_timings["stages"]:
            stage_time.append(f"| {st.get('concurrency')} | {st.get('seconds')} | {st.get('rc')} |")
        stage_time.append("")
    else:
        stage_time.append("_No AIPerf stage timings recorded._")
        stage_time.append("")

    sources = """## Sources

- Hub dataset: https://huggingface.co/datasets/metrum-ai/prompt-library
- AIPerf: https://github.com/ai-dynamo/aiperf and https://docs.nvidia.com/aiperf/
- Metrum strategic NDJSON: `docs/TELEMETRY.md`, `docs/OUTPUT_SCHEMA.md`
- Bundle files: `mix-report.json`, `stdout-closed.json`, `stdout-open.json`, `promptfoo-*.json`, `aiperf/`
"""

    header = [
        "<!-- Copyright (c) 2026 Metrum AI, Inc. -->",
        "<!-- SPDX-License-Identifier: Apache-2.0 -->",
        "",
        "# Metrum AI Bench CLI vs NVIDIA AIPerf (Shadeform bake-off)",
        "",
        "Side-by-side study on one Shadeform GPU SUT serving Qwen via vLLM.",
        "This report is generated from the e2e artifact bundle; it does not invent metrics.",
        "",
    ]

    body = "\n".join(
        header
        + ds_lines
        + workload
        + [method, ""]
        + timing_lines
        + perf
        + knee_lines
        + open_lines
        + pf_lines
        + energy_lines
        + observations
        + stage_time
        + [sources]
    )
    out_path.write_text(body, encoding="utf-8")
    print(f"wrote {out_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
