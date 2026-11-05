# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Independent all-smi/device-CLI overlap checks for BC-TELEM-001."""
import csv
import datetime as dt
import io
import json
import math


def finite(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


def evaluate(ndjson, nvidia_csv, gpu_uuid, parser_exit):
    if parser_exit != 0 or not isinstance(gpu_uuid, str) or not gpu_uuid.startswith('GPU-'):
        raise ValueError('telemetry parser failed or device identity missing')
    rows = [json.loads(line) for line in ndjson.splitlines() if line.strip()]
    runs = [row for row in rows if row.get('kind') == 'run']
    summaries = [row for row in rows if row.get('kind') == 'summary']
    if (len(runs) != 1 or len(summaries) != 1 or summaries[0].get('partial') is not False
            or summaries[0].get('dropped_telemetry_rows') != 0 or summaries[0].get('scrape_error_rows') != 0):
        raise ValueError('complete telemetry summary with zero dropped rows and scrape errors required')
    run = runs[0]
    if not run.get('run_id') or any(row.get('run_id') != run['run_id'] for row in rows):
        raise ValueError('telemetry run identity mismatch')
    base = dt.datetime.fromisoformat(run['t0_wall'].replace('Z', '+00:00'))
    if base.tzinfo is None:
        raise ValueError('telemetry wall epoch is not timezone-aware')
    requests = [row for row in rows if row.get('kind') == 'request' and row.get('warmup') is False]
    if not requests or any(not isinstance(row.get('t_sent_ns'), int) or not isinstance(row.get('t_done_ns'), int)
                           or row['t_sent_ns'] > row['t_done_ns'] for row in requests):
        raise ValueError('measured request time bounds missing')
    first = min(row['t_sent_ns'] for row in requests)
    last = max(row['t_done_ns'] for row in requests)
    if first < 0 or last <= first:
        raise ValueError('invalid measurement window')
    series = {}
    for metric in ('all_smi_gpu_power_consumption_watts', 'all_smi_gpu_utilization'):
        selected = [row for row in rows if row.get('kind') == 'telemetry' and row.get('src') == 'all-smi'
                    and row.get('metric') == metric and row.get('labels', {}).get('gpu_uuid') == gpu_uuid
                    and isinstance(row.get('t_ns'), int) and first <= row['t_ns'] <= last]
        if (len({row['t_ns'] for row in selected}) < 2 or any(not finite(row.get('value')) or row['value'] < 0 for row in selected)
                or (metric.endswith('utilization') and any(row['value'] > 100 for row in selected))):
            raise ValueError('insufficient or invalid matching all-smi device samples')
        series[metric] = selected
    # nvidia-smi uses local wall time; the runner explicitly fixes TZ=UTC.
    start_wall = base.timestamp() + first / 1e9
    end_wall = base.timestamp() + last / 1e9
    valid = []
    for row in csv.DictReader(io.StringIO(nvidia_csv), skipinitialspace=True):
        if row.get('uuid') != gpu_uuid:
            continue
        try:
            when = dt.datetime.strptime(row['timestamp'], '%Y/%m/%d %H:%M:%S.%f').replace(tzinfo=dt.timezone.utc).timestamp()
            power = float((row.get('power.draw [W]') or row['power.draw']).split()[0])
        except (ValueError, KeyError, IndexError):
            raise ValueError('invalid NVIDIA device sample')
        if start_wall <= when <= end_wall:
            if not math.isfinite(power) or power < 0:
                raise ValueError('invalid NVIDIA power sample')
            valid.append((when, power))
    if len({when for when, _ in valid}) < 2:
        raise ValueError('independent device samples do not overlap measured requests')
    return {'measured_requests': len(requests), 'all_smi_power_samples': len(series['all_smi_gpu_power_consumption_watts']),
            'all_smi_utilization_samples': len(series['all_smi_gpu_utilization']), 'device_cli_power_samples': len(valid),
            'parser_exit': parser_exit}
