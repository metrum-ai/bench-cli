#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Freeze the canonical W1 dataset used in the 2026-10-27 validation.

Requires Python 3.12.16, huggingface_hub==2.3.0, and pyarrow==26.0.0.
Usage: python3 freeze_w1.py --output /path/to/W1.jsonl
The exact expected digest is checked before the output file is written.
"""

import argparse
import hashlib
import json
import random
from pathlib import Path

from huggingface_hub import hf_hub_download
import pyarrow.parquet as pq

REVISION = "96df5e686bee6baa90b8bee7c28b81fa3fa6223d"
CANONICAL_SHA256 = "4adc601afa3e4ce355a204dd042f1f64c3a24bdd7ef57a8a9c9ccd21d0ce0dd7"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    path = hf_hub_download(
        "abisee/cnn_dailymail", "3.0.0/validation-00000-of-00001.parquet",
        revision=REVISION, repo_type="dataset",
    )
    articles = pq.read_table(path, columns=["article"]).column("article").to_pylist()
    random.Random(42).shuffle(articles)
    if len(articles) < 512:
        raise SystemExit("validation split has fewer than 512 articles")
    data = "".join(json.dumps({"prompt": article}) + "\n" for article in articles[:512]).encode("utf-8")
    digest = hashlib.sha256(data).hexdigest()
    if digest != CANONICAL_SHA256:
        raise SystemExit(f"W1 digest mismatch: {digest}; output not written")
    args.output.write_bytes(data)
    print(f"W1: 512 rows, sha256 {digest}")


if __name__ == "__main__":
    main()
