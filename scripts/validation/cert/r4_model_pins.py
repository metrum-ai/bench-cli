# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Prospective v1.9.0 catalog r4 serving identity constraints."""

GENERAL_MODEL = 'Qwen/Qwen3.8-27B-FP8'
GENERAL_REVISION = 'c3a462cc3d38cf351f3b26cbd763469eeb74ef81'
SMALL_MODEL = 'Qwen/Qwen3.5-0.8B'
SMALL_REVISION = '2fc06364715b967f1860aea9cf38778875588b17'
IMAGE = 'vllm/vllm-openai@sha256:d825baf9ce4b1b6bfc5ecc7b79880c7e6748c65a72d697ce3d47607aafa84c6e'
GENERAL_LANES = {'L1', 'L2', 'R', 'D', 'V', 'W', 'G1', 'G2', 'G3', 'G4', 'G6'}
G5_CASES = {'BC-GPU-013', 'BC-GPU-014', 'BC-GPU-015'}


def check(config, lane, case=None):
    """Bind exact r4 identity; mixed-model G5 must specify a known case."""
    if lane == 'G5':
        if case not in G5_CASES:
            raise ValueError('G5 r4 identity requires an explicit known case')
        small = case == 'BC-GPU-015'
    elif lane == 'G7':
        small = True
    elif lane in GENERAL_LANES:
        small = False
    else:
        return
    expected = (SMALL_MODEL, SMALL_REVISION, IMAGE) if small else (GENERAL_MODEL, GENERAL_REVISION, IMAGE)
    if (config.get('model'), config.get('revision'), config.get('image')) != expected:
        raise ValueError(f'{lane} r4 model, revision or release image mismatch')
