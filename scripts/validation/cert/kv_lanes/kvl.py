# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""LMCache frozen execution and launch adapter."""
import kv_c_adapter
import kv_lmcache_runner

AVAILABLE = True


def preflight(manifest):
    return kv_c_adapter.preflight(manifest, kv_lmcache_runner, 'KVL')


def freeze(args):
    return kv_c_adapter.freeze(args, kv_lmcache_runner, 'KVL')


def main(argv):
    return kv_c_adapter.main(argv, kv_lmcache_runner, 'KVL')
