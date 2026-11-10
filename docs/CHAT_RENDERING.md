<!-- Copyright (c) 2026 Metrum AI, Inc. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Pinned local chat rendering

With the `tokenizer` feature, `chat_rendering::render_wire_body` renders an exact
final HTTP JSON request body using an operator-reviewed, pinned local chat template
and local tokenizer. It complements [rendered-token validation](PREFIX_VALIDATION.md);
it does not issue requests or alter runner defaults.

## Integrity boundary

The caller supplies the final body bytes **after** all transformations, history
replacement, hints/nonces and cache mutation. The renderer hashes those exact
bytes as `body_sha256`. Whitespace changes to the HTTP JSON serialization change
this hash even when parsed messages are identical. `rendered_text_sha256` instead
hashes post-template text; these are deliberately different bases. `token_ids`
are the encoding of the rendered text, not character counts or supplied targets.

`RenderingIdentity` pins tokenizer ID/revision and exact artifact SHA-256 values,
chat-template SHA-256, special-token and generation-prompt settings, hint/nonce
inclusion and renderer version. Template/tokenizer artifact mismatch fails before
rendering. The caller must ensure the named revision actually produced those
artifacts; a file digest alone does not authenticate an upstream revision.

The body must have a nonempty `messages` array. Template context includes those
messages, optional `tools` and `documents` (null if absent), and the declared
`add_generation_prompt`. Additional operator-supplied special-token bindings such
as `bos_token`/`eos_token` cannot override those reserved fields. Role, tool pairing
and causal history must be validated by canonical adapters/runtime beforehand.

Renderer identity `minijinja-chat.v2` normalizes assistant function-call arguments
from serialized OpenAI JSON strings to HF-style JSON objects in template context
only. Already-object arguments are preserved; malformed/scalar arguments and
unsupported tool types fail without echoing arguments. Tools, call IDs, tool-result
content and message order remain inert data; no tool is executed. Generation-prompt
framing is controlled explicitly by identity, never inferred from the final role.
This changes rendering semantics from v1 and therefore requires a new renderer
identity; the public function signature and canonical message types are unchanged.

Object insertion order is preserved from the actual wire JSON, including decoded
function-argument objects, without changing canonical serde serialization. The
bounded `tojson` filter implements the default HF subset: spaced delimiters,
Unicode retained, insertion order retained and no HTML escaping. Explicit default
`ensure_ascii=false`, `sort_keys=false`, null indent/separators are accepted.
Nondefault formatting options, unknown keywords and floating-number serialization
are rejected rather than approximated (Python float spellings can differ).

HF template compatibility must be established for the exact pinned template;
passing synthetic HF-style goldens does not establish universal Python/Jinja
compatibility. Templates requiring unsupported JSON options fail closed. Unsupported features fail, and backend
cross-check mismatches must be Unknown. The runtime owns the exact issued-wire
hash even though this helper returns a convenience body digest.

No implicit tokenizer truncation or padding is accepted. Oversized or unsupported
inputs fail rather than silently clipping. Encoding does not prove serving-window
fit or that a particular backend applies the same chat framing. Runtime must
match the exact tokenizer/template/formatting policy or mark validation unknown.

## Template execution policy and limits

This optional feature uses MiniJinja 3.0.5 (Apache-2.0, upstream declared MSRV 1.70),
locked in Cargo.lock. Only local reviewed template text is evaluated. No loader,
filesystem/network callback, shell function or tool executor is installed. Dataset
scripts and ingested tool calls never execute. Unsupported Python/Jinja features,
functions or filters fail; there is no fallback template or remote execution.
`raise_exception` is supported only as a generic rejection; error diagnostics do
not echo body text, tool arguments or template-provided exception messages.

Artifact bytes, HTTP body bytes, rendered output bytes, encoded tokens, template
instruction fuel and recursion depth are bounded. A bounded writer stops excessive
output. Fuel/output limits are **not a general memory sandbox**: one template
expression may allocate intermediate data. Therefore templates are trusted,
operator-reviewed pinned artifacts, not arbitrary code accepted from dataset rows.
Budget failures must not be relabeled validated targets.

## Integration and fixtures

The returned exact body digest and token IDs can populate `RenderedBody` for the
separate warmup isolation interface. Keep the same final wire bytes until issuance;
any later mutation invalidates that evidence. Input-profile generation may use the
same helper locally, but cannot certify the backend's actual wire rendering.

The offline BPE/template fixture checks actual template evaluation, token-boundary
merging, distinct wire/rendered hash bases, artifact checks, reserved-key denial,
output/token budgets and unsupported-function rejection. No Hub download or model
inference is involved. Public evidence should export aggregate validation and
fingerprints, not real-user message text, tool arguments or raw token IDs.
