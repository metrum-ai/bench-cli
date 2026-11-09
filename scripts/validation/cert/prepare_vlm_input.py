#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Prepare a frozen VLM prompt input with absolute, independently pinned image paths.

Run before freezing a case, never from a validator or record writer. The
result is a new input: it must itself be pinned in the lane manifest, and the
original prompt and image must be included among the frozen inputs.
"""
import argparse
import json
import re
from pathlib import Path

import manifest_tool


def prepare(source, source_sha256, image, image_sha256, root, out):
    source = Path(source).resolve(strict=True)
    image = Path(image).resolve(strict=True)
    root = Path(root).resolve(strict=True)
    out = Path(out)
    for path, expected in ((source, source_sha256), (image, image_sha256)):
        if (not path.is_file() or not isinstance(expected, str)
                or not re.fullmatch(r'[a-f0-9]{64}', expected)
                or manifest_tool.sha256_file(path) != expected):
            raise ValueError('source or image pin mismatch')
    if not root.is_dir():
        raise ValueError('root must be a directory')
    rows = []
    for line in source.read_text(encoding='utf-8').splitlines():
        if not line.strip():
            continue
        row = json.loads(line, object_pairs_hook=manifest_tool.unique_object)
        if not isinstance(row, dict) or ('image_url' in row) == ('image_urls' in row):
            raise ValueError('each prompt must specify exactly one image field')
        key = 'image_url' if 'image_url' in row else 'image_urls'
        values = [row[key]] if key == 'image_url' else row[key]
        if not isinstance(values, list) or len(values) != 1 or not isinstance(values[0], str):
            raise ValueError('each prompt must specify one local image')
        value = values[0]
        if '://' in value and not value.startswith('file://'):
            raise ValueError('remote image is forbidden')
        path = Path(value.removeprefix('file://'))
        if not path.is_absolute():
            path = root / path
        if path.resolve(strict=True) != image:
            raise ValueError('prompt image does not match pinned image')
        row[key] = str(image) if key == 'image_url' else [str(image)]
        rows.append(row)
    if not rows:
        raise ValueError('prompt input is empty')
    content = ''.join(json.dumps(row, ensure_ascii=False, separators=(',', ':')) + '\n' for row in rows)
    # Exclusive creation avoids silently overwriting any previously frozen input.
    with out.open('x', encoding='utf-8') as stream:
        stream.write(content)
    return {'path': str(out.resolve()), 'sha256': manifest_tool.sha256_file(out)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('source', 'source-sha256', 'image', 'image-sha256', 'root', 'out'):
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args()
    try:
        pin = prepare(args.source, args.source_sha256, args.image, args.image_sha256, args.root, args.out)
    except (ValueError, OSError) as exc:
        parser.exit(1, f'VLM input preparation failed: {exc}\n')
    print(json.dumps(pin, sort_keys=True))


if __name__ == '__main__':
    main()
