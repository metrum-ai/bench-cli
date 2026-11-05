# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""CPU-only request-identity fixture tests; not GPU evidence."""
import base64
import hashlib
import json
import unittest

import trace_proxy


class ImageIdentityTests(unittest.TestCase):
    def test_hashes_image_without_recording_bytes_and_rejects_remote(self):
        raw = b'private-image-fixture'
        part = {'type': 'image_url', 'image_url': {'url': 'data:image/png;base64,' + base64.b64encode(raw).decode()}}
        body = {'model': 'm', 'messages': [{'role': 'user', 'content': [part]}], 'chat_template_kwargs': {'enable_thinking': True}}
        result = trace_proxy.image_identity(json.dumps(body).encode())
        self.assertEqual([hashlib.sha256(raw).hexdigest()], result['image_sha256'])
        self.assertNotIn(raw.decode(), json.dumps(result))
        part['image_url']['url'] = 'https://example.invalid/image.png'
        with self.assertRaises(ValueError): trace_proxy.image_identity(json.dumps(body).encode())


if __name__ == '__main__':
    unittest.main()
