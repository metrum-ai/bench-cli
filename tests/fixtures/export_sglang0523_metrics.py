#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Serialize upstream metric statements with injected inputs, without a server.

Pass the unmodified metrics_collector.py from SGLang revision
297e54fd105e974cdf401e5b4d20253a22c16f90. Uses prometheus-client==0.25.0.
Only the selected constructor assignments, the actual cache reporting block
and _log_gauge method are executed. This is a source-slice exporter fixture,
not a complete logger instantiation, deployed exporter capture or GPU evidence.
"""
import argparse
import ast
import hashlib
import importlib.metadata
from pathlib import Path
from types import SimpleNamespace
from typing import Union

from prometheus_client import Counter, Gauge, generate_latest, REGISTRY


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    args = parser.parse_args()
    assert importlib.metadata.version('prometheus-client') == '0.25.0'
    raw = args.source.read_bytes()
    assert hashlib.sha256(raw).hexdigest() == '745d697dcf71ab8aa148059bccba27a286658bd736ee4b8a08489d90797f19f2'
    tree = ast.parse(raw)
    classes = {n.name: n for n in tree.body if isinstance(n, ast.ClassDef)}
    tokenizer = classes['TokenizerMetricsCollector']
    scheduler = classes['SchedulerMetricsCollector']
    def method(cls, name):
        return next(n for n in cls.body if isinstance(n, ast.FunctionDef) and n.name == name)
    def assignment(cls, attr):
        return next(n for n in method(cls, '__init__').body if isinstance(n, ast.Assign)
                    and any(isinstance(t, ast.Attribute) and t.attr == attr for t in n.targets))
    labels = {'model_name': 'fixture-model', 'tp_rank': '0'}
    obj = SimpleNamespace(labels=labels)
    namespace = {'Counter': Counter, 'Gauge': Gauge, 'Union': Union, 'self': obj, 'labels': labels,
                 'cached_tokens': 80, 'cached_tokens_details': {'device': 32, 'host': 48, 'storage': 0}}
    block = next(n for n in method(tokenizer, 'observe_one_finished_request').body
                 if isinstance(n, ast.If) and ast.unparse(n.test) == 'cached_tokens > 0')
    statements = [assignment(tokenizer, 'cached_tokens_total'), assignment(scheduler, 'cache_hit_rate'),
                  method(scheduler, '_log_gauge'), block]
    exec(compile(ast.Module(body=statements, type_ignores=[]), str(args.source), 'exec'), namespace)
    namespace['_log_gauge'](obj, obj.cache_hit_rate, 0.625)
    namespace['cached_tokens_details'] = None
    namespace['cached_tokens'] = 16
    exec(compile(ast.Module(body=[block], type_ignores=[]), str(args.source), 'exec'), namespace)
    page = generate_latest(REGISTRY).decode()
    # Runtime-dependent process and created timestamps are outside this slice.
    lines = [line for line in page.splitlines() if line.startswith(('# HELP sglang:', '# TYPE sglang:', 'sglang:'))
             and '_created' not in line]
    header = '# Copyright (c) 2026 Metrum AI, Inc.\n# SPDX-License-Identifier: Apache-2.0\n'
    Path(__file__).with_name('sglang0523-source-slice.prom').write_text(header + '\n'.join(lines) + '\n')


if __name__ == '__main__':
    main()
