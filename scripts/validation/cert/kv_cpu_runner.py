#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Six offline KV CPU cases with independent oracles and rejection controls."""
import argparse
import copy
import datetime as dt
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

import execution_policy
import kv_cpu_checks as checks
import pretag_artifact
import result_writer
from execution_evidence import digest, phase, require_python

CASES = ('BC-KV-010', 'BC-KV-011', 'BC-KV-012', 'BC-KV-013', 'BC-KV-014', 'BC-KV-015')
REPO = Path(__file__).resolve().parents[3]


def now():
    return dt.datetime.now(dt.timezone.utc).isoformat().replace('+00:00', 'Z')


def dump(path, data):
    Path(path).write_bytes(checks.compact(data) + b'\n')


def pin(item):
    path = Path(item['path']).resolve(strict=True)
    checks.require(digest(path) == item['sha256'], 'frozen input changed')
    return path


def rights(license_hash):
    return {'decision': 'allowed', 'license': 'Apache-2.0', 'license_text_sha256': license_hash,
            'component_terms': {}, 'gated': False, 'terms_review': None,
            'redistribution_allowed': False, 'privacy_handling': 'Offline local adapter validation; no raw publication.',
            'unresolved': []}


def document_job(source, public=False, rendering=False):
    raw = source.read_bytes()
    lock = checks.fixture_lock()
    revision = lock['public_inputs']['revision'] if public else digest(checks.FIXTURES / 'lock.json')[:40]
    fields = ('prompt',) * 4 if public else ('id', 'doc', 'context', 'question')
    job = {'schema_version': 'public-dataset-job.v1',
        'origin': {'source_id': 'metrum-ai/prompt-library' if public else 'fixture/kv-cpu',
                   'resolved_revision': revision, 'config': 'sample' if public else 'fixture', 'split': 'train',
                   'files': [{'path': source.name, 'sha256': checks.sha(raw), 'bytes': len(raw)}]},
        'rights': rights(lock['public_inputs']['files']['LICENSE'] if public else lock['files']['LICENSE']),
        'mapping': {'adapter_id': 'kv-cpu-document', 'adapter_version': '1',
                    'expected_config': 'sample' if public else 'fixture', 'expected_split': 'train',
                    'row_id_field': fields[0], 'document_id_field': fields[1], 'document_field': fields[2],
                    'questions_field': fields[3], 'question_array': False, 'output_budget': 8},
        'formats': {source.name: {'format': 'jsonl'}}, 'local_files': {source.name: str(source)},
        'limits': {'max_file_bytes': 16 * 1024 * 1024, 'max_total_bytes': 16 * 1024 * 1024,
                   'max_rows': 10000, 'max_targets': 10000, 'max_body_bytes': 32 * 1024 * 1024, 'max_pairs': 64},
        'count': 8 if public else None, 'constraints': {'content_unique': rendering, 'max_pair_lcp': None},
        'block_size_tokens': 2, 'rendering': None}
    if rendering:
        job['rendering'] = {'identity': {'tokenizer_id': 'fixture/wordlevel', 'tokenizer_revision': revision,
            'tokenizer_sha256': lock['native_vocabulary_sha256'], 'chat_template_sha256': lock['files']['template.jinja'],
            'add_special_tokens': False, 'add_generation_prompt': True, 'hint_nonce_included': False,
            'renderer_version': 'minijinja-chat.v2'}, 'tokenizer_path': str(checks.FIXTURES / 'wordlevel.json'),
            'template_path': str(checks.FIXTURES / 'template.jinja'), 'special_tokens': {},
            'max_artifact_bytes': 8192, 'max_rendered_bytes': 8192, 'max_tokens': 128, 'template_fuel': 10000}
    return job


def trace_job(source):
    base = document_job(source)
    return {'kind': 'trace', 'origin': base['origin'], 'rights': base['rights'], 'source': str(source),
        'mapping': {'adapter_id': 'kv-cpu-trace', 'adapter_version': '1', 'row_id_field': 'id',
            'source_domain': 'local_pinned', 'selection': None, 'parent_root_sentinel': None, 'metadata_fields': [],
            'input_tokens_field': 'input', 'output_tokens_field': 'output', 'timestamp_field': 'time',
            'time_unit': 'seconds', 'time_scale': {'numerator': 1, 'denominator': 1}, 'rounding': 'reject',
            'hashes_field': 'hashes', 'block_tokens': 16, 'hash_coverage': 'full_blocks_only',
            'hash_encoding': {'kind': 'complete_chain'}, 'prefix_parent_field': 'prefix_parent',
            'session_parent_field': 'session_parent'}}


class Case:
    def __init__(self, root, binary, probe, env):
        self.root, self.binary, self.probe, self.env = root, binary, probe, env
        self.controls = []

    def invoke(self, name, job, probe=False, reject=False):
        job_path = self.root / (name + '-job.json')
        dump(job_path, job)
        if probe:
            checks.require(self.probe is not None, 'workflow package lacks offline trace/export probe')
            out = self.root / name
            argv = [str(self.probe), '--job', str(job_path), '--out', str(out)]
        else:
            out = self.root / (name + '.jsonl')
            argv = [str(self.binary), 'dataset', '--job', str(job_path), '--output', str(out),
                    '--report', str(self.root / (name + '-report.json')), '--offline',
                    '--cache-dir', str(self.root / 'empty-cache')]
        process = subprocess.run(argv, env=self.env, capture_output=True, timeout=120, check=False)
        (self.root / (name + '.stdout')).write_bytes(process.stdout)
        (self.root / (name + '.stderr')).write_bytes(process.stderr)
        dump(self.root / (name + '-process.json'), {'argv': argv, 'returncode': process.returncode})
        if reject:
            checks.require(process.returncode != 0, f'negative control accepted: {name}')
            checks.require(not out.exists(), f'rejected input emitted output: {name}')
            self.controls.append(name)
        else:
            checks.require(process.returncode == 0, f'product command failed: {name}; see stderr')
        return out

    def reject_job(self, name, job, mutate, probe=False):
        bad = copy.deepcopy(job)
        mutate(bad)
        self.invoke(name, bad, probe=probe, reject=True)

    def reject_source(self, name, job, data, probe=False):
        source = self.root / (name + '-source.jsonl')
        source.write_bytes(data)
        bad = copy.deepcopy(job)
        file = bad['origin']['files'][0]
        file.update(sha256=digest(source), bytes=source.stat().st_size)
        if probe:
            bad['source'] = str(source)
        else:
            bad['local_files'] = {file['path']: str(source)}
        self.invoke(name, bad, probe=probe, reject=True)

    def altered(self, name, checker):
        try:
            checker()
        except (ValueError, KeyError, AssertionError):
            self.controls.append(name)
        else:
            raise ValueError(f'altered output accepted: {name}')

    def execute(self, case, public):
        if case == 'BC-KV-010':
            source = public / 'sample.jsonl'
            origin = document_job(source, public=True)['origin']
            job = {'kind': 'hf', 'origin': origin,
                   'rights': rights(digest(public / 'LICENSE')), 'cache': str(self.root / 'hf-cache')}
            # Content-addressed cache expected by the actual HF materializer.
            cache = Path(job['cache']); cache.mkdir()
            cached = cache / digest(source)
            shutil.copyfile(source, cached)
            out = self.invoke('hf', job, probe=True)
            observed = checks.check_hf(job, source, out / 'hf.json')
            again = self.invoke('hf-repeat', job, probe=True)
            checks.require((out / 'hf.json').read_bytes() == (again / 'hf.json').read_bytes(),
                           'HF mapping not deterministic')
            self.reject_job('hf-checksum', job, lambda b: b['origin']['files'][0].update(sha256='0' * 64), True)
            cached.write_bytes(b'corrupt cache')
            self.invoke('hf-corrupt-cache', job, probe=True, reject=True)
            shutil.copyfile(source, cached)
            self.reject_job('hf-moving-revision', job, lambda b: b['origin'].update(resolved_revision='main'), True)
            self.reject_job('hf-remote-code', job, lambda b: b.update(trust_remote_code=True), True)
            changed = copy.deepcopy(job)
            changed['origin']['resolved_revision'] = '0' * 40
            self.altered('hf-modified-revision', lambda: checks.require(
                changed['origin']['resolved_revision'] == checks.fixture_lock()['public_inputs']['revision'],
                'HF revision differs from frozen input'))
            altered = checks.read_json(out / 'hf.json'); altered['rows'][0]['source_ordinal'] = 1
            bad = self.root / 'altered-hf.json'; dump(bad, altered)
            self.altered('hf-altered-mapping', lambda: checks.check_hf(job, source, bad))
            return observed
        if case == 'BC-KV-012':
            job = trace_job(checks.FIXTURES / 'trace.jsonl')
            out = self.invoke('trace', job, probe=True)
            observed = checks.check_trace(job, out / 'trace.json')
            again = self.invoke('trace-repeat', job, probe=True)
            checks.require((out / 'trace.json').read_bytes() == (again / 'trace.json').read_bytes(),
                           'trace serialization not deterministic')
            rows = [json.loads(line) for line in Path(job['source']).read_text().splitlines()]
            for name, field, value in [('missing-field', 'input', None), ('duplicate-id', 'id', 'a'),
                                       ('bad-boundary', 'session_parent', 'missing')]:
                bad = copy.deepcopy(rows)
                if value is None:
                    del bad[1][field]
                else:
                    bad[1][field] = value
                self.reject_source(name, job, b'\n'.join(checks.compact(r) for r in bad) + b'\n', probe=True)
            altered = checks.read_json(out / 'trace.json')
            altered['requests'].reverse()
            path = self.root / 'altered-trace.json'
            dump(path, altered)
            self.altered('altered-trace-order', lambda: checks.check_trace(job, path))
            return observed
        if case == 'BC-KV-015':
            job = checks.read_json(checks.FIXTURES / 'export.json')['job']
            out = self.invoke('export', job, probe=True)
            observed = checks.check_exports(job, out)
            self.reject_job('physical-claim', job, lambda b: b['seed']['phases'][0].update(cache_state='observed_resident'), True)
            self.reject_job('missing-phase', job, lambda b: b['phases'].pop('warm'), True)
            self.reject_job('malformed-state', job, lambda b: b['seed'].update(schema_version='bad'), True)
            for suffix in ('json', 'csv', 'html'):
                changed = self.root / ('altered-' + suffix)
                shutil.copytree(out, changed)
                path = changed / ('study.' + suffix)
                if suffix == 'json':
                    data = checks.read_json(path); data['phases'][0]['requests'] += 1; dump(path, data)
                elif suffix == 'csv':
                    path.write_text(path.read_text().replace('seconds', 'tokens'))
                else:
                    path.write_text(path.read_text().replace('&quot;requests&quot;: 3', '&quot;requests&quot;: 4'))
                self.altered('altered-' + suffix, lambda: checks.check_exports(job, changed))
            return observed
        source = checks.FIXTURES / 'documents.jsonl'
        job = document_job(source, rendering=case == 'BC-KV-014')
        if job['rendering']:
            native = self.root / 'tokenizer.json'
            native.write_bytes(checks.compact(checks.read_json(checks.FIXTURES / 'wordlevel.json')['tokenizer']))
            checks.require(digest(native) == checks.fixture_lock()['native_vocabulary_sha256'], 'native tokenizer mismatch')
            job['rendering']['tokenizer_path'] = str(native)
        out = self.invoke('documents', job)
        report = checks.read_json(self.root / 'documents-report.json')
        observed = checks.check_documents(job, source, out, report)
        again = self.invoke('documents-repeat', job)
        checks.require(out.read_bytes() == again.read_bytes(), 'canonical serialization not deterministic')
        self.reject_job('bad-checksum', job, lambda b: b['origin']['files'][0].update(sha256='0' * 64))
        self.reject_job('moving-revision', job, lambda b: b['origin'].update(resolved_revision='main'))
        self.reject_job('remote-code', job, lambda b: b.update(trust_remote_code=True))
        if case in ('BC-KV-011', 'BC-KV-014'):
            rows = [json.loads(line) for line in source.read_text().splitlines()]
            bad = copy.deepcopy(rows); del bad[1]['context']
            self.reject_source('missing-document', job, b'\n'.join(checks.compact(r) for r in bad) + b'\n')
            bad = copy.deepcopy(rows); bad[1]['id'] = bad[0]['id']
            self.reject_source('duplicate-id', job, b'\n'.join(checks.compact(r) for r in bad) + b'\n')
            self.reject_job('ambiguous-question', job, lambda b: b['mapping'].update(question_array=True))
        if case == 'BC-KV-013':
            for name, values in [('unknown-rights', {'license': None}), ('missing-license-hash', {'license_text_sha256': None}),
                                 ('review-required', {'decision': 'review_required'})]:
                self.reject_job(name, job, lambda b, v=values: b['rights'].update(v))
            bad_report = copy.deepcopy(report); bad_report['manifest']['origin']['resolved_revision'] = '0' * 40
            self.altered('conflicting-revision', lambda: checks.check_documents(job, source, out, bad_report))
            checks.require(report['manifest']['rights']['license_text_sha256'] ==
                           digest(checks.FIXTURES / 'LICENSE'), 'license text hash mismatch')
        if case == 'BC-KV-014':
            observed.update(checks.check_overlap(job, source, report))
            for key in ('tokenizer_sha256', 'chat_template_sha256'):
                self.reject_job('mismatched-' + key, job, lambda b, k=key: b['rendering']['identity'].update({k: '0' * 64}))
            rows = [json.loads(line) for line in source.read_text().splitlines()]
            bad = copy.deepcopy(rows); bad[1].update(context=bad[0]['context'], question=bad[0]['question'])
            duplicate = self.root / 'duplicates.jsonl'
            duplicate.write_bytes(b'\n'.join(checks.compact(r) for r in bad) + b'\n')
            duplicate_job = document_job(duplicate, rendering=True); duplicate_job['count'] = 3
            duplicate_job['rendering']['tokenizer_path'] = str(native)
            self.invoke('duplicate-rendered-prompt', duplicate_job, reject=True)
            changed = copy.deepcopy(report); changed['overlap']['lengths']['max'] += 1
            self.altered('altered-token-overlap', lambda: checks.check_overlap(job, source, changed))
        altered = self.root / 'altered-output.jsonl'
        data = out.read_bytes().replace(b'Question:', b'Changed!:')
        altered.write_bytes(data)
        self.altered('altered-output-bytes', lambda: checks.check_documents(job, source, altered, report))
        return observed


def run(manifest_path, out, dry_run=False):
    require_python()
    config = checks.read_json(manifest_path)
    cases = config.get('cases', {})
    checks.require(len(cases) == 1 and next(iter(cases)) in CASES and config.get('lane') == 'KVC', 'one KVC case required')
    case = next(iter(cases))
    candidate = config['candidate_sha']
    checks.require(result_writer.SHA.fullmatch(candidate), 'full candidate SHA required')
    if not dry_run:
        from kv_lanes.kvc import preflight
        outer_path = os.environ.get('CERT_INPUT_MANIFEST')
        checks.require(outer_path is not None, 'CPU certification requires the isolated frozen-envelope launcher')
        outer = checks.read_json(outer_path)
        preflight(outer)
        checks.require(outer['lane_manifest'] == {'path': str(Path(manifest_path).resolve()),
            'sha256': digest(manifest_path)} and outer['cases'][case]['out'] == str(Path(out).resolve()),
            'CPU inner spec/output differs from launched envelope')
    tools = execution_policy.validate_case_tools(config['tools'], 'KVC')
    checks.require(tools['python3']['path'] == str(Path(sys.executable).resolve()), 'interpreter differs from frozen pin')
    out = Path(out).resolve()
    out.mkdir()  # Refuse to overwrite any evidence, including failed attempts.
    if dry_run:
        dump(out / result_writer.SMOKE_MARKER, {'classification': result_writer.SMOKE_CLASS,
            'expectations_sha256': digest(checks.FIXTURES / 'lock.json')})
    if dry_run:
        env = execution_policy.prepare(tools, out, {})
        os.environ.update(CERT_INPUT_MANIFEST=str(Path(manifest_path).resolve()), CERT_CASE_ID=case,
                          CERT_PHASE_ROOT=str(out), CERT_TOOL_BIN=env['PATH'], PATH=env['PATH'])
    else:
        env = dict(os.environ)
        checks.require('CERT_MANIFEST' not in env and 'CERT_LANE' not in env, 'r4 manifest variables forbidden for CPU KV')
        execution_policy.verify(tools, Path(env['CERT_TOOL_BIN']))
    env.update(HF_HUB_OFFLINE='1', HF_DATASETS_OFFLINE='1', NO_BANNER='1')
    evidence = out / 'evidence'; evidence.mkdir()
    started, clock = now(), time.monotonic()
    observed, reason, state = {}, None, 'FAIL'
    controls = []
    try:
        lock = checks.fixture_lock()
        checks.require(config['public_revision'] == lock['public_inputs']['revision'], 'public revision mismatch')
        public = Path(config['public_inputs']).resolve()
        if case in ('BC-KV-010', 'BC-KV-011', 'BC-KV-013', 'BC-KV-014'):
            checks.verify_files(public, lock['public_inputs']['files'])
        members = pretag_artifact.verify(checks.read_json(pin(config['workflow_origin'])), candidate)
        binary = pin(config['binary'])
        checks.require(members.get(binary.name) == digest(binary), 'binary not in candidate workflow archive')
        probe = pin(config['probe']) if config.get('probe') else None
        if probe:
            checks.require(members.get(probe.name) == digest(probe), 'probe not in candidate workflow archive')
        if dry_run:
            phase(out, case, 'inputs-ready')
        runner = Case(evidence, binary, probe, env)
        observed = runner.execute(case, public)
        controls = runner.controls
        checks.fixture_lock()
        pin(config['binary'])
        if config.get('probe'):
            pin(config['probe'])
        if case in ('BC-KV-010', 'BC-KV-011', 'BC-KV-013', 'BC-KV-014'):
            checks.verify_files(public, lock['public_inputs']['files'])
        state = 'PASS'
    except (ValueError, KeyError, OSError, subprocess.SubprocessError) as exc:
        reason = str(exc)
    elapsed = time.monotonic() - clock
    dump(evidence / 'checks.json', {'ID': case, 'result': state, 'reason': reason,
        'observed': observed, 'rejection_controls': controls, 'elapsed_seconds': elapsed,
        'classification': 'offline dry run, not certification' if dry_run else 'candidate execution'})
    paths = [str(p.relative_to(out)) for p in sorted(evidence.rglob('*')) if p.is_file()]
    record = {'ID': case, 'candidate_sha': candidate, 'result': state, 'reason': reason,
        'owner': config['owner'], 'evidence_link': config['evidence_link'], 'attempt': 1,
        'start_utc': started, 'end_utc': now(), 'gpu_class': 'CPU-only', 'driver': 'not applicable',
        'versions': {'python': sys.version, 'binary_sha256': config['binary']['sha256'],
                     'fixture_lock_sha256': digest(checks.FIXTURES / 'lock.json'),
                     'workflow_origin_sha256': config['workflow_origin']['sha256']},
        'thresholds': {'all_positive_checks': True, 'all_rejection_controls': True, 'independent_reconciliation': True},
        'observed_values': observed or {'execution_error': reason}, 'evidence_paths': paths,
        'notes': 'Offline dry run, not certification.' if dry_run else 'Offline adapter/export correctness only.'}
    result_writer.write_record(out, record)
    if dry_run:
        phase(out, case, 'record-validated')
    return state == 'PASS'


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--dry-run', action='store_true')
    parser.add_argument('--case', choices=CASES)
    args = parser.parse_args(argv)
    if args.case:
        checks.require(set(checks.read_json(args.manifest).get('cases', {})) == {args.case}, 'launcher case mismatch')
    return 0 if run(args.manifest, args.out, args.dry_run) else 1


if __name__ == '__main__':
    raise SystemExit(main())
