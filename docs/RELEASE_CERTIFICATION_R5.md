<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# KV certification catalog r5

BC-KV-001 through BC-KV-015 are core, per-release gates. The nine live cases
are RC-GPU; the six offline data/export cases are RC. All must be selected,
executed against the frozen candidate, judged by `validate_results.py` and
included in the certification sheet. Existing rows and thresholds are unchanged.
Missing, FAIL, PENDING and smoke evidence cannot satisfy these gates. Unknown
metric values remain Unknown; they never become zero or a physical cache claim.

The catalog records proposed image descriptors, not proof of executable source
identity or runtime support. SGLang, LMCache, Dynamo and TensorRT-LLM carry
`TBD-pin` until source review, matching exposition fixtures and runtime checks
resolve their compatibility. Selection keeps these mandatory cases and marks
them `TBD-pin-not-executable`. Execution preflight and result validation reject
any row containing `TBD-pin`, including a submitted PASS or N-A. Removing a
required case from the frozen selection also fails policy recomputation.

The proposed vLLM index and amd64 child are recorded in each applicable row.
For every engine verify the descriptor chain, actual package/source revision,
model files, tokenizer and collector at execution freeze. Publisher labels alone
cannot establish source compatibility. No RC or development engine image is
eligible. TensorRT-LLM uses standalone stable 1.2.1, independently of Dynamo;
Dynamo KVBM uses stable 1.6.2 with the vLLM runtime and requires proof that its
conditional KVBM package is installed and operational.

Prefer Qwen/Qwen3.8-27B-FP8 at
`c3a462cc3d38cf351f3b26cbd763469eeb74ef81`. For an unsupported engine select
the newest verified supported official Qwen and document the reason and measured
fit. Qwen3-8B is a flagged last resort. Every live row allows H100 80 GB or RTX
PRO 6000 Blackwell only after actual runtime, kernel and KV headroom checks.
BC-KV-002 additionally needs at least 64 GiB host RAM. Its entire exclusive L1
window, including applicable setup and cleanup, remains bounded by two GPU-hours.
No other case inherits that bound.

Freeze exact stimuli, numerical sample counts, flags, schemas, hashes, source
profiles, ownership, time limits and cleanup before a run. The catalog's minute
values are planning estimates, not measured evidence. APC records timing without
a speedup threshold. L1 retains all four conditions: completed CPU-load-byte
delta >0, GPU-prefix-hit-token delta exactly 0, re-hit TTFT < cold TTFT, and
independent target-prefix GPU eviction evidence in an exclusive settled window.
SGLang hit-rate is a rate, LMCache store is not occupancy, and block counts do
not imply bytes. Optional unsupported tier evidence stays Unknown.

CPU cases use no server, model or GPU. Bind public prompt-library revision
`0666f62e581b482838ae2e17b333ee36ff3d01b0`, W2 digest
`e5480af17428c48e614a0840f30eda53acbea7f9efe869c5173cde5d5818edfd`, selected
file/manifest hashes, interpreter and dependencies. HF inputs require immutable
revisions and file SHA-256 checks. Remote code is prohibited. Freeze small
synthetic negative fixtures separately. Mock cache state in BC-KV-015 proves
export correctness only. Offline rehearsals are never certification evidence.

Changing the catalog or harness changes the candidate. Rebuild workflow-origin
artifacts, refreeze the full selection and execution requirements, and use the
existing reviewed carry-over process for any earlier results; a dry run or an
earlier r4 result does not establish an r5 PASS.
