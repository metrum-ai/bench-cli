<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Telemetry exporters

Install one-liners and default scrape targets for YAML under
[examples/](examples/). Verify ports and metric names against the project
README before a publishable run; vendors change tags.

Bind listeners to `127.0.0.1` when the exporter and the bench client share a
host. Paths are Prometheus text unless noted.

## all-smi (required default)

**Use the Metrum fork only.** Upstream lablup defaults to `/metrics`; the fork
used by this repo exposes `/metric`.

```bash
# Prefer the published Linux binary (see GitHub Releases for other arches):
curl -fsSL -o /tmp/all-smi.tgz \
  https://github.com/chetan-metrum-ai/all-smi/releases/download/v0.26.3-metrum.3/all-smi-linux-x86_64.tar.gz
tar -xzf /tmp/all-smi.tgz -C /tmp && sudo install -m 0755 /tmp/all-smi /usr/local/bin/all-smi
all-smi api --port 9090
# scrape: http://127.0.0.1:9090/metric
```

Example: [examples/all-smi.yaml](examples/all-smi.yaml).

## NVIDIA dcgm-exporter

```bash
docker run -d --rm --name dcgm-exporter --gpus all --cap-add SYS_ADMIN \
  -p 127.0.0.1:9400:9400 \
  nvcr.io/nvidia/k8s/dcgm-exporter:4.1.1-4.0.4-ubuntu22.04
# scrape: http://127.0.0.1:9400/metrics
```

Pin the image tag to a current `dcgm-exporter` release. Example:
[examples/dcgm.yaml](examples/dcgm.yaml).

## utkuozdemir/nvidia_gpu_exporter

```bash
docker run -d --gpus all \
  -e NVIDIA_DRIVER_CAPABILITIES=utility \
  -p 127.0.0.1:9835:9835 \
  utkuozdemir/nvidia_gpu_exporter:latest
# scrape: http://127.0.0.1:9835/metrics
```

Example: [examples/nvidia_gpu_exporter.yaml](examples/nvidia_gpu_exporter.yaml).

## ROCm device-metrics-exporter

```bash
docker run -d --privileged --device=/dev/dri --device=/dev/kfd \
  -v /sys:/sys:ro -p 127.0.0.1:5000:5000 --name device-metrics-exporter \
  rocm/device-metrics-exporter:v1.5.3
# scrape: http://127.0.0.1:5000/metrics
```

Example: [examples/rocm.yaml](examples/rocm.yaml).

## Intel XPU Manager (xpumd Prometheus)

xpumd v2 exports OpenTelemetry Prometheus on port 8080 (`hw_*` names). Bind
explicitly to loopback in the xpumd config when possible.

```bash
docker run -d --user 0 --cap-drop ALL --cap-add PERFMON --device /dev/dri \
  --publish 127.0.0.1:8080:8080 \
  ghcr.io/intel/xpumanager/xpumd:latest \
  --config /etc/xpumd/config-example.yaml
# scrape: http://127.0.0.1:8080/metrics
```

Example: [examples/xpum.yaml](examples/xpum.yaml).

## Intel Gaudi Prometheus Metric Exporter

```bash
# Docs pin vault.habana.ai/gaudi-metric-exporter/metric-exporter:<version>
docker run -d --privileged --net=host \
  vault.habana.ai/gaudi-metric-exporter/metric-exporter:1.24.1
# scrape: http://127.0.0.1:41611/metrics
```

Example: [examples/gaudi.yaml](examples/gaudi.yaml). Power is milliwatts;
scale to watts in YAML `units`.

## node_exporter

```bash
docker run -d --net=host --pid=host \
  -v /:/host:ro,rslave \
  quay.io/prometheus/node-exporter:latest \
  --path.rootfs=/host
# scrape: http://127.0.0.1:9100/metrics
```

Example: [examples/node_exporter.yaml](examples/node_exporter.yaml).

## Intel pcm-sensor-server

```bash
docker run -d --name pcm --privileged -p 127.0.0.1:9738:9738 ghcr.io/intel/pcm
# or: sudo ./pcm-sensor-server
# scrape: http://127.0.0.1:9738/metrics
```

Example: [examples/pcm.yaml](examples/pcm.yaml).

## prometheus-community/ipmi_exporter

```bash
docker run -d -p 127.0.0.1:9290:9290 prometheuscommunity/ipmi-exporter
# scrape: http://127.0.0.1:9290/metrics
```

Example: [examples/ipmi.yaml](examples/ipmi.yaml).

## jenningsloy318/redfish_exporter

```bash
# build binary from https://github.com/jenningsloy318/redfish_exporter
redfish_exporter --config.file=redfish_exporter.yml
# scrape: http://127.0.0.1:9610/redfish?target=<bmc-host>
```

Example: [examples/redfish.yaml](examples/redfish.yaml). Path is `/redfish`,
not `/metrics`.

## cAdvisor

```bash
VERSION=v0.49.1  # pin a current release; docs placeholders go stale
docker run -d --volume=/:/rootfs:ro --volume=/var/run:/var/run:ro \
  --volume=/sys:/sys:ro --volume=/var/lib/docker/:/var/lib/docker:ro \
  --publish=127.0.0.1:8080:8080 --name=cadvisor \
  ghcr.io/google/cadvisor:$VERSION
# scrape: http://127.0.0.1:8080/metrics
```

Example: [examples/cadvisor.yaml](examples/cadvisor.yaml).

## Serving engines

| Engine | Enable | Default scrape | Example |
|--------|--------|----------------|---------|
| vLLM | on by default with OpenAI server | `http://127.0.0.1:8000/metrics` | [examples/vllm.yaml](examples/vllm.yaml) |
| SGLang | `--enable-metrics` | `http://127.0.0.1:30000/metrics` | [examples/sglang.yaml](examples/sglang.yaml) |
| TensorRT-LLM | OpenAI-style server metrics | `http://127.0.0.1:8000/prometheus/metrics` | [examples/tensorrt_llm.yaml](examples/tensorrt_llm.yaml) |
| Triton | default on (`--allow-metrics=false` to disable) | `http://127.0.0.1:8002/metrics` | [examples/triton.yaml](examples/triton.yaml) |
| llama.cpp | `--metrics` or `LLAMA_ARG_ENDPOINT_METRICS=1` | `http://127.0.0.1:8080/metrics` | [examples/llamacpp.yaml](examples/llamacpp.yaml) |

Engine YAML files cover KV/queue/preemption series. Pair them with all-smi or
DCGM for board power and temperature.
