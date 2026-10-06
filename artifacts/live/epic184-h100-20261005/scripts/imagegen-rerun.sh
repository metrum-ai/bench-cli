#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
# Metrum AI Bench CLI: epic #184 live validation, imagegen single-run rerun (2026-10-05 ~21:34Z, on the GPU host).
# live_modalities.sh imagegen passed `--guidance 0.0`, which clap rejected
# ("tip: a similar argument exists: '--guidance-scale'"). The server and the sweep were unaffected;
# only the single run was repeated, by hand, with the command below (as run, unchanged).
cd /opt/metrum-bench && O=live-results/epic184-imagegen && rm -rf $O/images && bin/metrum-ai-bench-cli-imagegen --url http://127.0.0.1:8000/v1/images/generations --api-key dummy --model Tongyi-MAI/Z-Image-Turbo --scenario epic184-imagegen --prompt "a lighthouse on a rocky coast at dusk, watercolor" --num-requests 8 --concurrency 1 --num-inference-steps 9 --guidance-scale 0.0 --artifact-dir $O/images --sut $O/sut.json --require-sut --telemetry $O/telemetry.yaml --ndjson $O/run.ndjson --data-log $O/run.jsonl > $O/run.stdout 2> $O/run.stderr; echo "imagegen exit $?"
