#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Source-bound vLLM KV-event observer; cache lookup residency, never byte erasure.

The engine is unmodified. Raw replay multipart frames, HTTP control replies,
metrics and tokenizer output are retained and rechecked at evaluation.
"""
import base64
import hashlib
import http.client
import json
import math
import struct
import time
from collections import Counter
from pathlib import Path

import kv_r5_metrics as metrics
import kv_r5_runtime as runtime

REVISION = '4ba787ec9959e566d3b587a796dff07673324da5'
SOURCE_FILES = {
    "config/kv_events.py": "8b0e6d9b2473c185e68fb42a168ab4a096f056c4cda9e6a5653dcf11ab9899b8",
    "distributed/kv_events.py": "82b362a0a7f89318d9127addfce254d537f5078a31aa87beb08493b0953895ee",
    "distributed/kv_transfer/kv_connector/v1/offloading/events.py": "0ff3466bfb379db411ab6caff3a9b2cb2bb1b7305848b9f58bab543424339df7",
    "distributed/kv_transfer/kv_connector/v1/offloading/scheduler.py": "51a860d1b8f91567385ae2d0ffaff211fb10930b4856ea9460c1828a927c573d",
    "entrypoints/launchers/api_server/routers.py": "3a5dcc6484074709348c99bb5a7a5599f04653ef3d4ddd40293949342d445e4c",
    "entrypoints/rl/factories.py": "4b6a386f1a288f694cd6987248c2b2006376bbc9c7ef1d899b281714134bb539",
    "entrypoints/rl/online/api_router.py": "ec50b66b3f419d02eb209bfac251fde2773f3308930a6a11ebfc583f77e0289a",  # gitleaks:allow -- public source-file SHA256
    "entrypoints/serve/tokenize/api_router.py": "44b4dad23f61ea88e3a5c7c6ec7440c4f36eeb5e98b6e785d733eb73c81e560b",  # gitleaks:allow -- public source-file SHA256
    "entrypoints/serve/tokenize/protocol.py": "ca0090a337d07660c78bfde2793793ae128b0de124c7251de99ec6428b383ee3",
    "v1/core/block_pool.py": "64d304d2b431a6ce9a0a0425610122354e1ac11b6aff1ca7d467d4218183774c",
    "v1/core/kv_cache_manager.py": "39ae1c1564a034fd59736a92d962bfab5a0af03529e01349b765422dbd5a355d",
    "v1/core/kv_cache_utils.py": "de64494f8316d3e706802d1461cbf17ddbd421bb4c18946c085145aefd9846ea",
    "v1/core/sched/scheduler.py": "d76cc22257550972137df9c893505d0bd0f5d482374691e6eca4372772de501a",
    "v1/engine/core.py": "d02d32df5d15f0a8b8ffc731ed4bc6f7fcb68b8a9273ec5e4c5d1ab4bd7d7e01",
    "v1/kv_cache_interface.py": "1e0b5027cc13ed845488ff3f671292e6c57fcae99c5b7130ad92c9d176a38684"
}
SUCCESS = 'vllm:request_success_total'
RUNNING = 'vllm:num_requests_running'
WAITING = 'vllm:num_requests_waiting'
CONFIG = 'vllm:cache_config_info'
SPECS = {SUCCESS: ('counter', 'requests'), RUNNING: ('gauge', 'requests'),
         WAITING: ('gauge', 'requests'), CONFIG: ('gauge', 'ratio')}
END = b'\xff'*8
TOPIC = b'bench-cli-kv-r5'
MAX_BYTES = 16*1024*1024

# Runs inside the owned image: host validators need no msgpack/ZMQ dependency.
# DEALER preserves the empty ROUTER delimiter and permits a streamed response.
REPLAY_CLIENT = r'''import base64,json,sys,time,zmq
port,limit,seconds=map(int,sys.argv[1:])
ctx=zmq.Context(); sock=ctx.socket(zmq.DEALER); sock.setsockopt(zmq.LINGER,0)
sock.setsockopt(zmq.RCVHWM,20002); sock.connect('tcp://127.0.0.1:'+str(port))
sock.send_multipart([b'',bytes(8)]); rows=[]; size=0; end=time.monotonic()+seconds
try:
 while True:
  left=end-time.monotonic()
  if left<=0 or not sock.poll(max(1,int(left*1000))): raise RuntimeError('replay deadline')
  row=sock.recv_multipart(); size+=sum(map(len,row))
  if size>limit or len(rows)>10000: raise RuntimeError('replay bounds exceeded')
  rows.append([base64.b64encode(x).decode() for x in row])
  if len(row)==4 and row[2]==b'\xff'*8: break
 print(json.dumps(rows))
finally:
 sock.close(); ctx.term()
'''


def unpack(raw):
    """Bounded MessagePack subset emitted by the pinned msgspec structs.

    Reject duplicate map keys, extension values, trailing data and deep trees.
    Bytes remain bytes, so an integer hash cannot alias a binary hash.
    """
    if len(raw) > MAX_BYTES:
        raise ValueError('oversized event batch')
    cursor = 0
    def take(n):
        nonlocal cursor
        if n < 0 or cursor+n > len(raw):
            raise ValueError('truncated MessagePack')
        result = raw[cursor:cursor+n]
        cursor += n
        return result
    def number(n, signed=False):
        return int.from_bytes(take(n), 'big', signed=signed)
    def read(depth=0):
        if depth > 32:
            raise ValueError('deep event batch')
        tag = number(1)
        if tag < 128:
            return tag
        if tag >= 224:
            return tag-256
        if tag == 192:
            return None
        if tag in (194, 195):
            return tag == 195
        if tag in (202, 203):
            result = struct.unpack('>f' if tag == 202 else '>d', take(4 if tag == 202 else 8))[0]
            if not math.isfinite(result):
                raise ValueError('non-finite MessagePack')
            return result
        if 204 <= tag <= 211:
            return number(1 << ((tag-204) % 4), tag >= 208)
        if 196 <= tag <= 198:
            return take(number(1 << (tag-196)))
        if 160 <= tag <= 191 or 217 <= tag <= 219:
            length = tag-160 if tag < 192 else number(1 << (tag-217))
            return take(length).decode('utf-8')
        array = 144 <= tag <= 159 or tag in (220, 221)
        mapping = 128 <= tag <= 143 or tag in (222, 223)
        if not array and not mapping:
            raise ValueError('unsupported MessagePack type')
        length = tag % 16 if tag < 160 else number(2 if tag in (220, 222) else 4)
        if length > len(raw):
            raise ValueError('impossible event collection length')
        if array:
            return [read(depth+1) for _ in range(length)]
        result = {}
        for _ in range(length):
            key = read(depth+1)
            if not isinstance(key, str) or key in result:
                raise ValueError('invalid/duplicate event map key')
            result[key] = read(depth+1)
        return result
    result = read()
    if cursor != len(raw):
        raise ValueError('trailing MessagePack')
    return result


def decode_replay(rows):
    if not isinstance(rows, list) or not rows or len(rows) > 10001:
        raise ValueError('missing/bounded replay required')
    batches, size = [], 0
    for index, encoded in enumerate(rows):
        if not isinstance(encoded, list) or len(encoded) != 4:
            raise ValueError('invalid replay multipart shape')
        row = [base64.b64decode(x, validate=True) for x in encoded]
        size += sum(map(len, row))
        if size > MAX_BYTES or row[0] != b'':
            raise ValueError('invalid replay envelope')
        if row[2] == END:
            if index != len(rows)-1 or row[1] != b'' or row[3] != b'':
                raise ValueError('invalid replay terminator')
            return batches
        if row[1] != TOPIC or len(row[2]) != 8 or int.from_bytes(row[2], 'big') != index:
            raise ValueError('event sequence gap, restart or wrong topic')
        batch = unpack(row[3])
        if (not isinstance(batch, list) or len(batch) not in (2, 3)
                or type(batch[0]) not in (float, int) or not math.isfinite(batch[0])
                or not isinstance(batch[1], list) or (len(batch) == 3 and batch[2] not in (0, None))):
            raise ValueError('unsupported batch/rank')
        batches.append(batch[1])
    raise ValueError('replay terminator missing')


def token_hash(tokens):
    return hashlib.sha256(json.dumps(tokens, separators=(',', ':')).encode()).hexdigest()


def opaque(value):
    if type(value) is int and 0 <= value < 2**64:
        return ('int', value)
    if isinstance(value, bytes) and 1 <= len(value) <= 64:
        return ('bytes', value)
    raise ValueError('invalid opaque block hash')


def plan(data):
    result = runtime.load(runtime.pin(data['inputs']['eviction_plan']))
    if result.get('schema_version') != 'vllm033-eviction-plan.v1':
        raise ValueError('source-bound observer plan required')
    tokens = result['prefix_token_ids']
    if not tokens or len(tokens) > 131072 or any(type(t) is not int or not 0 <= t < 2**32 for t in tokens):
        raise ValueError('bounded frozen prefix tokens required')
    if token_hash(tokens) != data['target_prefix_sha256']:
        raise ValueError('target prefix token identity mismatch')
    groups = result['groups']
    if (not groups or len(groups) > 16 or [g['group_idx'] for g in groups] != list(range(len(groups)))
            or not any(g['kind'] == 'full_attention' for g in groups)):
        raise ValueError('complete ordered group inventory with full attention required')
    for g in groups:
        if (type(g['group_idx']) is not int or g['kind'] not in ('full_attention', 'mamba') or type(g['block_size']) is not int
                or g['block_size'] <= 0 or len(tokens) % g['block_size']):
            raise ValueError('unsupported/un-aligned target group geometry')
    # The inventory must be independently captured and reviewed at Stage-0;
    # this runner does not derive physical groups/bytes from a logical counter.
    inventory = runtime.load(runtime.pin(data['inputs']['kv_group_inventory']))
    if (inventory.get('source_revision') != REVISION or inventory.get('groups') != groups
            or inventory.get('image') != data['image'] or inventory.get('model_revision') != data['model_revision']
            or not inventory.get('raw_evidence')):
        raise ValueError('reviewed engine group inventory missing')
    for item in inventory['raw_evidence']:
        runtime.pin(item)
        if item not in data['inputs'].values():
            raise ValueError('raw group inventory evidence must be a transitive frozen input')
    request = result['target_request']
    import kv_r5_proxy
    identity = kv_r5_proxy.identity(json.dumps(request).encode())
    target = next(p for p in data['phases'] if p['name'] == 'rehit')['expected_requests']
    if len(target) != 1 or any(identity[key] != target[0][key] for key in ('messages_sha256', 'options')):
        raise ValueError('tokenizer request differs from the rehit wire identity')
    allowed = {'model', 'stream', 'stream_options', 'max_tokens', 'max_completion_tokens',
               'temperature', 'top_p', 'top_k', 'seed', 'ignore_eos', 'chat_template_kwargs',
               'kv_cache_report_mode', 'skip_special_tokens'}
    for phase in data['phases']:
        for row in phase['expected_requests']:
            options = row['options']
            if set(options)-allowed or options.get('kv_cache_report_mode', 'incremental') != 'incremental':
                raise ValueError('unsupported hashing/reporting/sampling request option')
    if not all(isinstance(m, dict) and set(m) <= {'role', 'content'} and isinstance(m.get('content'), str)
               for m in request['messages']):
        raise ValueError('observer target must be plain text chat')
    if not result['cache_config'] or not {'num_gpu_blocks', 'block_size'} <= result['cache_config'].keys():
        raise ValueError('reported cache geometry must be frozen')
    if any(type(result['cache_config'][k]) is not int or result['cache_config'][k] <= 0
           for k in ('num_gpu_blocks', 'block_size')):
        raise ValueError('reported cache geometry must have known positive counts')
    return result


def validate(data):
    if data.get('eviction_observer') != 'vllm033-kv-events.v1':
        raise ValueError('reviewed KV-events observer required')
    module = Path(__file__).resolve()
    if not any(runtime.pin(item) == module for item in data['inputs'].values()):
        raise ValueError('observer module must be a frozen input')
    if any(p.get('reset_prefix_cache') for p in data['phases']):
        raise ValueError('fresh owned engine required; no cache reset in observed epoch')
    forbidden = {'--kv-events-config', '--kv-transfer-config', '--data-parallel-size', '-dp',
                 '--tensor-parallel-size', '-tp', '--pipeline-parallel-size', '-pp',
                 '--enable-lora', '--speculative-config', '--config', '--chat-template',
                 '--enable-prompt-embeds', '--enable-mm-embeds', '--prefix-caching-hash-algo'}
    if any(a.split('=')[0] in forbidden for a in data['server_flags']):
        raise ValueError('observer topology/hash/configuration override')
    boolean = {'--enable-prefix-caching', '--enable-chunked-prefill', '--no-enable-chunked-prefill', '--enforce-eager'}
    valued = {'--max-num-seqs', '--max-model-len', '--gpu-memory-utilization', '--kv-cache-dtype',
              '--kv-offloading-size', '--kv-offloading-backend', '--block-size',
              '--max-num-batched-tokens', '--mamba-cache-mode'}
    flags = iter(data['server_flags'])
    for flag in flags:
        name, equal, value = flag.partition('=')
        if name in boolean and not equal:
            continue
        if name not in valued or (not equal and next(flags, None) is None) or (equal and not value):
            raise ValueError('unreviewed observer server option')
    if any(r.get('warmup') for p in data['phases'] for r in p['expected_requests']):
        raise ValueError('observed phases require zero held-out warmup requests')
    plan(data)


def server_options():
    # Both sockets stay inside the container. Replay is read with docker exec;
    # there is no new host port to expose or leave behind at cleanup.
    config = {'enable_kv_cache_events': True, 'publisher': 'zmq', 'endpoint': 'tcp://*:5557',
              'replay_endpoint': 'tcp://*:5558', 'topic': TOPIC.decode(),
              'buffer_steps': 10000, 'hwm': 100000, 'max_queue_size': 100000}
    return ['--kv-events-config', json.dumps(config), '--tensor-parallel-size', '1',
            '--pipeline-parallel-size', '1', '--data-parallel-size', '1']


def state(rows, frozen):
    """Replay a multiset: duplicate physical blocks may share a lookup hash."""
    batches = decode_replay(rows)
    groups = {g['group_idx']: g for g in frozen['groups']}
    counts, prefixes, stored = Counter(), {}, set()
    target = tuple(frozen['prefix_token_ids'])
    for batch in batches:
        for event in batch:
            if not isinstance(event, dict) or event.get('type') not in ('BlockStored', 'BlockRemoved', 'AllBlocksCleared'):
                raise ValueError('unknown cache event')
            if event['type'] == 'AllBlocksCleared':
                raise ValueError('cache clear in observed epoch')
            if event.get('medium') in ('CPU', 'STORAGE'):
                continue  # These never prove GPU eviction.
            if event.get('medium') != 'GPU' or event.get('locality') not in (None, 'LOCAL') or event.get('ownership') is not None:
                raise ValueError('unknown event residency')
            group = event.get('group_idx')
            if type(group) is not int or group not in groups:
                raise ValueError('event outside frozen group inventory')
            hashes = event['block_hashes']
            if not isinstance(hashes, list) or (not hashes and event['type'] != 'BlockStored'):
                raise ValueError('invalid GPU hash event')
            keys = [(group, opaque(h)) for h in hashes]
            if event['type'] == 'BlockRemoved':
                for key in keys:
                    if counts[key] <= 0:
                        raise ValueError('removal without prior store')
                    counts[key] -= 1
                continue
            spec = groups[group]
            if (event.get('kv_cache_spec_kind') != spec['kind'] or type(event['block_size']) is not int
                    or not 0 < event['block_size'] <= spec['block_size']
                    or event.get('kv_cache_spec_sliding_window') is not None
                    or event.get('lora_id') is not None or event.get('lora_name') is not None
                    or event.get('lookahead_token_ids') is not None
                    or any(x not in (None, []) for x in (event.get('extra_keys') or []))):
                raise ValueError('unsupported cache hash/spec semantics')
            tokens = event['token_ids']
            if not isinstance(tokens, list) or len(tokens) > 262144 or any(type(t) is not int or not 0 <= t < 2**32 for t in tokens):
                raise ValueError('invalid event tokens')
            if not keys:
                continue  # A masked Mamba range can legitimately emit no hashes.
            if spec['kind'] == 'full_attention':
                parent = event['parent_block_hash']
                prefix = (0, True, hashlib.sha256()) if parent is None else prefixes.get(opaque(parent))
                width = event['block_size']
                if prefix is None or len(tokens) != len(keys)*width:
                    raise ValueError('incomplete full-attention token chain')
                for offset, (_, h) in enumerate(keys):
                    length, matches, digest = prefix
                    chunk = tuple(tokens[offset*width:(offset+1)*width])
                    hasher = digest.copy()
                    for token in chunk:
                        hasher.update(token.to_bytes(4, 'big'))
                    prefix = (length+len(chunk), matches and chunk == target[length:length+len(chunk)], hasher)
                    previous = prefixes.get(h)
                    if previous is not None and (previous[0], previous[2].digest()) != (prefix[0], hasher.digest()):
                        raise ValueError('conflicting opaque hash binding')
                    prefixes[h] = prefix
            stored.update(keys)
            if len(stored) > 100000:
                raise ValueError('too many cache entries')
            counts.update(keys)
    required, seen_groups = set(), set()
    # Mamba hashes use the same Request.block_hashes boundaries; bind them
    # through the full-attention chain, not the masked Mamba token_ids array.
    for group, h in stored:
        bound = prefixes.get(h)
        if bound is None:
            raise ValueError('opaque group hash has no full-attention token binding')
        spec = groups[group]
        length, matches, _ = bound
        if matches and length <= len(target):
            required.add((group, h))
            seen_groups.add(group)
    if seen_groups != groups.keys():
        raise ValueError('target not observed in every frozen cache group')
    for group, spec in groups.items():
        lengths = {prefixes[h][0] for g, h in required if g == group}
        expected = set(range(spec['block_size'], len(target)+1, spec['block_size'])) if spec['kind'] == 'full_attention' else {len(target)}
        if not expected <= lengths:
            raise ValueError('incomplete target-prefix GPU entries')
    return {'resident_target_entries': sum(counts[k] for k in required),
            'target_keys': required, 'counts': counts, 'batch_count': len(batches)}


def counter_snapshot(page, frozen, model):
    values = metrics.snapshot(page, SPECS)
    counters, configs = {}, []
    for (name, labels), value in values.items():
        labels = dict(labels)
        if name == CONFIG:
            if value != 1 or labels.get('engine') != '0' or any(labels.get(k) != str(v) for k, v in frozen['cache_config'].items()):
                raise ValueError('reported engine cache geometry differs from plan')
            configs.append(labels)
            continue
        base = {k: v for k, v in labels.items() if k != 'finished_reason'}
        if base != {'model_name': model, 'engine': '0'}:
            raise ValueError('counter engine/model identity differs')
        if name == SUCCESS:
            if set(labels) != {'model_name', 'engine', 'finished_reason'}:
                raise ValueError('completion counter identity missing')
            counters[labels['finished_reason']] = value
        elif value != 0:
            raise ValueError('engine still has running/waiting requests')
    if len(configs) != 1 or set(counters) != {'stop', 'length', 'abort', 'error', 'repetition'}:
        raise ValueError('one engine cache configuration and completions required')
    return counters


def completion_delta(before, after):
    if before.keys() != after.keys() or any(after[k] < before[k] for k in before):
        raise ValueError('completion counter reset/identity change')
    # Disjoint FinishReason bins for exactly one model/engine; never replicas.
    return sum(after[k]-before[k] for k in before)


def control(instance, directory, method, path, body=None):
    conn = http.client.HTTPConnection('127.0.0.1', instance.data['port'],
                                      timeout=max(.1, min(30, instance.deadline-time.monotonic())))
    try:
        conn.request(method, path, None if body is None else json.dumps(body), {'Content-Type': 'application/json'})
        response = conn.getresponse()
        raw = response.read(4*1024*1024+1)
        row = {'method': method, 'path': path, 'request': body, 'status': response.status,
               'body_b64': base64.b64encode(raw).decode()}
        with (directory/'control.jsonl').open('a') as stream:
            stream.write(json.dumps(row)+'\n')
        if response.status != 200 or len(raw) > 4*1024*1024:
            raise ValueError('observer control endpoint failed')
        return json.loads(raw)
    finally:
        conn.close()


def capture(instance, directory, expected=None, baseline=None):
    directory.mkdir()
    if control(instance, directory, 'POST', '/pause?mode=wait&clear_cache=false') != {'status': 'paused'}:
        raise ValueError('drain fence not acknowledged')
    if control(instance, directory, 'GET', '/is_paused') != {'is_paused': True}:
        raise ValueError('engine is not paused')
    frozen = plan(instance.data)
    for attempt in range(100):
        page = instance.get(instance.data['port'], '/metrics')
        (directory/f'metrics-{attempt:03}.prom').write_text(page)
        counts = counter_snapshot(page, frozen, instance.data['served_model'])
        difference = None if baseline is None else completion_delta(baseline, counts)
        if expected is None or difference == expected:
            break
        if difference > expected or time.monotonic() >= instance.deadline:
            raise ValueError('competing request or unsettled completion count')
        time.sleep(.1)
    else:
        raise ValueError('completion metrics did not settle')
    previous = None
    for attempt in range(100):
        result = instance.docker('exec', instance.name, 'python3', '-c', REPLAY_CLIENT,
                                 '5558', str(MAX_BYTES), '10', timeout=15)
        rows = json.loads(result.stdout)
        runtime.save(directory/f'replay-{attempt:03}.json', rows)
        decode_replay(rows)
        if previous == rows:
            break
        previous = rows
        if time.monotonic() >= instance.deadline:
            raise TimeoutError('replay tail did not settle')
    else:
        raise ValueError('replay tail did not stabilize')
    runtime.save(directory/'snapshot.json', {'metrics_attempt': int(sorted(directory.glob('metrics-*.prom'))[-1].stem.split('-')[1]),
                                           'replay_attempt': attempt})
    return rows, counts


def before_phase(instance, phase, directory):
    frozen = plan(instance.data)
    baseline = getattr(instance, '_kv_observer_counts', None)
    rows, counts = capture(instance, directory/'observer-before', 0 if baseline is not None else None, baseline)
    if baseline is None:
        if decode_replay(rows) or any(counts.values()):
            raise ValueError('fresh engine already has requests/cache events')
        request = frozen['target_request']
        body = {'model': instance.data['served_model'], 'messages': request['messages'],
                'add_special_tokens': False, 'add_generation_prompt': True, 'continue_final_message': False,
                'chat_template_kwargs': request.get('chat_template_kwargs')}
        tokens = control(instance, directory/'observer-before', 'POST', '/tokenize', body)['tokens']
        if tokens[:len(frozen['prefix_token_ids'])] != frozen['prefix_token_ids'] or len(tokens) <= len(frozen['prefix_token_ids']):
            raise ValueError('frozen prefix differs from actual server tokenization')
    instance._kv_observer_before = counts
    if control(instance, directory/'observer-before', 'POST', '/resume') != {'status': 'resumed'}:
        raise ValueError('resume failed')


def after_phase(instance, phase, directory):
    _, counts = capture(instance, directory/'observer-after', len(phase['expected_requests']), instance._kv_observer_before)
    instance._kv_observer_counts = counts


def retained(directory, frozen, model, resumed=False):
    controls = [json.loads(line) for line in (directory/'control.jsonl').read_text().splitlines()]
    expected = [('POST', '/pause?mode=wait&clear_cache=false', {'status': 'paused'}),
                ('GET', '/is_paused', {'is_paused': True})]
    for row, (method, path, reply) in zip(controls, expected):
        if (row['method'], row['path'], row['status'], json.loads(base64.b64decode(row['body_b64'], validate=True))) != (method, path, 200, reply):
            raise ValueError('raw drain-fence evidence differs')
    if len(controls) < 2 or (resumed and (controls[-1]['path'] != '/resume' or controls[-1]['status'] != 200
            or json.loads(base64.b64decode(controls[-1]['body_b64'])) != {'status': 'resumed'})):
        raise ValueError('raw pause/resume evidence missing')
    paths = [c['path'] for c in controls]
    valid = [['/pause?mode=wait&clear_cache=false', '/is_paused']]
    if resumed:
        valid = [valid[0]+['/resume'], valid[0]+['/tokenize', '/resume']]
    if paths not in valid:
        raise ValueError('unexpected control activity in observed bracket')
    meta = runtime.load(directory/'snapshot.json')
    index = meta['replay_attempt']
    if type(index) is not int or not 1 <= index < 100:
        raise ValueError('stable replay pair absent')
    rows = runtime.load(directory/f'replay-{index:03}.json')
    if rows != runtime.load(directory/f'replay-{index-1:03}.json'):
        raise ValueError('replay tail not stable')
    decode_replay(rows)
    counts = counter_snapshot((directory/f'metrics-{meta["metrics_attempt"]:03}.prom').read_text(), frozen, model)
    return rows, counts, controls


def verify(data, phases):
    frozen = plan(data)
    states, last_rows, last_counts = {}, None, None
    epoch = runtime.load(phases[0]['directory'].parent/'worker-epoch.json')
    if not epoch:
        raise ValueError('worker epoch evidence missing')
    for phase in phases:
        directory, name = phase['directory'], phase['plan']['name']
        if runtime.load(directory/'worker-epoch.json') != epoch:
            raise ValueError('worker process continuity changed')
        before, a, controls = retained(directory/'observer-before', frozen, data['served_model'], True)
        after, b, _ = retained(directory/'observer-after', frozen, data['served_model'])
        if completion_delta(a, b) != len(phase['plan']['expected_requests']):
            raise ValueError('competing/missing admitted requests')
        if last_rows is None:
            if decode_replay(before) or any(a.values()):
                raise ValueError('observer did not start at a fresh engine epoch')
            tokenize = [c for c in controls if c['path'] == '/tokenize']
            request = frozen['target_request']
            expected_body = {'model': data['served_model'], 'messages': request['messages'],
                             'add_special_tokens': False, 'add_generation_prompt': True,
                             'continue_final_message': False,
                             'chat_template_kwargs': request.get('chat_template_kwargs')}
            if len(tokenize) != 1 or tokenize[0]['status'] != 200 or tokenize[0]['request'] != expected_body:
                raise ValueError('raw tokenizer binding missing')
            tokens = json.loads(base64.b64decode(tokenize[0]['body_b64']))['tokens']
            prefix = frozen['prefix_token_ids']
            if tokens[:len(prefix)] != prefix or len(tokens) <= len(prefix):
                raise ValueError('tokenizer prefix contradicts frozen target')
        elif before != last_rows or completion_delta(last_counts, a) != 0:
            raise ValueError('events/requests outside the captured phase brackets')
        # Replay from 0 must retain all previous bytes exactly.
        if after[:len(before)-1] != before[:-1]:
            raise ValueError('event transcript changed across brackets')
        states[name] = (None if name == 'cold' else state(before, frozen), state(after, frozen))
        last_rows, last_counts = after, b
    pressure_before, pressure_after = states['pressure']
    rehit_before, _ = states['rehit']
    if (pressure_before['resident_target_entries'] <= 0 or pressure_after['resident_target_entries'] != 0
            or rehit_before['resident_target_entries'] != 0
            or pressure_before['target_keys'] != pressure_after['target_keys']
            or pressure_before['target_keys'] != rehit_before['target_keys']):
        raise ValueError('target GPU cache lookup entries were not fully evicted by pressure')
    rehit = next(p for p in phases if p['plan']['name'] == 'rehit')
    def raw_page(side):
        directory = rehit['directory']/('observer-'+side)
        meta = runtime.load(directory/'snapshot.json')
        return (directory/f'metrics-{meta["metrics_attempt"]:03}.prom').read_text()
    query, hit = 'vllm:prefix_cache_queries_total', 'vllm:prefix_cache_hits_total'
    delta = metrics.deltas(raw_page('before'), raw_page('after'), {query: ('counter', 'tokens'), hit: ('counter', 'tokens')})
    if metrics.single(delta, query) <= 0 or metrics.single(delta, hit) != 0:
        raise ValueError('independent rehit bracket is not a GPU prefix-cache miss')
    # Existing runtime.wire/product_rows proves the sole rehit is the frozen A.
    # The pinned native scheduler's pause wait drains transfer jobs, and no
    # other admitted request is present in this exactly-one-request window.
    return {'schema_version': 'kv-exclusive-window.v2', 'source_revision': REVISION,
            'prefix_sha256': data['target_prefix_sha256'], 'source': 'vllm033-kv-events',
            'same_epoch': True, 'target_gpu_evicted': True, 'exclusive_requests': True, 'exclusive_transfers': True,
            'async_work_settled': True, 'pressure_geometry_verified': True,
            'rehit_miss_accounting_verified': True,
            'gpu_lookup_entries_before': pressure_before['resident_target_entries'],
            'gpu_lookup_entries_after': pressure_after['resident_target_entries'],
            'unit': 'GPU cache lookup entries; no byte-erasure/physical-occupancy claim',
            'raw_evidence': ['observer-before/', 'observer-after/']}
