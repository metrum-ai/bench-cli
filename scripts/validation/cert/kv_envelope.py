# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Shared offline KV envelope validation; no r4 serving-contract preflight."""
import json
import os
import re
from pathlib import Path

import execution_policy
import harness_integrity
import manifest_tool
import result_writer
from execution_evidence import digest, require_python

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
SCHEMA = REPO / 'schemas/kv-r5-frozen.v1.json'


def load(path):
    return json.loads(Path(path).read_text(), object_pairs_hook=manifest_tool.unique_object,
                      parse_constant=lambda x: (_ for _ in ()).throw(ValueError(f'invalid JSON number: {x}')))


def _shape(value, schema, document, location='$'):
    """Evaluate only the local schema vocabulary, failing on unsupported keywords.

    Keeping this small vocabulary explicit makes standalone CPU preflight work
    without adding a Python package dependency or permitting remote references.
    """
    supported = {'$ref', '$comment', '$schema', '$defs', 'title', 'description', 'type', 'required',
                 'properties', 'patternProperties', 'additionalProperties', 'minProperties', 'maxProperties',
                 'minItems', 'maxItems', 'items', 'minLength', 'pattern', 'const', 'enum'}
    if set(schema) - supported:
        raise ValueError('unsupported KV schema keyword')
    if '$ref' in schema:
        ref = schema['$ref']
        if not ref.startswith('#/$defs/') or '/' in ref[len('#/$defs/'):]:
            raise ValueError('only local KV schema definitions are supported')
        return _shape(value, document['$defs'][ref[len('#/$defs/'):]], document, location)
    kind = schema.get('type')
    if kind and not {'object': isinstance(value, dict), 'array': isinstance(value, list),
                     'string': isinstance(value, str)}.get(kind, False):
        raise ValueError(f'{location}: invalid type')
    if 'const' in schema and value != schema['const']:
        raise ValueError(f'{location}: wrong constant')
    if 'enum' in schema and value not in schema['enum']:
        raise ValueError(f'{location}: invalid enum')
    if isinstance(value, dict):
        if not set(schema.get('required', [])) <= value.keys():
            raise ValueError(f'{location}: missing required fields')
        if len(value) < schema.get('minProperties', 0) or len(value) > schema.get('maxProperties', float('inf')):
            raise ValueError(f'{location}: invalid property count')
        for key, child in value.items():
            matches = ([schema['properties'][key]] if key in schema.get('properties', {}) else [])
            matches += [rule for pattern, rule in schema.get('patternProperties', {}).items() if re.search(pattern, key)]
            if not matches:
                extra = schema.get('additionalProperties', True)
                if extra is False:
                    raise ValueError(f'{location}: unknown field {key}')
                if isinstance(extra, dict):
                    matches = [extra]
            for rule in matches:
                _shape(child, rule, document, location + '.' + key)
    if isinstance(value, list):
        if len(value) < schema.get('minItems', 0) or len(value) > schema.get('maxItems', float('inf')):
            raise ValueError(f'{location}: invalid item count')
        for i, item in enumerate(value):
            _shape(item, schema.get('items', {}), document, f'{location}[{i}]')
    if isinstance(value, str):
        if len(value) < schema.get('minLength', 0) or ('pattern' in schema and not re.search(schema['pattern'], value)):
            raise ValueError(f'{location}: invalid string')


def validate_shape(manifest):
    schema = load(SCHEMA)
    _shape(manifest, schema, schema)
    return manifest


def pin(item, executable=False):
    if (not isinstance(item, dict) or not isinstance(item.get('path'), str)
            or not isinstance(item.get('sha256'), str) or not re.fullmatch(r'[a-f0-9]{64}', item['sha256'])):
        raise ValueError('invalid KV file pin')
    path = Path(item['path'])
    if (not path.is_absolute() or str(path) != item['path'] or '..' in path.parts
            or str(path.resolve(strict=True)) != str(path) or not path.is_file()):
        raise ValueError('KV pin must be a canonical absolute file')
    if digest(path) != item['sha256'] or (executable and not os.access(path, os.X_OK)):
        raise ValueError('KV file pin changed or is not executable')
    return path


def pins(value):
    if isinstance(value, dict):
        if 'path' in value and 'sha256' in value:
            yield value
        for child in value.values():
            yield from pins(child)
    elif isinstance(value, list):
        for child in value:
            yield from pins(child)


def validate(manifest, lane=None):
    """Recheck source, artifact origin, transitive pins, tools and exact argv.

    Returns the single case mapping. Lane preflight must additionally validate
    its engine/CPU spec, source identity, actual host and cleanup requirements.
    """
    require_python()
    validate_shape(manifest)
    import kv_dispatch
    import validate_results
    import build_roles
    case = next(iter(manifest['cases']))
    actual_lane = manifest['lane']
    if lane is not None and lane != actual_lane:
        raise ValueError('KV envelope lane mismatch')
    kv_dispatch.require_lane(case, actual_lane)
    row = validate_results.load_catalog(REPO / 'docs/RELEASE_CERTIFICATION_CATALOG.csv')[case]
    if row['Parallel group'] != actual_lane:
        raise ValueError('KV catalog lane mismatch')
    validate_results.require_executable(row)
    if manifest['harness_repo'] != str(REPO):
        raise ValueError('KV harness repository differs from executing checkout')
    closure = harness_integrity.verify(REPO, manifest['candidate_sha'])
    if manifest['harness_closure'] != closure:
        raise ValueError('KV harness closure differs from candidate')
    entry = manifest['cases'][case]
    out = Path(entry['out'])
    if str(out) != entry['out'] or '..' in out.parts or str(out.resolve()) != str(out):
        raise ValueError('KV output must be a canonical absolute path')
    inputs = {}
    for item in manifest['inputs']:
        path = pin(item)
        if str(path) in inputs:
            raise ValueError('duplicate KV transitive input')
        inputs[str(path)] = item['sha256']
    receipt = pin(manifest['build_receipt'])
    spec_path = pin(manifest['lane_manifest'])
    spec = load(spec_path)
    if spec.get('candidate_sha') != manifest['candidate_sha']:
        raise ValueError('KV inner specification candidate mismatch')
    if spec.get('case') != case or spec.get('tools') != manifest['tools']:
        raise ValueError('KV inner specification case/tools mismatch')
    # The existing receipt verifier independently binds the successful workflow,
    # archive checksum, package members and measured/validation-only roles.
    measured, _ = build_roles.load_receipt(receipt, manifest['candidate_sha'])
    if manifest['candidate'] not in measured:
        raise ValueError('KV candidate is not a shipped measured workflow artifact')
    pin(manifest['candidate'], executable=True)
    execution_policy.validate_case_tools(manifest['tools'], actual_lane)
    if 'dirname' not in manifest['tools']:
        raise ValueError('KV case entry requires pinned dirname')
    cap, grace = spec.get('timeout_seconds'), spec.get('cleanup_grace_seconds')
    if type(cap) is not int or type(grace) is not int or not 0 < grace < cap:
        raise ValueError('KV case requires frozen timeout and cleanup grace')
    root = spec.get('cgroup_root')
    if (not isinstance(root, str) or not Path(root).is_absolute() or '..' in Path(root).parts
            or str(Path(root)) != root):
        raise ValueError('KV case requires a canonical delegated cgroup root')
    launcher = HERE / 'case_entry.sh'
    if pin(entry.get('launcher'), executable=True) != launcher:
        raise ValueError('KV command requires the pinned case entry')
    expected = [str(launcher), '--case', case, '--lane', actual_lane.lower(),
                '--manifest', str(spec_path), '--manifest-sha256', manifest['lane_manifest']['sha256'], '--out', str(out)]
    if entry['command'] != expected:
        raise ValueError('KV command differs from frozen lane specification')
    # Every pin in the inner spec, build origin, tools and interpreted harness
    # must appear in the explicitly frozen transitive closure.
    values = [manifest, spec, load(receipt)]
    for item in pins(values):
        path = pin(item)
        result_writer.require_input_origin(out, path.parent)
        if inputs.get(str(path)) != item['sha256']:
            raise ValueError('KV transitive input closure is incomplete')
    log = receipt.parent / 'validation-only.log'
    if inputs.get(str(log)) != digest(log):
        raise ValueError('KV build log is missing from transitive inputs')
    if inputs.get(str(SCHEMA)) != digest(SCHEMA):
        raise ValueError('KV envelope schema is missing from transitive inputs')
    return manifest['cases']
