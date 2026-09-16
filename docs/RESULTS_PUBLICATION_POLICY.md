<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Results publication policy

Status: DRAFT — pending counsel review (C3). Owner: CEO.

This policy covers how results produced with Metrum AI Bench may be published
and when they may be described as Metrum AI Bench results.

## 1. Ownership and endorsement

Harness results belong to the **publisher** of those results. Running or citing
Metrum AI Bench does **not** constitute endorsement by Metrum AI, Inc. unless
Metrum AI is separately engaged in writing for that purpose.

## 2. Manifest requirement

A published result that **names Metrum AI Bench** (including “Measured with
Metrum AI Bench”) must include an **unmodified run summary** that contains at
least:

- Tool version
- Workload identification
- Client environment
- **SUT block**
- Full configuration used for the run

Produce a compliant run with:

```text
--sut <file> --require-sut
```

A claim that omits this manifest (including the SUT block) is **not** a
Metrum AI Bench result under this policy, even if the software was used.

## 3. Trademark use

Trademark use in connection with published results follows
[TRADEMARKS.md](../TRADEMARKS.md).

## 4. Metrum’s own studies

Studies published by Metrum AI must meet the same manifest requirements as
third-party publications.

## 5. Vendor non-suppression

Vendors and partners **may not** require suppression or delay of third-party
results obtained with Metrum AI Bench as a condition of access, partnership, or
support.

## 6. Declared vs observed

Where a field is supplied by declaration rather than measured by the harness,
it must be labeled accordingly (for example, `provenance: "declared"`). Do not
present declared values as observed measurements.

## 7. Reporting incorrect results

If you believe a published result misuses the Metrum AI Bench name, omits a
required manifest, or is otherwise incorrect under this policy, report it to
[opensource@metrum.ai](mailto:opensource@metrum.ai) with links to the
publication and any available run artifacts.
