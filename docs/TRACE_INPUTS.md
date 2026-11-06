<!-- Copyright (c) 2026 Metrum AI, Inc. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Structural trace input building blocks

This is an early source-stage library slice, not a registered trace file adapter,
canonical runtime record exporter, scheduler or serving/cache measurement.
The frozen [canonical runtime contract](CANONICAL_INPUTS.md) is unchanged.

## Exact source time

`trace_inputs::normalize_trace_time` accepts an exact nonnegative decimal string,
a declared `TraceTimeUnit`, positive rational `TraceTimeScale`, and explicit
`FractionalMillis` policy. Units are seconds, milliseconds or microseconds.
No magnitude-based unit guessing, epoch interpretation, rebasing, sorting, jitter
or actual scheduling occurs. The caller must declare the source's time origin and
perform any checked origin subtraction before assigning runtime arrival offsets.
For fractional milliseconds, origin subtraction must use exact source values
before rounding; subtracting independently floored values is not equivalent.

The parser retains the original string (including leading zeroes), permits at
most 18 fractional digits and 64 bytes, rejects signs, exponent notation,
whitespace and malformed values, and uses checked u128 rational arithmetic.
Cross-cancellation limits intermediate growth. Unsupported arithmetic range or a
result exceeding u64 fails; nothing is silently clipped or wrapped.

- `92.516` seconds becomes exactly `92516` milliseconds.
- `1.5` seconds scaled by `2/3` becomes exactly `1000` milliseconds.
- `1250` microseconds has sub-millisecond precision: `Reject` fails; explicitly
  selecting `Floor` produces `1` and records `discarded_fraction: true`.

`NormalizedTraceTime` retains source value/unit, scale, rounding policy, normalized
milliseconds and the loss flag. It is source metadata, not an actual arrival or
an issuance observation. Actual scheduling drift/cap engagement belong to runtime
reports. A source timestamp/identifier may itself be private: serialization is
not automatic permission to publish it.

An eventual JSON/CSV reader must preserve the original numeric lexeme. Parsing
through f64 and converting back to a string can lose distinctions before this
helper sees them. Unknown/missing units must not acquire a default from magnitude.

## Remaining adapter requirements

This helper does not ingest or verify any external trace assets, establish their
license/privacy eligibility, infer sessions, reconstruct prefix chains or recover
original text. A source adapter must pin and verify its actual files and rights,
enforce byte/row limits, preserve stable lineage/order and document field mapping.
Prefix-parent chains and session dependencies require separate validation.
Partial-tail and full-block conventions must be explicit. Length/time-only inputs
retain unknown overlap; no session ID or timestamp proves prefix reuse.

Hash-to-text construction must be separately versioned, labeled constructed rather
than recovered text, and checked with the target tokenizer/template. Post-template
source hashes cannot silently receive a second chat framing pass. Logical/modelled
cache behavior never becomes observed physical cache-hit or eviction evidence.

Tests use synthetic exact-decimal values only and require no remote assets.
