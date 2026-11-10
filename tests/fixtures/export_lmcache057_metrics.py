#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Run full MP metric subscriber classes on synthetic events, without a server.

Pass the LMCache 0.5.7 source root (236f3eec). Uses OTel SDK 1.40.0,
Prometheus exporter 0.61b0, prometheus-client 0.25.0. Exact source classes and
helpers are compiled unchanged; only package imports and EventBus dispatch are
replaced by direct calls with synthetic events. No model/GPU is executed.
"""
import argparse
import ast
from abc import ABC, abstractmethod
from collections import Counter
import hashlib
import importlib.metadata
from pathlib import Path
from types import SimpleNamespace

from opentelemetry import metrics
from opentelemetry.exporter.prometheus import PrometheusMetricReader
from opentelemetry.sdk.metrics import MeterProvider
from prometheus_client import generate_latest

HASHES = {
    'lookup': '7c06623bf9588bb6f4f915a72bb94749c54c4b15ce7b8b34951de6b31086b173',
    'l1': 'f02c8c90e72b1b7f66b17b849fa59e8c0aa086f3a2124094415f80a5eb436ca4',
    'engine': '189253a08ed3cc97dcc3f6f01757daf8d5d79d443eca0fe61da91bfef3db4078',
    'mp_transfer': 'd40fd2775bfad3b6480b82d3da2b58ddca32ae4a8c51c76440e62d49b3bcecf8',
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source_root', type=Path)
    args = parser.parse_args()
    for name, version in [('opentelemetry-sdk', '1.40.0'), ('opentelemetry-exporter-prometheus', '0.61b0'), ('prometheus-client', '0.25.0')]:
        assert importlib.metadata.version(name) == version
    root = args.source_root / 'lmcache/v1/mp_observability'
    future = ast.ImportFrom(module='__future__', names=[ast.alias(name='annotations')], level=0)
    namespace = {'metrics': metrics, 'ABC': ABC, 'abstractmethod': abstractmethod, 'Counter': Counter}
    def execute(path, names, digest=None):
        raw = path.read_bytes()
        if digest is not None:
            assert hashlib.sha256(raw).hexdigest() == digest
        tree = ast.parse(raw)
        nodes = [n for n in tree.body if isinstance(n, (ast.FunctionDef, ast.ClassDef)) and n.name in names]
        assert len(nodes) == len(names)
        module = ast.fix_missing_locations(ast.Module(body=[future, *nodes], type_ignores=[]))
        exec(compile(module, str(path), 'exec'), namespace)
    execute(root / 'event_bus.py', ['EventSubscriber'])
    execute(root / 'subscribers/metrics/utils.py', ['group_by_salt', 'emit_salt_counts'])
    reader = PrometheusMetricReader(disable_target_info=True)
    provider = MeterProvider(metric_readers=[reader])
    metrics.set_meter_provider(provider)
    for module, names in [('lookup', ['_lookup_attrs', 'LookupMetricsSubscriber']), ('l1', ['L1MetricsSubscriber']),
                          ('engine', ['EngineMetricsSubscriber']), ('mp_transfer', ['MPTransferCountersSubscriber'])]:
        execute(root / f'subscribers/metrics/{module}.py', names, HASHES[module])
    lookup = namespace['LookupMetricsSubscriber']()
    lookup._on_lookup_prefetch_end(SimpleNamespace(metadata={'model_name': 'fixture-model', 'cache_salt': 'fixture',
        'requested_tokens': 1568, 'hit_tokens': 784, 'l1_hit_tokens': 784, 'l2_hit_tokens': 0,
        'prompt_tokens': 1568, 'request_hit_tokens': 784}))
    l1 = namespace['L1MetricsSubscriber']()
    event = SimpleNamespace(metadata={'keys': [SimpleNamespace(cache_salt='fixture')] * 2, 'l1_tag': 'default'})
    l1._on_write_finished(event)
    l1._on_read_finished(event)
    namespace['EngineMetricsSubscriber']()._on_retrieve_end(SimpleNamespace(metadata={'retrieved_count': 2,
        'engine_id': 'fixture-worker', 'model_name': 'fixture-model', 'cache_salt': 'fixture'}))
    transfers = namespace['MPTransferCountersSubscriber']()
    for method in ['_on_store_submitted', '_on_store_finished', '_on_retrieve_submitted', '_on_retrieve_finished']:
        getattr(transfers, method)(SimpleNamespace(metadata={'device': 'cuda:0'}))
    lines = [line for line in generate_latest().decode().splitlines()
             if line.startswith(('# HELP lmcache_mp_', '# TYPE lmcache_mp_', 'lmcache_mp_'))]
    header = '# Copyright (c) 2026 Metrum AI, Inc.\n# SPDX-License-Identifier: Apache-2.0\n'
    Path(__file__).with_name('lmcache057-mp-source-slice.prom').write_text(header + '\n'.join(lines) + '\n')
    provider.shutdown()


if __name__ == '__main__':
    main()
