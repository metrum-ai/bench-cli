# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Independent first-measured-request counter boundary for the serial 008 stage."""
import http.client
import json
import threading
import time
from pathlib import Path

import kv_r5_metrics as metrics
import trace_proxy

QUERY = 'vllm:prefix_cache_queries_total'
HIT = 'vllm:prefix_cache_hits_total'
SUCCESS = 'vllm:request_success_total'
RUNNING = 'vllm:num_requests_running'
WAITING = 'vllm:num_requests_waiting'
APC = {QUERY: ('counter', 'tokens'), HIT: ('counter', 'tokens')}
SPECS = {**APC, SUCCESS: ('counter', 'requests'), RUNNING: ('gauge', 'requests'), WAITING: ('gauge', 'requests')}


def completions(page, model):
    values = metrics.snapshot(page, SPECS)
    reasons = {}
    for (name, labels), value in values.items():
        labels = dict(labels)
        identity = {k: v for k, v in labels.items() if k != 'finished_reason'}
        if identity != {'model_name': model, 'engine': '0'}:
            raise ValueError('cold boundary engine/model identity changed')
        if name == SUCCESS:
            if set(labels) != {'model_name', 'engine', 'finished_reason'}:
                raise ValueError('cold boundary completion identity missing')
            reasons[labels['finished_reason']] = value
        elif name in (RUNNING, WAITING) and value != 0:
            return None
    if set(reasons) != {'stop', 'length', 'abort', 'error', 'repetition'}:
        raise ValueError('cold boundary completion bins missing')
    # These are disjoint source FinishReason bins, never replica counters.
    return sum(reasons.values())


def compare(before, after, model, completed_before):
    if completions(before, model) != completed_before or completions(after, model) != completed_before+1:
        raise ValueError('first measured request window is not exclusive and settled')
    # Also reject a reset or changed finish-reason series within the bracket.
    metrics.deltas(before, after, SPECS)
    rows = metrics.deltas(before, after, APC)
    if metrics.single(rows, QUERY) <= 0 or metrics.single(rows, HIT) != 0:
        raise ValueError('first measured request is not prefix-cold')
    return rows


def reset_ack(path):
    value = json.loads(Path(path).read_text())
    reply = json.loads(value['body'])
    if ((value.get('method'), value.get('path'), value['status']) != ('POST', '/reset_prefix_cache', 200)
            or not isinstance(reply, dict) or reply.get('success') is not True):
        raise ValueError('session stage reset acknowledgement unsuccessful')
    return value


class Recorder(trace_proxy.Recorder):
    def __init__(self, path, port, model, first, completed_before):
        super().__init__(path)
        self.directory, self.port, self.model = Path(path).parent, port, model
        self.first, self.completed_before = first, completed_before
        self.boundary_lock = threading.Lock()
        self.before = None

    def snapshot(self, side, expected):
        for attempt in range(50):
            conn = http.client.HTTPConnection('127.0.0.1', self.port, timeout=3)
            try:
                conn.request('GET', '/metrics')
                response = conn.getresponse()
                raw = response.read(4*1024*1024+1)
                if response.status != 200 or len(raw) > 4*1024*1024:
                    raise ValueError('cold boundary exporter unavailable')
                page = raw.decode()
            finally:
                conn.close()
            name = f'first-measured-{side}-{attempt:03}.prom'
            (self.directory/name).write_text(page)
            count = completions(page, self.model)
            if count == expected:
                return name, page
            if count is not None and count > expected:
                raise ValueError('unexpected admitted request at cold boundary')
            time.sleep(.1)
        raise ValueError('cold boundary counters did not settle')

    def event(self, request_id, kind, data=None, **extra):
        if request_id == self.first and kind == 'send':
            self.before = self.snapshot('before', self.completed_before)
        if request_id == self.first and kind == 'complete':
            after = self.snapshot('after', self.completed_before+1)
            compare(self.before[1], after[1], self.model, self.completed_before)
            receipt = {'schema_version': 'kv-session-cold.v1', 'request': self.first,
                       'completed_before': self.completed_before, 'before': self.before[0], 'after': after[0]}
            with (self.directory/'first-measured-cold.json').open('x') as stream:
                json.dump(receipt, stream)
        super().event(request_id, kind, data, **extra)


def handler(base, recorder):
    class SerialStage(base):
        def do_POST(self):
            # The frozen stage uses sweep=1. Hold the next POST at this one
            # request boundary until its final counter observation completes;
            # response chunks/stream TTFT are forwarded by the existing handler.
            with recorder.boundary_lock:
                return super().do_POST()
    return SerialStage


def verify(phase, model, completed_before):
    directory, requests = phase['directory'], phase['plan']['expected_requests']
    first = next(i for i, r in enumerate(requests) if not r['warmup'])+1
    receipt = json.loads((directory/'first-measured-cold.json').read_text())
    if (type(receipt.get('request')) is not int or type(receipt.get('completed_before')) is not int
            or (receipt.get('schema_version'), receipt.get('request'), receipt.get('completed_before')) != (
            'kv-session-cold.v1', first, completed_before+first-1)):
        raise ValueError('first measured counter boundary differs from frozen stage')
    pages = []
    for side in ('before', 'after'):
        name = receipt[side]
        path = directory/name
        if path.parent.resolve() != directory.resolve() or not name.startswith('first-measured-'+side+'-') or not name.endswith('.prom'):
            raise ValueError('cold boundary raw page path invalid')
        pages.append(path.read_text())
    values = [metrics.snapshot(text, APC) for text in (phase['before'], *pages, phase['after'])]
    if any(v.keys() != values[0].keys() for v in values) or any(
            not values[0][k] <= values[1][k] <= values[2][k] <= values[3][k] for k in values[0]):
        raise ValueError('first measured counter boundary is outside its stage')
    return compare(*pages, model, receipt['completed_before'])
