-- Copyright (c) 2026 Metrum AI, Inc.
-- SPDX-License-Identifier: Apache-2.0
-- Compare DCGM energy counter delta vs trapezoid of power (manual follow-up).
SELECT json_extract_string(json, '$.metric') AS metric, count(*) AS n
FROM read_ndjson_auto('run.ndjson')
WHERE json_extract_string(json, '$.kind') = 'telemetry'
  AND json_extract_string(json, '$.metric') IN (
    'DCGM_FI_DEV_POWER_USAGE',
    'DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION',
    'all_smi_gpu_power_consumption_watts'
  )
GROUP BY 1;
