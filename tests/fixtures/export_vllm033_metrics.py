#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Generate real vLLM logger exposition with synthetic inputs; no engine is run.

Requires vLLM 0.33.0 and prometheus-client 0.26.0 in a separate CPU Python
installation. Output is a parser fixture, not workload or GPU evidence.
"""
import hashlib
import importlib.metadata
from pathlib import Path
from types import SimpleNamespace

import prometheus_client
from vllm.config import ObservabilityConfig
from vllm.v1.metrics import loggers
from vllm.v1.metrics.stats import PrefixCacheStats, SchedulerStats

assert importlib.metadata.version('vllm') == '0.33.0'
assert importlib.metadata.version('prometheus-client') == '0.26.0'
assert hashlib.sha256(Path(loggers.__file__).read_bytes()).hexdigest() == (
    'a17291f044251f1586457b6bd1542f98660724dd07a426d61ce852186b95b9e1'
)
config = SimpleNamespace(
    observability_config=ObservabilityConfig(kv_cache_metrics=True),
    model_config=SimpleNamespace(served_model_name='fixture-model', max_model_len=2048, is_diffusion=False),
    speculative_config=None, kv_transfer_config=None, ec_transfer_config=None,
    lora_config=None, watermark_config=None,
)
logger = loggers.PrometheusStatLogger(config)
# These values are deliberately injected, not measured from a vLLM engine.
stats = SchedulerStats(
    kv_cache_usage=0.375,
    prefix_cache_stats=PrefixCacheStats(queries=100, hits=40),
    connector_prefix_cache_stats=PrefixCacheStats(queries=60, hits=24),
)
logger.record(stats, None)
out = Path(__file__).with_name('vllm033-logger-synthetic-config.prom')
out.write_bytes(prometheus_client.generate_latest())
print(f'wrote {out.name}; no engine or GPU run')
