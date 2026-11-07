<!-- Copyright (c) 2026 Metrum AI, Inc. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Structural trace input building blocks

This source-stage library provides exact time normalization and a declarative
verified JSONL adapter, not a catalog registration, canonical runtime exporter,
runtime scheduler or serving/cache measurement.
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

The JSONL adapter preserves numeric time with serde RawValue; other readers must
likewise preserve the original numeric lexeme. Parsing
through f64 and converting back to a string can lose distinctions before this
helper sees them. Unknown/missing units must not acquire a default from magnitude.

## Verified JSONL structural source adapter

`trace_adapters::normalize_jsonl` checks origin membership, exact bytes/length/SHA,
reviewed rights policy and source/row/chain/aggregate expansion limits before
returning `structural-source-trace.v1` ([JSON Schema](../schemas/structural-source-trace.v1.json)).
The private artifact retains origin, mapping, license label, rights-policy digest,
source-file digest and per-row declared full-parent reuse. `metadata` exports only
counts/digests and explicit unknown/unavailable status; it omits source IDs, raw
times and hash topology. Policy fingerprints are not independent legal clearance.
Mapping declares source domain, identity/length/time/hash fields, block size,
units/scale/rounding, hash coverage and encoding. Explicit null row-ID field uses
`line:N` source positions, never claims a publisher identifier. Declared optional
metadata columns remain private attributes rather than guessed dependencies.
Root sentinels (such as -1) are explicit; null already denotes no parent.

Optional selection declares physical start line and maximum selected rows. The
entire pinned file is checksum/length verified BEFORE selection; source physical
line count is not a full validated record/corpus count. Selected graphs still
must be dependency-closed. Limits without selection reject excess rows rather
than silently truncating. Selection is retained in mapping/fingerprints. Nothing is
inferred from field names, timestamp magnitude or input length. Source output
lengths remain u64 source metadata, including zero/out-of-runtime-range values;
there is no unchecked narrowing to runtime output targets.

Complete-chain or parent-delta encoding is explicit. Deltas reuse only the declared
number of known full parent blocks, never a parent partial tail. Full-block-only,
including-partial-tail and partial-prefix coverage have distinct count checks.
Prefix and sequential-session parents are independent fields; each graph rejects
missing parents/cycles/depth over 256. File order is preserved even for forward
parents or decreasing times. Prefix links do not fabricate conversation history.

Interned node identity includes the full parent path: equal raw symbols below
different parents are not the same prefix node. Hashes are opaque symbols, not
original token IDs or recovered text. Partial-tail symbols are separate digests,
not full reusable blocks. Missing hash mappings keep known prefix nodes absent;
length/time-only inputs retain overlap Unknown. All adapter outputs also keep
rendered-token overlap Unknown and physical-cache evidence unavailable, even when
source full-block topology is known. Time conversion is not origin rebasing or
actual issuance. Source records/identifiers remain private by default.

Actual external assets still require pinned file checksums and applicable rights/
privacy review; synthetic tests do not establish this. `github_source::materialize`
uses explicitly public GitHub paths/revisions, an anonymous client (never Hub
headers), restricted redirects, rights/byte limits, and the shared checksum-copy/
cache verifier. Corrupt cache entries fail closed; no mirror/source-script fallback.

`trace_presets::qwen_trace_a` pins source revision/file bytes/LFS digest from the
published metadata and uses 16-token post-template blocks with explicit rounding
and selected rows. Optional chat/parent/type/turn fields are retained as attributes,
not inferred prefix or sequential dependencies. Named pins are not rights approval,
corpus validation, token recovery or general catalog eligibility.

The opt-in `trace_source_probe` example verifies the full asset before selecting
256 rows and emits metadata only. Keep the content cache outside the repository:

```sh
cargo run --locked --example trace_source_probe -- \
  --accept-source-terms ../trace-cache
# Re-verify the same pinned cache without network:
cargo run --locked --example trace_source_probe -- \
  --accept-source-terms ../trace-cache --offline
```

The acceptance flag acknowledges the reviewed source policy; it cannot override
license/access gates for other sources. The profile preserves publisher `chat_id`
as source identity and physical line position separately. No source IDs, hashes,
raw timestamps or original text are printed by the metadata summary.

Hash-to-text construction must be separately versioned, labeled constructed rather
than recovered text, and checked with the target tokenizer/template. Post-template
source hashes cannot silently receive a second chat framing pass. Logical/modelled
cache behavior never becomes observed physical cache-hit or eviction evidence.

Tests use synthetic exact-decimal values only and require no remote assets.
