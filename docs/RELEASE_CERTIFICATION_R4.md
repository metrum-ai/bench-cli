<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# v1.9.0 certification catalog r4 model plan

This is a proposed, not executed, release plan. The model update follows a
maintainer decision; no acceptance threshold is relaxed. The previous-release
BC-PERF-001 configuration is unchanged. Image and model pins are frozen in each
changed catalog row; an image's architecture support is not proof that its
parser, offload connector, capacity, or image path works for a workload.

- **A — general text and vision**: `Qwen/Qwen3.8-27B-FP8`, revision
  `c3a462cc3d38cf351f3b26cbd763469eeb74ef81`, official FP8 checkpoint.
- **B — deliberately small model**: `Qwen/Qwen3.5-0.8B`, revision
  `2fc06364715b967f1860aea9cf38778875588b17`, smallest current official Qwen
  generation found with a small-model variant. Validate reasoning/endpoint and
  comparator behavior against the frozen workload; size alone is not proof.
- **Fit boundary:** the upstream Qwen3.8 recipe states FP8 weight files are
  30,866,866,928 bytes and gives a 38 GB minimum derived from weights plus a
  heuristic. It lists H100 but does not list RTX PRO 6000 for this FP8 variant.
  H100 is the proposed reference class. RTX PRO is a tentative alternative,
  blocked until the exact checkpoint, kernel, runtime and KV headroom are
  independently verified; nominal 96 GB alone is not a fit proof.
- **I — A/B image:** `vllm/vllm-openai:v0.33.0` OCI index
  `sha256:d825baf9ce4b1b6bfc5ecc7b79880c7e6748c65a72d697ce3d47607aafa84c6e`;
  amd64 child `sha256:91ee972ab751fb7180bc71c175865e36b091686600dca783d52b2511399810b3`.
  Record both index and actual platform child. The public release's GitHub tag
  is `v0.33.0rc3` despite title `v0.33.0`; its architecture registry contains
  `Qwen3_5ForConditionalGeneration`. Runtime validation remains required.

## Model and proposed GPU fit by case

| Cases | Model | Proposed class / special constraint |
| --- | --- | --- |
| BC-LIVE-001; BC-REASON-001/002/003/004/006; BC-COMP-002; BC-INTEG-001; BC-GPU-001–010/012–014/016; BC-COMP-003 | A + I | H100 80 GB; RTX PRO 6000 96 GB only after the compatibility check above. FP8 weight size is not a measured total; verify loaded model and KV headroom at each frozen concurrency. BC-INTEG-001 remains subject to its recorded N/A. |
| BC-LIVE-002; BC-REASON-005; BC-GPU-011 | A + I | H100 80 GB; RTX PRO 6000 96 GB only after the compatibility check above. Send actual image inputs and verify thinking, image handling and parser compatibility. No text-only substitute. |
| BC-GPU-015 | B + I | One >=48 GB NVIDIA GPU; two independent small-model servers with the existing 0.45 allocation each. Verify both fit and retain the real kill/failover criteria. |
| BC-PERF-003 | B + I | H100 or L40S, with frozen small-model W1/W2 A/B configuration and unchanged six-point thresholds. |
| BC-HW-001 | A + I | H100 80 GB, distinct from the RTX PRO 6000 Blackwell performance lane; all existing controls must pass. |
| BC-PERF-001; BC-PERF-002; BC-COMP-001; BC-TELEM-001 | Existing pinned performance specs | RTX PRO 6000 Blackwell; the earlier-release gates and their exact configurations are unchanged. |
| BC-LIVE-003; BC-LIVE-004 | Existing pinned ASR and image-generation models | RTX PRO 6000 or H100; modality-specific models are not text or vision-language substitutes. |
| BC-OPS-001 | None | All allocated classes; deletion proof remains required. |

The two N/A decisions and all case selection and sign-off requirements remain
separate. Cases not listed are non-GPU catalog cases. This list is planning,
not an assertion that each case has been selected or that memory fit is proven.

Sources: [official Qwen FP8 metadata](https://huggingface.co/api/models/Qwen/Qwen3.8-27B-FP8),
[official small Qwen metadata](https://huggingface.co/api/models/Qwen/Qwen3.5-0.8B),
[official vLLM image metadata](https://hub.docker.com/v2/repositories/vllm/vllm-openai/tags/v0.33.0),
and [Qwen3.8 vLLM recipe](https://github.com/vllm-project/recipes/blob/main/models/Qwen/Qwen3.8-27B.yaml).
