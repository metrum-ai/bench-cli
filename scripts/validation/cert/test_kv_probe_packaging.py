# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Certification helper cannot expand installation or published archive targets."""
import hashlib
import io
import json
import subprocess
import sys
import tarfile
import tempfile
import textwrap
import tomllib
import unittest
from pathlib import Path

import check_cert_probe_packaging as packaging
import pretag_artifact

REPO = Path(__file__).resolve().parents[3]


class ProbePackagingTests(unittest.TestCase):
    def test_default_install_targets_and_certification_feature(self):
        manifest = tomllib.loads((REPO / 'Cargo.toml').read_text())
        self.assertNotIn('cert-probes', manifest['features']['default'])
        helper = next(target for target in manifest['bin'] if target['name'] == packaging.PROBE)
        self.assertEqual(['cert-probes'], helper['required-features'])
        normal = {target['name'] for target in manifest['bin']
                  if set(target.get('required-features', [])) <= set(manifest['features']['default'])}
        self.assertEqual(packaging.PUBLIC_BINS, normal)
        self.assertIn(helper['path'], manifest['package']['exclude'])
        pretag = (REPO / '.github/workflows/pretag-certification.yml').read_text()
        self.assertIn('--features otlp,tokenizer --target', pretag)
        self.assertIn('--features otlp,tokenizer,cert-probes --target', pretag)
        self.assertIn('--bin ' + packaging.PROBE, pretag)
        release = (REPO / '.github/workflows/release.yml').read_text()
        self.assertNotIn('--features otlp,tokenizer,cert-probes', release)
        self.assertEqual(['otlp', 'tokenizer'], pretag_artifact.measured_features('metrum-ai-bench-cli-llm'))
        self.assertEqual(['otlp', 'tokenizer', 'cert-probes'], pretag_artifact.measured_features(packaging.PROBE))

    def test_release_asset_selector_ignores_probe_even_when_a_stale_file_exists(self):
        # Execute the actual metadata selector, with the probe present among
        # targets. Its output controls which existing executables get copied.
        release = (REPO / '.github/workflows/release.yml').read_text()
        code = release.split("mapfile -t bins", 1)[1].split("python3 -c '", 1)[1].split("\n          ')", 1)[0]
        manifest = tomllib.loads((REPO / 'Cargo.toml').read_text())
        metadata = {'packages': [{'targets': [{'name': target['name'], 'kind': ['bin'],
            'required-features': target.get('required-features', [])} for target in manifest['bin']]}]}
        result = subprocess.run([sys.executable, '-c', textwrap.dedent(code)], input=json.dumps(metadata),
                                text=True, capture_output=True, check=True)
        self.assertEqual(packaging.PUBLIC_BINS, set(result.stdout.splitlines()))

    def test_pack_case_expectations_are_unchanged(self):
        rows = (REPO / 'docs/RELEASE_CERTIFICATION_CATALOG.csv').read_text().splitlines()
        for case, expected in [('BC-PACK-001', 'a72c0b8979e5951f254473e64ca73cc8f29bf11e6448b49c76326275eb59e544'),
                               ('BC-PACK-002', 'c79fb9e935d990204acfb718c17599f7996dec7f6fcdd27f3e12c1ee87318865')]:
            line = next(row for row in rows if row.startswith(case + ','))
            self.assertEqual(expected, hashlib.sha256(line.encode()).hexdigest())

    def test_actual_crate_checker_rejects_probe_target_or_source(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'synthetic.crate'
            for mode in ('normal', 'target', 'source'):
                bins = packaging.PUBLIC_BINS | ({packaging.PROBE} if mode == 'target' else set())
                manifest = '\n'.join(f'[[bin]]\nname={json.dumps(name)}' for name in sorted(bins)).encode()
                with tarfile.open(path, 'w:gz') as archive:
                    files = {'fixture/Cargo.toml': manifest}
                    if mode == 'source':
                        files['fixture/src/bin/' + packaging.PROBE + '.rs'] = b'synthetic fixture'
                    for name, data in files.items():
                        info = tarfile.TarInfo(name); info.size = len(data)
                        archive.addfile(info, io.BytesIO(data))
                if mode == 'normal':
                    self.assertEqual(sorted(packaging.PUBLIC_BINS), packaging.check_archive(path))
                else:
                    with self.assertRaises(ValueError):
                        packaging.check_archive(path)


if __name__ == '__main__':
    unittest.main()
