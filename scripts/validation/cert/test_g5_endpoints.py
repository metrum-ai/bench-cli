# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""CPU-only synthetic oracle tests, not GPU evidence."""
import base64
import json
import unittest

import g5_endpoints_checks as checks
import g5_endpoints_runner as runner


class EndpointChecksTests(unittest.TestCase):
    def fixture(self):
        rows = []
        for seq, endpoint, start, end, error in ((0, 'survivor', 0, 1, None), (1, 'victim', 0, 1, None),
                                                   (2, 'victim', 3, 4, {'kind': 'connect'}), (3, 'survivor', 3, 4, None)):
            rows.append({'schema_version': 'bench.request.v5', 'phase': 'measure', 'seq': seq, 'endpoint': endpoint,
                         'started_at': f'2026-01-01T00:00:0{start}Z', 'completed_at': f'2026-01-01T00:00:0{end}Z',
                         'answer_completed': error is None, 'error': error})
        rows.append({'schema_version': 'bench.summary.v5', 'attempted': 4, 'successes': 3, 'errors': 1})
        stream = b'data: {"choices":[],"usage":{"prompt_tokens":2,"completion_tokens":3}}\n\ndata: [DONE]\n\n'
        frame = base64.b64encode(stream).decode()
        traces = {}
        for name in ('survivor', 'victim'):
            events = [{'request': 1, 'kind': 'send', 'monotonic_ns': 1},
                      {'request': 1, 'kind': 'chunk', 'monotonic_ns': 2, 'bytes_b64': frame},
                      {'request': 1, 'kind': 'complete', 'monotonic_ns': 3}]
            if name == 'survivor':
                events.append({'request': 2, 'kind': 'send', 'monotonic_ns': 8})
            traces[name] = '\n'.join(map(json.dumps, events))
        kill = {'kill_exit': 0, 'kill_start_ns': 4, 'kill_end_ns': 5,
                'kill_start_utc': '2026-01-01T00:00:02Z', 'kill_end_utc': '2026-01-01T00:00:02.5Z'}
        return rows, traces, kill

    def test_frozen_ports_and_memory_fraction(self):
        config = {'port': 8001, 'proxy_port': 8002, 'second_port': 8003, 'second_proxy_port': 8004,
                  'server_flags': ['--gpu-memory-utilization', '0.45']}
        self.assertEqual([8001, 8002, 8003, 8004], runner.configuration(config))
        with self.assertRaises(ValueError):
            runner.configuration({**config, 'second_port': 8001})
        with self.assertRaises(ValueError):
            runner.configuration({**config, 'server_flags': ['--gpu-memory-utilization', '0.9']})
        with self.assertRaises(ValueError):
            runner.configuration({**config, 'server_flags': ['--gpu-memory-utilization', '0.45', '--gpu-memory-utilization=0.45']})

    def test_real_ordering_and_counts_are_required(self):
        rows, traces, kill = self.fixture()
        encode = lambda: '\n'.join(map(json.dumps, rows))
        self.assertEqual(1, checks.verify(encode(), traces, kill, 1)['survivor_post_kill_answers'])
        with self.assertRaises(ValueError):
            checks.verify(encode(), traces, kill, 0)
        rows[3]['started_at'] = '2026-01-01T00:00:01Z'
        with self.assertRaises(ValueError):
            checks.verify(encode(), traces, kill, 1)
        rows[3]['started_at'] = '2026-01-01T00:00:03Z'
        rows[-1]['errors'] = 0
        with self.assertRaises(ValueError):
            checks.verify(encode(), traces, kill, 1)


if __name__ == '__main__':
    unittest.main()
