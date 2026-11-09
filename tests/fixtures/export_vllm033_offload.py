#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Generate vLLM 0.33 logger text with a real config and synthetic stats.

The local model directory must contain the config and tokenizer from the pinned
Qwen3.5-0.8B revision. Requires vLLM 0.33.0, prometheus-client 0.26.0 and a
compatible CPU PyTorch environment. No model weights, engine or GPU is run.
"""
import argparse
import hashlib
import importlib.metadata
import os
from pathlib import Path

os.environ['HF_HUB_OFFLINE'] = '1'
os.environ['TRANSFORMERS_OFFLINE'] = '1'

from prometheus_client import generate_latest
from vllm.config import DeviceConfig, KVTransferConfig, ModelConfig, ObservabilityConfig, VllmConfig
from vllm.distributed.kv_transfer.kv_connector.v1.offloading import metrics as offload_metrics
from vllm.v1.metrics import loggers
from vllm.v1.metrics.stats import PrefixCacheStats, SchedulerStats


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--model-dir', type=Path, required=True)
args = parser.parse_args()
assert importlib.metadata.version('vllm') == '0.33.0'
assert importlib.metadata.version('prometheus-client') == '0.26.0'
assert sha(loggers.__file__) == 'a17291f044251f1586457b6bd1542f98660724dd07a426d61ce852186b95b9e1'
assert sha(offload_metrics.__file__) == 'd01f41fcbbf9476188432e32449c9dd2d2a2c4ba6d32f06b42ba3246fe6e1c57'
assert sha(args.model_dir / 'config.json') == 'b90b86f35c8e6925ef74ee04d0e758f0a845c83a42089ad82bbaa948de9b4204'
assert sha(args.model_dir / 'tokenizer.json') == '5f9e4d4901a92b997e463c1f46055088b6cca5ca61a6522d1b9f64c4bb81cb42'
model = ModelConfig(model=str(args.model_dir), tokenizer=str(args.model_dir),
                    served_model_name='fixture-model', max_model_len=2048)
config = VllmConfig(
    model_config=model,
    device_config=DeviceConfig(device='cpu'),
    observability_config=ObservabilityConfig(kv_cache_metrics=True),
    kv_transfer_config=KVTransferConfig(kv_connector='OffloadingConnector', kv_role='kv_both',
                                        kv_connector_extra_config={'cpu_bytes_to_use': 1073741824}),
)
logger = loggers.PrometheusStatLogger(config)
offload = offload_metrics.OffloadingConnectorStats()
# Injected numbers test serialization only, not real transfer activity.
for name, amount in (
    ('vllm:kv_offload_load_bytes', 4096),
    ('vllm:kv_offload_store_bytes', 8192),
    ('vllm:kv_offload_load_time', 0.01),
    ('vllm:kv_offload_store_time', 0.02),
    ('vllm:kv_offload_allocation_failure', 1),
):
    offload.increase_counter(name, amount)
offload.observe_histogram('vllm:kv_offload_lookup_sync_delay_seconds', 0.0005)
stats = SchedulerStats(
    kv_cache_usage=0.375,
    prefix_cache_stats=PrefixCacheStats(queries=100, hits=40),
    connector_prefix_cache_stats=PrefixCacheStats(queries=60, hits=24),
    kv_connector_stats=offload.to_dict(),
)
logger.record(stats, None)
out = Path(__file__).with_name('vllm033-offload-synthetic-stats.prom')
out.write_bytes(generate_latest())
print(f'wrote {out.name}; no engine or GPU run')
