#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Serialize injected events with pinned upstream classes and publisher methods.

Requires msgspec==0.19.0. No engine import, model, CUDA or network listener.
Usage: python generate_kv_event_fixture.py PINNED_KV_EVENTS_PY OUTPUT_JSON
"""
import ast
import base64
import hashlib
import itertools
import json
import logging
import queue
import sys
import time
from collections import deque
from pathlib import Path
from types import SimpleNamespace
from typing import Any

import msgspec

SOURCE = '82b362a0a7f89318d9127addfce254d537f5078a31aa87beb08493b0953895ee'


def generate(source):
    if msgspec.__version__ != '0.19.0':
        raise ValueError('fixture generation requires pinned msgspec 0.19.0')
    if hashlib.sha256(source).hexdigest() != SOURCE:
        raise ValueError('upstream kv_events.py does not match reviewed source')
    tree = ast.parse(source)
    names = {'EventBatch', 'KVCacheEvent', 'BlockStored', 'BlockRemoved', 'AllBlocksCleared', 'KVEventBatch'}
    classes = [n for n in tree.body if isinstance(n, ast.ClassDef) and n.name in names]
    ns = {'msgspec': msgspec, 'Any': Any, 'ExternalBlockHash': bytes | int,
          'queue': queue, 'time': time, 'logger': logging.getLogger(__name__)}
    exec(compile(ast.Module(body=classes, type_ignores=[]), 'pinned-upstream-structs', 'exec'), ns)
    publisher = next(n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == 'ZmqEventPublisher')
    methods = [n for n in publisher.body if isinstance(n, ast.FunctionDef)
               and n.name in ('publish', '_publisher_thread', '_service_replay')]
    exec(compile(ast.Module(body=methods, type_ignores=[]), 'pinned-upstream-publisher', 'exec'), ns)
    a, b = b'a'*32, b'b'*32
    def stores():
        return [ns['BlockStored'](block_hashes=[a, b], parent_block_hash=None, token_ids=[1, 2, 3, 4],
                block_size=2, lora_id=None, medium='GPU', lora_name=None, group_idx=0,
                kv_cache_spec_kind='full_attention'),
                ns['BlockStored'](block_hashes=[b], parent_block_hash=None, token_ids=[1, 2, 3, 4],
                block_size=4, lora_id=None, medium='GPU', lora_name=None, group_idx=1, kv_cache_spec_kind='mamba')]
    def removals():
        return [ns['BlockRemoved'](block_hashes=[a, b], medium='GPU', group_idx=0),
                ns['BlockRemoved'](block_hashes=[b], medium='GPU', group_idx=1)]
    sent = []
    state = SimpleNamespace(_running=True, _data_parallel_rank=0, _event_queue=queue.Queue(),
                            _replay=None, _pub=SimpleNamespace(send_multipart=sent.append),
                            _seq_gen=itertools.count(), _topic_bytes=b'bench-cli-kv-r5', _buffer=deque())
    for i, events in enumerate([stores(), stores(), removals(), removals()]):
        ns['publish'](state, ns['KVEventBatch'](ts=1000.+i, events=events))
    state._running = False
    ns['_publisher_thread'](state)
    replay = []
    state.END_SEQ = b'\xff'*8
    state._replay = SimpleNamespace(recv_multipart=lambda: [b'fixture-client', b'', bytes(8)],
                                    send_multipart=replay.append)
    ns['_service_replay'](state)
    assert [tuple(row[2:]) for row in replay[:-1]] == sent
    return {'schema_version': 'vllm033-kv-events-serialization-fixture.v1',
            'source_revision': '4ba787ec9959e566d3b587a796dff07673324da5', 'source_sha256': SOURCE,
            'serializer': 'msgspec '+msgspec.__version__,
            'scope': 'injected synthetic events; actual upstream structs, publish, publisher thread and replay serialization; no GPU run',
            'replay': [[base64.b64encode(x).decode() for x in row[1:]] for row in replay]}


if __name__ == '__main__':
    Path(sys.argv[2]).write_text(json.dumps(generate(Path(sys.argv[1]).read_bytes()), indent=2)+'\n')
