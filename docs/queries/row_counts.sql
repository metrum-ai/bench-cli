-- Copyright (c) 2026 Metrum AI, Inc.
-- SPDX-License-Identifier: Apache-2.0
--
-- DuckDB: counts by NDJSON kind.
--   duckdb -c ".read docs/queries/row_counts.sql"   # set path below first

SELECT kind, count(*) AS n
FROM read_ndjson_auto('/tmp/run.ndjson')
GROUP BY 1
ORDER BY 1;
