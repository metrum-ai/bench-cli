#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Common single-server create confirmation, not a timeout proof."""
import re


def container_id(root):
    import stage0_milestones
    path = stage0_milestones.local_file(root, 'evidence/container-id.txt')
    with path.open('rb') as stream:
        data = stream.read(66)
    if not re.fullmatch(rb'[0-9a-f]{64}\n', data):
        raise ValueError('invalid Docker create response')
    return data[:-1].decode('ascii')


def successful_create(root, *, gpu_name, gpu_uuid):
    """Require the successful CLI ID and matching owner/image/GPU inspect receipt.

    Only meaningful for a supported exact single-server dispatch after an
    ordinary successful launcher and an empty cgroup. Never closes ambiguous creates.
    """
    import r4_image_provenance as r4
    import stage0_milestones
    stage0_milestones.check_r4_identity(root, gpu_name=gpu_name, gpu_uuid=gpu_uuid,
                                        index=r4.INDEX, child=r4.AMD64, config=r4.CONFIG)
    return True
