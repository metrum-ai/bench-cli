-- Copyright (c) 2026 Metrum AI, Inc.
-- SPDX-License-Identifier: Apache-2.0
--
-- DuckDB: power sample counts and unweighted mean per measure stage.
-- Replace the NDJSON path. Prefer analyze.py for time-weighted means.

WITH stages AS (
  SELECT run_id, stage, load, t_start_ns, t_end_ns
  FROM read_ndjson_auto('/tmp/run.ndjson')
  WHERE kind = 'stage' AND phase = 'measure'
),
power AS (
  SELECT run_id, t_ns, value, metric, src
  FROM read_ndjson_auto('/tmp/run.ndjson')
  WHERE kind = 'telemetry'
    AND (
      metric LIKE '%power%'
      OR metric IN (
        'DCGM_FI_DEV_POWER_USAGE',
        'all_smi_gpu_power_consumption_watts',
        'nvidia_smi_power_draw_watts',
        'gpu_power_usage',
        'hw_power',
        'ipmi_dcmi_power_consumption_current_watts'
      )
    )
    AND unit IN ('W', '1')
)
SELECT
  s.stage,
  s.load,
  count(p.t_ns) AS power_samples,
  avg(p.value) AS power_mean_unweighted_w,
  (s.t_end_ns - s.t_start_ns) / 1e9 AS duration_s
FROM stages s
LEFT JOIN power p
  ON p.run_id = s.run_id
 AND p.t_ns >= s.t_start_ns
 AND p.t_ns < s.t_end_ns
GROUP BY s.stage, s.load, s.t_start_ns, s.t_end_ns
ORDER BY s.stage;
