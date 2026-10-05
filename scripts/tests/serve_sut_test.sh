#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Metrum AI Bench CLI: offline self-test for the SUT that
# scripts/live/serve/*.sh writes. Runs each launcher's `sut` subcommand
# (no docker, no GPU, HF_HUB_OFFLINE=1 so no network) and checks
# model.quantization, notes, and the MODEL, IMAGE, and SERVE_ARGS_OVERRIDE
# guards.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SERVE="${ROOT}/scripts/live/serve"
work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT
export HF_HUB_OFFLINE=1
unset MODEL IMAGE ASR_STACK QUANTIZATION SUT_NOTES_OVERRIDE SOURCES_OVERRIDE SERVE_ARGS_OVERRIDE

failures=0
fail() { echo "serve_sut_test: FAIL $*"; failures=$((failures + 1)); }

# check <label> <jq filter that must be true> <launcher> [VAR=value ...]
check() {
  local label="$1" filter="$2" launcher="$3"; shift 3
  if ! env "$@" "${SERVE}/${launcher}" sut >"${work}/sut.json" 2>"${work}/err.txt"; then
    fail "${label}: launcher exited non-zero"; cat "${work}/err.txt"; return 0
  fi
  jq -e "${filter}" "${work}/sut.json" >/dev/null || { fail "${label}: ${filter}"; cat "${work}/sut.json"; }
}

# reject <label> <stderr substring> <launcher> [VAR=value ...]
reject() {
  local label="$1" want="$2" launcher="$3"; shift 3
  if env "$@" "${SERVE}/${launcher}" sut >"${work}/sut.json" 2>"${work}/err.txt"; then
    fail "${label}: expected a non-zero exit"; return 0
  fi
  grep -qF -- "${want}" "${work}/err.txt" || { fail "${label}: stderr lacks ${want}"; cat "${work}/err.txt"; }
}

for l in llm vlm asr imagegen; do
  check "${l} default" '.model.quantization == null and .extra.quantization_source == "none" and (.notes | length > 0)' "${l}.sh"
done
check "llm default notes" '.model.id == "Qwen/Qwen3-8B" and (.notes | contains("Qwen3-8B"))' llm.sh

check "llm FP8 name" '.model.quantization == "fp8" and .extra.quantization_source == "model_name" and (.notes | startswith("researched FP8")) and (.notes | contains("fp8 derived from the MODEL name"))' \
  llm.sh MODEL=Qwen/Qwen3-8B-FP8 "SUT_NOTES_OVERRIDE=researched FP8" SOURCES_OVERRIDE=s
check "vlm AWQ name" '.model.quantization == "awq"' vlm.sh MODEL=Qwen/Qwen3-VL-8B-Instruct-AWQ SUT_NOTES_OVERRIDE=n SOURCES_OVERRIDE=s
check "GPTQ-Int4 prefers method" '.model.quantization == "gptq"' llm.sh MODEL=Qwen/Qwen3-8B-GPTQ-Int4 SUT_NOTES_OVERRIDE=n SOURCES_OVERRIDE=s
check "dot and underscore tokens" '.model.quantization == "w8a8"' llm.sh MODEL=x/Model_W8A8 SUT_NOTES_OVERRIDE=n SOURCES_OVERRIDE=s
check "no false match inside a word" '.model.quantization == null' llm.sh MODEL=x/awqward-fp80 SUT_NOTES_OVERRIDE=n SOURCES_OVERRIDE=s
# q <label> <MODEL> <want quantization as JSON>: name-derived marker cases.
q() { check "$1" ".model.quantization == $3" llm.sh "MODEL=$2" SUT_NOTES_OVERRIDE=n SOURCES_OVERRIDE=s; }
q "w8a16" x/Model-W8A16 '"w8a16"'
q "w4a16g128 one token" x/Model-w4a16g128 '"w4a16"'
q "w4a16-g128 split" x/Model-W4A16-G128 '"w4a16"'
q "nvfp4a16" x/Model-NVFP4A16 '"nvfp4a16"'
q "fp8e4m3" x/Model-fp8e4m3 '"fp8"'
q "int4wo" x/Model-int4wo '"int4wo"'
q "trailing slash" Qwen/Qwen3-8B-FP8/ '"fp8"'
q "rightmost marker wins" x/Model-FP8-to-BF16 null
q "weak only" nvidia/Llama-3.1-8B-Instruct-FP4 '"fp4"'
check "unquantized marker source" '.extra.quantization_source == "model_name" and (.notes | contains("model.quantization=null"))' \
  llm.sh MODEL=x/Model-FP8-to-BF16 SUT_NOTES_OVERRIDE=n SOURCES_OVERRIDE=s
check "QUANTIZATION trimmed" '.model.quantization == "fp8" and .extra.quantization_source == "env"' llm.sh "QUANTIZATION=  fp8 "
for v in None NONE null Null; do
  check "QUANTIZATION=${v}" '.model.quantization == null and .extra.quantization_source == "env"' llm.sh "QUANTIZATION=${v}"
done
reject "QUANTIZATION with inner space" "must be one word" llm.sh "QUANTIZATION=fp8 dynamic"
check "serve args flag" '.model.quantization == "fp8" and .extra.quantization_source == "serve_args"' \
  llm.sh "SERVE_ARGS_OVERRIDE=--quantization fp8 --max-model-len 32768" SUT_NOTES_OVERRIDE=n SOURCES_OVERRIDE=s
check "serve args flag=value" '.model.quantization == "awq"' llm.sh SERVE_ARGS_OVERRIDE=--quantization=awq SUT_NOTES_OVERRIDE=n SOURCES_OVERRIDE=s
check "QUANTIZATION env wins" '.model.quantization == "modelopt" and .extra.quantization_source == "env"' \
  llm.sh QUANTIZATION=modelopt MODEL=Qwen/Qwen3-8B-FP8 SUT_NOTES_OVERRIDE=n SOURCES_OVERRIDE=s
check "QUANTIZATION=none" '.model.quantization == null and .extra.quantization_source == "env"' \
  llm.sh QUANTIZATION=none MODEL=Qwen/Qwen3-8B-FP8 SUT_NOTES_OVERRIDE=n SOURCES_OVERRIDE=s

reject "llm override without notes" "set SUT_NOTES_OVERRIDE" llm.sh MODEL=Qwen/Qwen3-14B
reject "llm quantized override without notes" "set SUT_NOTES_OVERRIDE" llm.sh MODEL=Qwen/Qwen3-8B-AWQ
reject "vlm override without notes" "set SUT_NOTES_OVERRIDE" vlm.sh MODEL=Qwen/Qwen3-VL-32B-Instruct
reject "asr override without notes" "set SUT_NOTES_OVERRIDE" asr.sh MODEL=openai/whisper-large-v3
reject "imagegen override without notes" "set SUT_NOTES_OVERRIDE" imagegen.sh MODEL=x/other

# #216: IMAGE and SERVE_ARGS_OVERRIDE follow the MODEL rule (#212), and every
# override needs SOURCES_OVERRIDE.
reject "MODEL with notes, no sources" "set SOURCES_OVERRIDE (source URLs) for" llm.sh MODEL=Qwen/Qwen3-14B SUT_NOTES_OVERRIDE=n
reject "MODEL error names the model" "MODEL=Qwen/Qwen3-14B overrides the default Qwen/Qwen3-8B" llm.sh MODEL=Qwen/Qwen3-14B
reject "MODEL with sources, no notes" "set SUT_NOTES_OVERRIDE (researched notes) for" llm.sh MODEL=Qwen/Qwen3-14B SOURCES_OVERRIDE=s
check "MODEL with notes and sources" '.model.id == "Qwen/Qwen3-14B" and .notes == "n" and .extra.launcher_sources == "s"' \
  llm.sh MODEL=Qwen/Qwen3-14B SUT_NOTES_OVERRIDE=n SOURCES_OVERRIDE=s

reject "IMAGE without notes" "set SUT_NOTES_OVERRIDE (researched notes) and SOURCES_OVERRIDE (source URLs) for" llm.sh IMAGE=vllm/vllm-openai:v0.31.0
reject "IMAGE error names the image" "IMAGE=vllm/vllm-openai:v0.31.0 overrides the default vllm/vllm-openai:v0.30.0" \
  llm.sh IMAGE=vllm/vllm-openai:v0.31.0
reject "IMAGE with sources, no notes" "set SUT_NOTES_OVERRIDE (researched notes) for" llm.sh IMAGE=vllm/vllm-openai:v0.31.0 SOURCES_OVERRIDE=s
reject "IMAGE with notes, no sources" "set SOURCES_OVERRIDE (source URLs) for" llm.sh IMAGE=vllm/vllm-openai:v0.31.0 SUT_NOTES_OVERRIDE=n
check "IMAGE with notes and sources" '.runtime.version == "v0.31.0" and .extra.image == "vllm/vllm-openai:v0.31.0" and .notes == "n" and .extra.launcher_sources == "s"' \
  llm.sh IMAGE=vllm/vllm-openai:v0.31.0 SUT_NOTES_OVERRIDE=n SOURCES_OVERRIDE=s
for l in vlm asr imagegen; do
  reject "${l} IMAGE without notes" "set SUT_NOTES_OVERRIDE" "${l}.sh" IMAGE=x/other:v1
done
reject "asr omni IMAGE without notes" "overrides the default vllm/vllm-omni:v0.30.0" asr.sh ASR_STACK=omni IMAGE=x/other:v1
check "IMAGE equal to the default" '.extra.image == "vllm/vllm-openai:v0.30.0" and (.notes | contains("Qwen3-8B"))' \
  llm.sh IMAGE=vllm/vllm-openai:v0.30.0
check "asr omni default image" '.extra.image == "vllm/vllm-omni:v0.30.0"' asr.sh ASR_STACK=omni

reject "SERVE_ARGS_OVERRIDE without notes" "set SUT_NOTES_OVERRIDE (researched notes) and SOURCES_OVERRIDE (source URLs) for" llm.sh "SERVE_ARGS_OVERRIDE=--max-model-len 16384"
reject "SERVE_ARGS_OVERRIDE error names the flags" "overrides the default '--reasoning-parser qwen3 --max-model-len 32768'" \
  llm.sh "SERVE_ARGS_OVERRIDE=--max-model-len 16384"
reject "SERVE_ARGS_OVERRIDE with sources, no notes" "set SUT_NOTES_OVERRIDE (researched notes) for" llm.sh "SERVE_ARGS_OVERRIDE=--max-model-len 16384" SOURCES_OVERRIDE=s
reject "SERVE_ARGS_OVERRIDE with notes, no sources" "set SOURCES_OVERRIDE (source URLs) for" llm.sh "SERVE_ARGS_OVERRIDE=--max-model-len 16384" SUT_NOTES_OVERRIDE=n
check "SERVE_ARGS_OVERRIDE with notes and sources" '(.runtime.config | contains("--max-model-len 16384")) and .notes == "n"' \
  llm.sh "SERVE_ARGS_OVERRIDE=--max-model-len 16384" SUT_NOTES_OVERRIDE=n SOURCES_OVERRIDE=s
check "SERVE_ARGS_OVERRIDE equal to the default" '.notes | contains("Qwen3-8B")' \
  llm.sh "SERVE_ARGS_OVERRIDE=--reasoning-parser qwen3 --max-model-len 32768"
reject "vlm SERVE_ARGS_OVERRIDE without notes" "set SUT_NOTES_OVERRIDE" vlm.sh "SERVE_ARGS_OVERRIDE=--max-model-len 8192"
reject "all three without notes" "IMAGE=vllm/vllm-openai:v0.31.0 overrides" \
  llm.sh MODEL=Qwen/Qwen3-14B IMAGE=vllm/vllm-openai:v0.31.0 "SERVE_ARGS_OVERRIDE=--max-model-len 16384"
check "SOURCES_OVERRIDE alone keeps launcher notes" '.extra.launcher_sources == "s" and (.notes | contains("Qwen3-8B"))' llm.sh SOURCES_OVERRIDE=s

if (( failures > 0 )); then
  echo "serve_sut_test: ${failures} failure(s)"
  exit 1
fi
echo "serve_sut_test: ok"
