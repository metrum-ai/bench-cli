<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# AGENTS.md

Vendor-neutral entry point for AI coding agents working in this repository.
Read this before you drive a benchmark. Human contributor build and style rules
live in [CLAUDE.md](CLAUDE.md).

## What this repository is

Metrum AI Bench CLI is a **client**. It measures the latency, throughput, and
quality of OpenAI-compatible endpoints for LLM, VLM, ASR, and image
generation. It does not serve models, does not host GPUs, and does not install
a serving engine for you. Choosing and running the serving stack is your job
and the operator's.

- Install and quickstart: [README.md](README.md)
- Generated flag reference: [docs/CLI.md](docs/CLI.md)
- What is actually verified, per modality: [docs/CLAIMS_LEDGER.md](docs/CLAIMS_LEDGER.md)

## Default campaign

Run these steps in order for any multi-host or publishable measurement. Each
step links to its section in [docs/AGENT_RUNBOOK.md](docs/AGENT_RUNBOOK.md)
(that file lands in a later commit; link the path regardless).

1. **Intake** - collect modality, model, engine, host, budget, and target
   metrics from the operator. [docs/INTAKE.md](docs/INTAKE.md) and
   [docs/AGENT_RUNBOOK.md#intake](docs/AGENT_RUNBOOK.md#intake)
2. **Provision or verify hosts** - bring up or confirm the GPU hosts and
   record their identity. [docs/AGENT_RUNBOOK.md#provision-or-verify-hosts](docs/AGENT_RUNBOOK.md#provision-or-verify-hosts)
3. **Bootstrap** - install the runtime, drivers, and prebuilt binaries on the
   host. [docs/AGENT_RUNBOOK.md#bootstrap](docs/AGENT_RUNBOOK.md#bootstrap)
4. **Serve** - search upstream docs, then start the engine with recorded
   flags. [docs/AGENT_RUNBOOK.md#serve](docs/AGENT_RUNBOOK.md#serve)
5. **Telemetry** - start exporters and scrape them; set the YAML series list
   from the live response. [docs/AGENT_RUNBOOK.md#telemetry](docs/AGENT_RUNBOOK.md#telemetry)
6. **Prompt pool** - extract a publishable prompt mix from the prompt library.
   [docs/AGENT_RUNBOOK.md#prompt-pool](docs/AGENT_RUNBOOK.md#prompt-pool)
7. **Sweep** - run the concurrency or rate sweep at the chosen ISL/OSL.
   [docs/AGENT_RUNBOOK.md#sweep](docs/AGENT_RUNBOOK.md#sweep)
8. **Validate** - check exit codes, error counts, and target-length gates.
   [docs/AGENT_RUNBOOK.md#validate](docs/AGENT_RUNBOOK.md#validate)
9. **Copy** - copy the data log and summary off the host before cleanup.
   [docs/AGENT_RUNBOOK.md#copy](docs/AGENT_RUNBOOK.md#copy)
10. **Report** - assemble the summary and SUT block for publication.
    [docs/AGENT_RUNBOOK.md#report](docs/AGENT_RUNBOOK.md#report)
11. **Clean up** - stop the engine and tear down hosts, confirming each
    deletion. [docs/AGENT_RUNBOOK.md#clean-up](docs/AGENT_RUNBOOK.md#clean-up)

## Hard rules

- **Run load from the GPU host.** The benchmark binaries run on the GPU host
  and are downloaded there. Never drive load from the agent machine, even
  when the endpoint is reachable from it.
- **Prebuilt releases only.** Never source-build bench-cli, all-smi, or the
  serving engine. Download release binaries onto the GPU host and record each
  binary path and `--version` in the SUT notes.
- **Never expose secrets.** Do not print, log, or commit API keys, tokens, or
  other credentials. Do not put credentials in files committed to source
  control.
- **Check every exit code.** Treat a non-zero exit as a failure and stop the
  campaign until it is understood. Do not continue past a failed step.
- **Never invent a number.** Every figure must trace to a run summary plus its
  per-request log. If you do not have both, you do not have a number.
- **Do not overclaim.** Do not claim more than
  [docs/CLAIMS_LEDGER.md](docs/CLAIMS_LEDGER.md) allows. "Supported in code" is
  not "verified live".

## Defaults

Propose defaults for modality, model, engine, host, ISL/OSL, concurrency, and
budget, then get explicit operator confirmation before spending money or
starting load. The intake form and its defaults are in
[docs/INTAKE.md](docs/INTAKE.md). The default campaign is LLM chat only unless
the operator selects other modalities in intake.

## Truth order

When sources disagree, resolve in this order:

1. `--help` output beats docs.
2. Docs in this repository beat memory or prior runs.
3. Upstream engine docs and model cards beat this repository for serving
   flags and model settings.

Search online for the current vendor documentation for the exact model,
engine, and engine version before every real run. Do not reuse memorized
launch flags. Record sources and chosen arguments in the SUT
`runtime.config` and `notes`.
