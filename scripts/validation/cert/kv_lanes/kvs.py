# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""SGLang frozen execution and launch adapter."""
import kv_c_adapter
import kv_sglang_runner

AVAILABLE = True


def preflight(manifest):
    return kv_c_adapter.preflight(manifest, kv_sglang_runner, 'KVS')


def freeze(args):
    return kv_c_adapter.freeze(args, kv_sglang_runner, 'KVS')


def main(argv):
    return kv_c_adapter.main(argv, kv_sglang_runner, 'KVS')
