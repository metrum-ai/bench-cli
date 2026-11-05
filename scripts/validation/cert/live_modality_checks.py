# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Independent request-cohort assertions for the three local modality lanes."""
import json
import math


def number(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


def evaluate(data, modality, smoke_exit, headline_exit, artifacts):
    if modality not in ('vlm', 'asr', 'imagegen') or smoke_exit or headline_exit:
        raise ValueError('live smoke or independent headline assertion failed')
    rows = [json.loads(line) for line in data.splitlines() if line.strip()]
    requests = [row for row in rows if '.request.' in row.get('schema_version', '') and row.get('phase') == 'measure']
    summaries = [row for row in rows if '.summary.' in row.get('schema_version', '')]
    if len(summaries) != 1 or not requests or any(row.get('error') is not None for row in requests):
        raise ValueError('missing or failed measured requests or non-unique summary')
    summary = summaries[0]
    if (summary.get('attempted') != len(requests) or summary.get('successes') != len(requests)
            or summary.get('config', {}).get('common', {}).get('require_sut') is not True or not summary.get('sut')):
        raise ValueError('summary cohort or required SUT mismatch')
    values = [row.get('modality_metrics', {}) for row in requests]
    if modality == 'vlm' and any(not number(row.get('image_count')) or row['image_count'] <= 0 for row in values):
        raise ValueError('every VLM success must include an image')
    if modality == 'asr' and any(not number(row.get('wer')) or not number(row.get('cer'))
                                  or row['wer'] < 0 or row['cer'] < 0 for row in values):
        raise ValueError('every ASR success must include finite WER and CER')
    if modality == 'imagegen':
        returned = [row.get('images_returned') for row in values]
        if any(not number(value) or value <= 0 or not float(value).is_integer() for value in returned):
            raise ValueError('every imagegen success must return images')
        if artifacts != sum(returned):
            raise ValueError('saved-image count differs from returned images')
    return {'attempted': len(requests), 'successes': len(requests), 'saved_images': artifacts,
            'smoke_exit': smoke_exit, 'headline_exit': headline_exit}
