#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Private r5 wire capture using the existing loopback response recorder."""
import hashlib
import json

import trace_proxy


def identity(body):
    value = json.loads(body)
    messages = value.get('messages')
    if not isinstance(messages, list) or not messages:
        raise ValueError('chat messages required')
    # Hash the entire ordered conversation, including recorded assistant turns.
    canonical = json.dumps(messages, sort_keys=True, separators=(',', ':'), ensure_ascii=False)
    return {'messages_sha256': hashlib.sha256(canonical.encode()).hexdigest(),
            'options': {key: val for key, val in value.items() if key != 'messages'}}


if __name__ == '__main__':
    trace_proxy.request_identity = identity
    raise SystemExit(trace_proxy.main())
