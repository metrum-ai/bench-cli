# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline release provenance and staging fixtures."""
import contextlib
import hashlib
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import perf_all_smi as smi
import perf_all_smi_prepare as prepare


def digest(data):
    return hashlib.sha256(data).hexdigest()


class ReleaseAssetProvenance(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.data = b'\x7fELF\x02\x01' + bytes(12) + (62).to_bytes(2, 'little') + bytes(44)
        self.binary = self.root / 'all-smi'
        self.binary.write_bytes(self.data)
        self.binary.chmod(0o755)
        self.asset = self.root / 'all-smi-linux-x86_64.tar.gz'
        self.checksum = self.root / 'published.sha256'
        self.table = patch.dict(smi.ASSETS, {})
        self.table.start()
        self.addCleanup(self.table.stop)
        host = patch.object(smi.platform, 'machine', return_value='x86_64')
        host.start()
        self.addCleanup(host.stop)
        self.make_asset()

    def make_asset(self, count=1, kind=tarfile.REGTYPE):
        with tarfile.open(self.asset, 'w:gz') as tar:
            for _ in range(count):
                member = tarfile.TarInfo('all-smi')
                member.type = kind
                member.linkname = 'elsewhere' if kind != tarfile.REGTYPE else ''
                member.size = len(self.data) if kind == tarfile.REGTYPE else 0
                tar.addfile(member, io.BytesIO(self.data))
        self.asset_sha = digest(self.asset.read_bytes())
        libc = 'musl' if 'musl' in self.asset.name else 'gnu'
        smi.ASSETS[self.asset.name] = ('x86_64', libc, self.asset_sha)
        self.checksum.write_text(f'{self.asset_sha}  {self.asset.name}\n')

    def verify(self, pin=None):
        return smi.verify_release_asset(self.binary, pin or digest(self.binary.read_bytes()),
                                       self.asset, self.checksum, digest(self.checksum.read_bytes()) if self.checksum.exists() else '0' * 64,
                                       self.root / 'evidence')

    def test_gnu(self):
        result = self.verify()
        self.assertEqual(result['libc'], 'gnu')
        self.assertEqual(result['arch'], 'x86_64')
        self.assertEqual(result['published_checksum_file_sha256'], digest(self.checksum.read_bytes()))
        self.assertFalse(result['rebuild_equality_required'])
        self.assertEqual(json.loads((self.root / 'evidence/all-smi-provenance.json').read_text()), result)

    def test_musl(self):
        self.asset = self.root / 'all-smi-linux-x86_64-musl.tar.gz'
        self.make_asset()
        self.assertEqual(self.verify()['libc'], 'musl')

    def test_unknown_asset(self):
        self.asset = self.asset.rename(self.root / 'unknown.tar.gz')
        with self.assertRaises(ValueError): self.verify()

    def test_missing_checksum(self):
        self.checksum.unlink()
        with self.assertRaises(ValueError): self.verify()

    def test_malformed_wrong_name_and_checksum_mismatch(self):
        for text in ('bad', f'{self.asset_sha}  wrong.tar.gz\n',
                     f'{self.asset_sha}  {self.asset.name}\nextra\n',
                     f'{"0" * 64}  {self.asset.name}\n'):
            with self.subTest(text=text):
                self.checksum.write_text(text)
                with self.assertRaises(ValueError): self.verify()

    def test_checksum_file_pin_changed(self):
        with self.assertRaises(ValueError):
            smi.inspect_release_asset(self.asset, self.checksum, '0' * 64)

    def test_asset_changed(self):
        self.asset.write_bytes(self.asset.read_bytes() + b'changed')
        with self.assertRaises(ValueError): self.verify()

    def test_binary_not_in_asset(self):
        self.binary.write_bytes(b'other')
        with self.assertRaises(ValueError): self.verify()

    def test_member_count_and_links(self):
        for count, kind in ((0, tarfile.REGTYPE), (2, tarfile.REGTYPE),
                            (1, tarfile.SYMTYPE), (1, tarfile.LNKTYPE)):
            with self.subTest(count=count, kind=kind):
                self.make_asset(count, kind)
                with self.assertRaises(ValueError): self.verify()

    def test_elf_mismatch(self):
        self.data = self.data[:18] + (183).to_bytes(2, 'little') + self.data[20:]
        self.make_asset()
        with self.assertRaises(ValueError): self.verify()

    def test_aarch64(self):
        self.data = self.data[:18] + (183).to_bytes(2, 'little') + self.data[20:]
        self.binary.write_bytes(self.data)
        self.asset = self.root / 'all-smi-linux-aarch64.tar.gz'
        self.make_asset()
        smi.ASSETS[self.asset.name] = ('aarch64', 'gnu', self.asset_sha)
        with patch.object(smi.platform, 'machine', return_value='aarch64'):
            self.assertEqual(self.verify()['arch'], 'aarch64')

    def test_host_mismatch(self):
        with patch.object(smi.platform, 'machine', return_value='aarch64'):
            with self.assertRaises(ValueError): self.verify()

    def test_changed_binary_pin(self):
        with self.assertRaises(ValueError): self.verify('0' * 64)

    def test_prepare_and_refuse_overwrite(self):
        output = self.root / 'staged'
        args = ['--asset', str(self.asset), '--checksum', str(self.checksum),
                '--output', str(output), '--evidence', str(self.root / 'prepared')]
        stream = io.StringIO()
        with contextlib.redirect_stdout(stream):
            self.assertEqual(prepare.main(args), 0)
        pins = json.loads(stream.getvalue())
        self.assertEqual(set(pins), {'all_smi_binary', 'all_smi_release_asset', 'all_smi_published_checksum'})
        self.assertEqual(output.read_bytes(), self.data)
        self.assertEqual(output.stat().st_mode & 0o777, 0o755)
        with contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit): prepare.main(args)
        self.assertEqual(output.read_bytes(), self.data)


if __name__ == '__main__':
    unittest.main()
