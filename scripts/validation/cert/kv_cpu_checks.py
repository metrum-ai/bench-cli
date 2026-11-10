# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Independent CPU oracles. These do not import the product's implementation."""
import csv
import hashlib
import io
import json
from html.parser import HTMLParser
from pathlib import Path

FIXTURES = Path(__file__).resolve().parent / 'fixtures/kv_cpu'


def require(condition, reason):
    if not condition:
        raise ValueError(reason)


def compact(value):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':'), allow_nan=False).encode()


def sha(data):
    return hashlib.sha256(data).hexdigest()


def read_json(path):
    from manifest_tool import unique_object
    return json.loads(Path(path).read_text(), object_pairs_hook=unique_object,
                      parse_constant=lambda x: (_ for _ in ()).throw(ValueError(x)))


def verify_files(root, expected):
    for name, digest in expected.items():
        require(sha((Path(root) / name).read_bytes()) == digest, f'input checksum mismatch: {name}')


def fixture_lock():
    lock = read_json(FIXTURES / 'lock.json')
    verify_files(FIXTURES, lock['files'])
    verify_files(Path(__file__).resolve().parents[3], lock['schemas'])
    return lock


def check_documents(job, source, output, report):
    """Reconstruct ordered target identities, payload bytes and manifest hashes."""
    raw = Path(source).read_bytes()
    file = job['origin']['files'][0]
    require(file['sha256'] == sha(raw) and file['bytes'] == len(raw), 'source bytes mismatch')
    origin, mapping = job['origin'], job['mapping']
    rows = [json.loads(line) for line in raw.splitlines()]
    expected = []
    for row in rows:
        questions = row[mapping['questions_field']]
        if not mapping['question_array']:
            questions = [questions]
        document = row[mapping['document_field']]
        group = sha(compact([origin['source_id'], row[mapping['document_id_field']], sha(document.encode())]))
        for i, question in enumerate(questions):
            row_id = row[mapping['row_id_field']]
            ident = sha(compact([origin['source_id'], origin['resolved_revision'], file['sha256'], row_id, i]))
            expected.append((ident, row_id, group, document + '\n\nQuestion:\n' + question))
    if job['count'] is not None:
        expected = expected[:job['count']]
    actual = [read_json_line(line) for line in Path(output).read_text().splitlines()]
    require(len(actual) == len(expected), 'canonical target count mismatch')
    manifest = report['manifest']
    require(manifest['origin'] == origin and manifest['rights'] == job['rights'], 'manifest source/rights mismatch')
    require(manifest['transformations'][:2] == ['document-first-question-last.v1', 'selection:first-fit.v1'],
            'processing chain missing')
    for row, (ident, row_id, group, body) in zip(actual, expected):
        require(row['record_id'] == ident and row['source_row_id'] == row_id and row['group_id'] == group,
                'canonical identity mismatch')
        require(row['body'] == {'Chat': [{'role': 'user', 'content': body}]}, 'document/question bytes mismatch')
        require(row['expected_output_tokens'] == mapping['output_budget'], 'output budget mismatch')
        require(all(row[k] is None for k in ('session_id', 'turn_index', 'trace_id', 'arrival_offset_ms')),
                'fabricated runtime boundary')
        envelope = row['source']
        require(envelope['source_id'] == origin['source_id'] and
                envelope['resolved_revision'] == origin['resolved_revision'] and
                envelope['files'] == origin['files'], 'canonical lineage mismatch')
        # Fingerprint uses declaration order, which JSON parsing preserves.
        require(envelope['validation_manifest']['sha256'] == sha(compact(manifest)), 'manifest digest mismatch')
    ids = [entry[0] for entry in expected]
    require(manifest['selected_ids_sha256'] == sha(compact(ids)), 'selected IDs digest mismatch')
    require(manifest['body_order_sha256'] == sha(compact([sha(e[3].encode()) for e in expected])),
            'body order digest mismatch')
    require(len(set(ids)) == len(ids), 'duplicate target IDs')
    return {'source_rows': len(rows), 'targets': len(expected), 'canonical_sha256': sha(Path(output).read_bytes())}


def read_json_line(line):
    from manifest_tool import unique_object
    return json.loads(line, object_pairs_hook=unique_object)


def distribution(values):
    values = sorted(values)
    n = len(values)
    return {'n': n, 'min': values[0] if n else None, 'max': values[-1] if n else None,
            'p50': values[(n * 50 + 99) // 100 - 1] if n else None,
            'p95': values[(n * 95 + 99) // 100 - 1] if n else None}


def check_overlap(job, source, report):
    """Encode the frozen WhitespaceSplit/WordLevel fixture with a separate trie."""
    tok = read_json(job['rendering']['tokenizer_path'])
    vocab = tok['model']['vocab']
    require(tok['pre_tokenizer']['type'] == 'WhitespaceSplit', 'oracle tokenizer contract')
    bodies, tokens, groups, rendered_records = [], [], [], []
    for line in Path(source).read_text().splitlines():
        row = json.loads(line)
        body = row['context'] + '\n\nQuestion:\n' + row['question']
        bodies.append(body)
        tokens.append([vocab.get(word, vocab['[UNK]']) for word in body.split()])
        groups.append(row['doc'])
        origin = job['origin']
        rendered_records.append({
            'record_id': sha(compact([origin['source_id'], origin['resolved_revision'],
                                     origin['files'][0]['sha256'], row['id'], 0])),
            'group_id': sha(compact([origin['source_id'], row['doc'], sha(row['context'].encode())])),
            'tokens': tokens[-1], 'rendered_body_sha256': sha(body.encode()), 'supplied_input_target': None})
    require(len(set(map(tuple, tokens))) == len(tokens) and len(set(bodies)) == len(bodies),
            'duplicate rendered prompts')
    within, across, blocks = [], [], []
    trie = {}
    for i, seq in enumerate(tokens):
        node, depth = trie, 0
        for token in seq:
            if token not in node:
                break
            depth += 1
            node = node[token]
        blocks.append(depth // job['block_size_tokens'] * job['block_size_tokens'])
        node = trie
        for token in seq:
            node = node.setdefault(token, {})
        for j in range(i):
            common = 0
            for left, right in zip(seq, tokens[j]):
                if left != right:
                    break
                common += 1
            (within if groups[i] == groups[j] else across).append(common)
    overlap = report['overlap']
    require(overlap['identity'] == job['rendering']['identity'], 'rendering identity mismatch')
    for key, values in [('lengths', list(map(len, tokens))), ('within_group_lcp', within),
                        ('across_group_lcp', across), ('block_aligned_reuse_ceiling', blocks), ('warmup_lcp', [])]:
        require(overlap[key] == distribution(values), f'independent overlap mismatch: {key}')
    require(overlap['sample_size'] == len(tokens) and overlap['exact_token_duplicates'] == 0 and
            overlap['exact_body_duplicates'] == 0, 'overlap count mismatch')
    require(overlap['body_order_sha256'] == sha(compact(rendered_records)), 'rendered token/body digest mismatch')
    return {'rendered_targets': len(tokens), 'within_lcp': within, 'across_lcp': across,
            'trie_reuse_ceiling_tokens': blocks}


def check_trace(job, output):
    trace = read_json(output)
    source = [json.loads(line) for line in Path(job['source']).read_text().splitlines()]
    require(trace['source'] == job['origin'], 'trace source mismatch')
    require(len(trace['requests']) == len(source), 'trace count mismatch')
    for i, (raw, request) in enumerate(zip(source, trace['requests'])):
        require(request['source_row_id'] == raw['id'] and request['source_line'] == i, 'trace order/ID mismatch')
        for key, expected in [('source_input_tokens', raw['input']), ('source_output_tokens', raw['output']),
                              ('prefix_parent', raw['prefix_parent']), ('session_parent', raw['session_parent'])]:
            require(request[key] == expected, f'trace reconstruction mismatch: {key}')
        require(request['source_time']['milliseconds'] == [0, 125, 250][i], 'trace timestamp mismatch')
        require(len(request['known_full_prefix_nodes']) == raw['input'] // 16, 'trace hash coverage mismatch')
    a, b, c = [r['known_full_prefix_nodes'] for r in trace['requests']]
    require(a[0] == b[0] and a[0] != c[0], 'trace prefix chain mismatch')
    require(trace['physical_cache_evidence'] == 'unavailable', 'trace physical evidence fabricated')
    return {'ordered_rows': len(source), 'session_edges': 1, 'prefix_edges': 1}


def check_hf(job, source, output):
    data = read_json(output)
    raw = Path(source).read_bytes()
    source_hash = sha(raw)
    require(data['origin'] == job['origin'], 'HF revision/lineage mismatch')
    require(source_hash == job['origin']['files'][0]['sha256'], 'HF source checksum mismatch')
    source_rows = [json.loads(line) for line in raw.splitlines()]
    require(len(source_rows) == len(data['rows']), 'HF row count mismatch')
    for i, (row, actual) in enumerate(zip(source_rows, data['rows'])):
        ident = sha(compact([job['origin']['source_id'], job['origin']['resolved_revision'], source_hash, i]))
        require(actual == {'record_id': ident, 'source_ordinal': i, 'row': row}, 'HF exact row mapping mismatch')
    return {'source_bytes': len(raw), 'rows': len(source_rows), 'ids': len({r['record_id'] for r in data['rows']})}


class PreBlocks(HTMLParser):
    def __init__(self):
        super().__init__()
        self.blocks, self.current = [], None

    def handle_starttag(self, tag, attrs):
        if tag == 'pre':
            self.current = ''

    def handle_data(self, data):
        if self.current is not None:
            self.current += data

    def handle_endtag(self, tag):
        if tag == 'pre':
            self.blocks.append(json.loads(self.current))
            self.current = None


def check_exports(job, directory):
    directory = Path(directory)
    data = read_json(directory / 'study.json')
    require(data['schema_version'] == 'kv-study-export.v1', 'export schema mismatch')
    csv_rows = list(csv.DictReader(io.StringIO((directory / 'study.csv').read_text())))
    html = PreBlocks()
    html.feed((directory / 'study.html').read_text())
    phases = data['phases']
    require(len(phases) == len(csv_rows) == len(job['seed']['phases']), 'export phase count mismatch')
    require(html.blocks[:len(phases)] == phases and html.blocks[-1] == data['report'], 'HTML evidence mismatch')
    for i, (phase, row, seed) in enumerate(zip(phases, csv_rows, job['seed']['phases'])):
        ident = seed['id']
        require(phase['phase_id'] == row['phase_id'] == f'phase-{i + 1}' and
                phase['phase_label'] == row['phase_label'] == 'phase: ' + ident, 'export phase order/ID mismatch')
        mock = [r for r in job['phases'][ident] if not r['warmup']]
        expected = {'requests': len(mock), 'errors': sum(not r['success'] for r in mock),
                    'cached_token_known': sum(r['success'] and r['cached_tokens'] is not None for r in mock)}
        expected['cached_token_unknown'] = expected['requests'] - expected['cached_token_known']
        for key, value in expected.items():
            require(phase[key] == int(row[key]) == value, f'export count mismatch: {key}')
        require(phase['ttft_unit'] == row['ttft_unit'] == 'seconds', 'export unit mismatch')
        require(phase['cache_state'] == row['cache_state'] == 'unknown', 'export physical claim')
        require(json.loads(row['cohorts_json']) == phase['cohorts'], 'CSV cohort mismatch')
        require(json.loads(row['scheduled_delays_json']) == phase['scheduled_delays'], 'CSV scheduling mismatch')
        require(phase['cohorts']['warmup_excluded'] == sum(r['warmup'] for r in job['phases'][ident]),
                'warmup inclusion mismatch')
    return {'phases': len(phases), 'formats': ['json', 'csv', 'html'], 'physical_cache_evidence': 'unknown'}
