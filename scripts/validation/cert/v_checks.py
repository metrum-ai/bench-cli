# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Independent real-image reasoning smoke assertions for BC-REASON-005."""
import json
import math

import g1_clock_checks
import g4_modality_checks


def evaluate(data, trace, code, image_sha, model):
    rows = [json.loads(line) for line in data.splitlines() if line.strip()]
    requests = [row for row in rows if '.request.' in row.get('schema_version', '') and row.get('phase') == 'measure']
    if len(requests) != 7:
        raise ValueError('exactly seven measured VLM requests required')
    def valid_image_count(row):
        value = row.get('modality_metrics', {}).get('image_count')
        return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value) and value > 0
    if any(row.get('error') is not None or row.get('answer_completed') is not True
           or not valid_image_count(row) for row in requests):
        raise ValueError('every measured VLM request must complete with an image and no error')
    images = g4_modality_checks.verify_images(trace, image_sha, model)
    if images != 7:
        raise ValueError('real server saw a different number of pinned-image requests')
    clocks = g1_clock_checks.evaluate(data, trace, code)
    if clocks['requests'] != 7 or clocks['first_any_compared'] != 7 or clocks['reasoning_streams'] < 1:
        raise ValueError('real reasoning and first-any clock coverage incomplete')
    return {'measured': 7, 'completed': 7, 'real_pinned_image_requests': images, 'clocks': clocks}
