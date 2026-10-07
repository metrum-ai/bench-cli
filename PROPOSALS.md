<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Proposals

Design notes for changes that are not implemented yet. Each proposal lists the
motivation, a sketch of the interface, and compatibility notes. Nothing here is
a committed roadmap; `--help` and the source remain authoritative.

## 1. `--fail-on-isl-mismatch`

**Motivation.** `--fail-on-osl-mismatch` gates output length, but input length
has no equivalent. ISL mismatches are counted in the summary and written to the
per-request log, so a drifted prompt pool can still pass a run that only gates
OSL. Campaign runners currently gate ISL themselves from strategic NDJSON
request rows.

**Sketch.** Add `--fail-on-isl-mismatch` next to `--fail-on-osl-mismatch` in
the shared tolerance flags. A successful request mismatches when
`abs(measured_input_tokens - isl_target) > isl_tolerance`. Mirror the OSL gate
semantics: modality binaries compare every measured success; the strategic
runner compares the last stage only and writes summary, CSV, and HTML before
exiting nonzero. Emit the same mismatch count and exit code shape.

**Compatibility.** Additive flag, default off, so existing runs are unchanged.
Both `--isl-target` and `--isl-tolerance` are already present. The main open
question is which token source feeds the comparison (`usage.prompt_tokens`
versus a tokenizer fallback) and how that interacts with proposal 3.

## 2. Tolerances default from mix report or profile

**Motivation.** `--isl-tolerance` and `--osl-tolerance` default to `0.0`.
Without a mix report, zero means exact matching, which is rarely intended and
silently fails a drifted run. With `--prompt-mix-report`, `IslOslTargets::resolve`
treats zero as a sentinel and uses the report's tolerances, so the same flag
value behaves two different ways depending on other flags.

**Sketch.** Make the tolerance flags `Option<f64>`. Resolve them in one place:
an explicit CLI value wins; otherwise use the mix report tolerance; otherwise
use the named profile tolerance when a profile is active; otherwise require the
operator to pass a value (or fall back to a documented, non-zero default) rather
than treating zero as exact. Record the resolved source in the summary config.

**Compatibility.** This changes resolved behavior for runs that relied on the
zero sentinel, so it needs a minor version and a CHANGELOG entry. The clap
surface stays the same because `Option<f64>` still accepts an explicit value.
The proposal is intentionally separate from the publishable-default
documentation, which already tells operators to pass both tolerances.

## 3. `--isl-token-basis server` via `/tokenize`

**Motivation.** `--isl-token-basis` currently accepts only `supplied-target`,
which reads the dataset's `target_input_tokens` and excludes the generation
hint and the server chat template. Runtime ISL is defined by the server
tokenizer and template, so the supplied target can be tens of tokens away from
the measured `usage.prompt_tokens`. Operators calibrate by hand today.

**Sketch.** Add a `server` basis that calls the server's `/tokenize` endpoint
(vLLM exposes one; confirm per engine and version) for each rendered prompt and
stores the returned token count in the mix report. Fall back to supplied-target
with a clear warning when the endpoint is missing or returns an unexpected
shape. Record the tokenizer revision and template in the report.

**Compatibility.** Additive enum value. Engines without `/tokenize` keep the
current behavior through the fallback. This basis makes ISL selection
network-dependent, so offline selection still needs supplied-target.

## 4. `--filter-isl-window` with a minimum distinct-row count

**Motivation.** The selector can hit a mean or median ISL target with a wide
spread, including repeated rows. A campaign that wants a bounded ISL window
cannot express it today and must filter JSONL after selection, which can drop
the achieved mix below the target.

**Sketch.** Add `--filter-isl-window MIN:MAX` with
`--filter-isl-min-distinct N`. Filter candidate rows to the window before
selection, then require at least `N` distinct source rows so repeats cannot
satisfy the window with one row. Fail with the existing diagnostics shape when
the window cannot be met inside `--count-slack`.

**Compatibility.** Additive and off by default. When the window is too narrow
for the dataset, the tool fails before writing output, matching the current
failure behavior.

## 5. Modality `--ndjson` and `--telemetry` already exist

**Motivation.** Documentation and older scripts implied that only the strategic
binary could write NDJSON or scrape Prometheus telemetry, and that modality
binaries needed an external sidecar. That is wrong: the LLM, VLM, ASR, and
imagegen binaries flatten `TelemetryArgs` and accept both flags.

**Sketch.** This is a documentation item, not a new flag. State that the
modality binaries accept `--ndjson` and `--telemetry` and that the sidecar is
optional. Keep the sidecar for scraping GPU series outside the benchmark
process or when the process cannot reach the exporter.

**Compatibility.** No code change. This note exists so future proposals do not
re-add an already-shipped flag.

## 6. `metrum-ai-bench-cli campaign check <dir>`

**Motivation.** A multi-host campaign produces many runs, logs, and telemetry
sidecars across directories. Checking completeness and validity is currently a
pile of shell. The campaign skill describes the checks but does not ship a
binary subcommand.

**Sketch.** Add `metrum-ai-bench-cli campaign check <dir>` that walks a campaign
directory and verifies, per cell: a summary exists, successes equal requested
with zero errors, every request sits inside the declared ISL and OSL windows,
the SUT block is present and complete, the telemetry recorder span covers the
sweep, and a checksum manifest is present. Print a table and exit nonzero on
any failed cell. Read the campaign manifest that `intake` already writes; do
not invent a second schema.

**Compatibility.** New subcommand, so no existing invocation changes. It needs
a campaign directory layout to be specified and versioned before implementation.
