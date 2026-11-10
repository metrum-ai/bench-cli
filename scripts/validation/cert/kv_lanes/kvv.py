# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Isolated KVV adapter; never enters the r4 engine lifecycle."""
import kv_r5_runtime as runtime
import kv_r5_vllm_runner as family

AVAILABLE = True


def preflight(manifest):
    return runtime.adapter_preflight(manifest, family, 'KVV')


def freeze(args):
    return runtime.freeze_adapter(args, family, 'KVV')


def main(argv):
    return runtime.adapter_main(argv, family)
