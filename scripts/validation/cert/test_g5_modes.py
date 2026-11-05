# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Synthetic fixtures only; no real GPU verification is implied."""
import base64
import json
import unittest

import g5_modes_checks


class ModesChecksTests(unittest.TestCase):
    def fixture(self, streaming, infer):
        events, rows = [], []
        for i in range(4):
            rid = i + 1
            identity = {'request': rid, 'kind': 'request_identity', 'thinking': False, 'max_tokens': 128, 'prompt_sha256': f'{i:064x}',
                        'model': 'm', 'temperature': 0, 'streaming': streaming}
            usage = {'prompt_tokens': 2, 'completion_tokens': 3}
            if streaming:
                raw = b'data: ' + json.dumps({'choices': [{'index': 0, 'delta': {'content': 'ok'}}]}).encode() + b'\n\n'
                raw += b'data: ' + json.dumps({'choices': [], 'usage': usage}).encode() + b'\n\ndata: [DONE]\n\n'
            else:
                raw = json.dumps({'choices': [{'message': {'content': 'ok'}}], 'usage': usage}).encode()
            events += [identity, {'request': rid, 'kind': 'headers', 'status': 200},
                       {'request': rid, 'kind': 'chunk', 'bytes_b64': base64.b64encode(raw).decode()},
                       {'request': rid, 'kind': 'complete'}]
            rows.append({'schema_version': 'bench.request.v5', 'phase': 'measure', 'seq': i, 'error': None,
                         'answer_completed': True, 'prompt_tokens': 2, 'completion_tokens': 3,
                         'usage_prompt_tokens_present': True, 'usage_completion_tokens_present': True,
                         'ttft_source': 'stream' if streaming else 'first_byte_approx' if infer else None,
                         'ttft_s': .1 if streaming else .2 if infer else None, 'first_byte_s': .2,
                         'first_reasoning_s': None, 'ttft_first_any_s': None, 'ttft_visible_answer_s': None})
        rows.append({'schema_version': 'bench.summary.v5', 'successes': 4})
        return '\n'.join(map(json.dumps, rows)), '\n'.join(map(json.dumps, events))

    def test_real_shape_checks_and_fabrication_rejection(self):
        for stream, infer in ((True, False), (False, False), (False, True)):
            data, trace = self.fixture(stream, infer)
            result = g5_modes_checks.evaluate(data, trace, stream, infer, 'm')
            self.assertEqual(4, result['answers'])
            with self.assertRaises(ValueError):
                g5_modes_checks.evaluate(data.replace('"completion_tokens": 3', '"completion_tokens": 4', 1), trace, stream, infer, 'm')
            if not stream:
                bad = data.replace('"first_reasoning_s": null', '"first_reasoning_s": 0.1', 1)
                with self.assertRaises(ValueError):
                    g5_modes_checks.evaluate(bad, trace, stream, infer, 'm')


if __name__ == '__main__':
    unittest.main()
