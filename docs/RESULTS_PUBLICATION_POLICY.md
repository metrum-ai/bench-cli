<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Results publication policy

Status: Approved September 21, 2026. Owner: CEO.

This policy covers how results produced with Metrum AI Bench CLI may be
published and when they may be described as Metrum AI Bench CLI results.

## 1. Ownership and endorsement

Harness results belong to the **publisher** of those results. Running or citing
Metrum AI Bench CLI does **not** constitute endorsement by Metrum AI, Inc.
unless Metrum AI is separately engaged in writing for that purpose.

## 2. Manifest requirement

A published result that **names Metrum AI Bench CLI** (including "Measured
with Metrum AI Bench CLI") must include an **unmodified run summary** that
contains at least:

- Tool version
- Workload identification
- Client environment
- **SUT block** with publication inventory: `gpu.model`, `gpu.count` (>0),
  `driver_version`, `runtime.name`, `runtime.version`, `runtime.config`
  (exact launch command or serving flags), and `host_os`
- Full configuration used for the run

Produce a compliant run with:

```text
--sut <file> --require-sut
```

`--require-sut` refuses incomplete SUT declarations before any request is sent.

For token-length-matched compares, also set `--isl-target` / `--osl-target`
(or `--prompt-mix-report`) with `--fail-on-osl-mismatch` and a bounding
`--max-tokens`.

A claim that omits this manifest (including the SUT block) is **not** a
Metrum AI Bench CLI result under this policy, even if the software was used.

## 3. Trademark use

Trademark use in connection with published results follows
[TRADEMARKS.md](../TRADEMARKS.md).

## 4. Metrum's own studies

Studies published by Metrum AI must meet the same manifest requirements as
third-party publications.

## 5. Vendor non-suppression

Vendors and partners **may not** require suppression or delay of third-party
results obtained with Metrum AI Bench CLI as a condition of access,
partnership, or support.

## 6. Declared vs observed

Where a field is supplied by declaration rather than measured by the harness,
it must be labeled accordingly (for example, `provenance: "declared"`, or
`field_provenance` entries of `"observed"` from `sut init --probe` for local
host facts only). Do not present declared values as observed measurements, and
do not treat local probe output as verification of a remote serving host.

## 7. Reporting incorrect results

If you believe a published result misuses the Metrum AI Bench CLI name, omits a
required manifest, or is otherwise incorrect under this policy, report it to
[opensource@metrum.ai](mailto:opensource@metrum.ai) with links to the
publication and any available run artifacts.
