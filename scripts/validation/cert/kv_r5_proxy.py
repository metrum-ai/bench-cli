#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Private r5 wire capture using the existing loopback response recorder."""
import hashlib
import json
import argparse
import sys

import trace_proxy


def identity(body):
    value = json.loads(body)
    messages = value.get('messages')
    if not isinstance(messages, list) or not messages:
        raise ValueError('chat messages required')
    # Hash the entire ordered conversation, including recorded assistant turns.
    canonical = json.dumps(messages, sort_keys=True, separators=(',', ':'), ensure_ascii=False)
    text_only = all(isinstance(m, dict) and isinstance(m.get('content'), str)
                    and set(m) <= {'role', 'content', 'name'} for m in messages)
    return {'messages_sha256': hashlib.sha256(canonical.encode()).hexdigest(),
            'text_only': text_only,
            'options': {key: val for key, val in value.items() if key != 'messages'}}


if __name__ == '__main__':
    trace_proxy.request_identity = identity
    parser = argparse.ArgumentParser(add_help=False, allow_abbrev=False)
    parser.add_argument('--session-first-measured', type=int)
    parser.add_argument('--session-completed-before', type=int)
    parser.add_argument('--session-model')
    args, remaining = parser.parse_known_args()
    if args.session_first_measured is not None:
        import kv_r5_session_cold as cold
        if args.session_first_measured < 1 or args.session_completed_before is None or args.session_completed_before < 0 or not args.session_model:
            parser.error('complete session cold-boundary settings required')
        port = int(remaining[remaining.index('--upstream-port')+1])
        trace_proxy.Recorder = lambda path: cold.Recorder(path, port, args.session_model,
            args.session_first_measured, args.session_completed_before)
        original_handler = trace_proxy.make_handler
        trace_proxy.make_handler = lambda upstream, recorder, *a, **kw: cold.handler(
            original_handler(upstream, recorder, *a, **kw), recorder)
        sys.argv = [sys.argv[0], *remaining]
    raise SystemExit(trace_proxy.main())
