-- Copyright (c) 2026 Metrum AI, Inc.
-- SPDX-License-Identifier: Apache-2.0
--
-- DuckDB sketch: energy counter first/last vs stage duration.
-- Full trapezoid and multi-GPU handling: docs/queries/analyze.py

WITH stages AS (
  SELECT run_id, stage, load, t_start_ns, t_end_ns
  FROM read_ndjson_auto('/tmp/run.ndjson')
  WHERE kind = 'stage' AND phase = 'measure'
),
energy AS (
  SELECT run_id, t_ns, value, metric, labels
  FROM read_ndjson_auto('/tmp/run.ndjson')
  WHERE kind = 'telemetry'
    AND metric IN (
      'DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION',
      'nvidia_smi_energy_joules_total',
      'gpu_energy_consumed',
      'hw_energy',
      'habanalabs_energy',
      'nv_energy_consumption'
    )
),
bounds AS (
  SELECT
    s.stage,
    s.load,
    e.metric,
    min(e.value) AS first_seen_j,
    max(e.value) AS last_seen_j,
    max(e.value) - min(e.value) AS energy_delta_j_approx,
    (s.t_end_ns - s.t_start_ns) / 1e9 AS duration_s
  FROM stages s
  JOIN energy e
    ON e.run_id = s.run_id
   AND e.t_ns >= s.t_start_ns
   AND e.t_ns < s.t_end_ns
  GROUP BY s.stage, s.load, e.metric, s.t_start_ns, s.t_end_ns
)
SELECT *
FROM bounds
ORDER BY stage, metric;
