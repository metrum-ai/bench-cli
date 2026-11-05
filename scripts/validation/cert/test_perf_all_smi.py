# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""CPU fixtures for pinned offline all-smi provenance; no exporter runs."""
import hashlib
import io
import os
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import perf_all_smi
import perf_all_smi_prepare


def archive():
    stream = io.BytesIO()
    with tarfile.open(fileobj=stream, mode='w') as tar:
        info = tarfile.TarInfo('Cargo.toml'); info.size = 7
        tar.addfile(info, io.BytesIO(b'fixture'))
    return stream.getvalue()


class AllSmiTests(unittest.TestCase):
    def test_reproduce_uses_pinned_tree_and_offline_secret_free_environment(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); binary = root / 'all-smi'; binary.write_bytes(b'binary'); binary.chmod(0o700)
            protoc = root / 'protoc.crate'
            with tarfile.open(protoc, 'w:gz') as tar:
                info = tarfile.TarInfo('protoc-bin-vendored-linux-x86_64-3.2.0/bin/protoc')
                info.mode = 0o755; info.size = 7; tar.addfile(info, io.BytesIO(b'fixture'))
                info = tarfile.TarInfo('protoc-bin-vendored-linux-x86_64-3.2.0/include')
                info.type = tarfile.DIRTYPE; tar.addfile(info)
            expected = hashlib.sha256(b'binary').hexdigest(); seen = []
            tool_hash = hashlib.sha256(protoc.read_bytes()).hexdigest()
            def run(args, **kwargs):
                result = unittest.mock.Mock(returncode=0, stderr=b'')
                if args[0] == 'git' and 'rev-parse' in args: result.stdout = perf_all_smi.SOURCE_REVISION + '\n'
                elif args[0] == 'git' and 'show' in args: result.stdout = '1700000000\n'
                elif args[0] == 'git': result.stdout = archive()
                elif args[0] in ('rustc', 'cargo') and '--version' in args: result.stdout = 'fixture compiler\n'
                else: raise AssertionError('unexpected command')
                return result
            def build(args, _cwd, env, _evidence):
                seen.append(env)
                out = Path(args[args.index('--target-dir') + 1]) / 'release/all-smi'; out.parent.mkdir(parents=True)
                out.write_bytes(b'binary')
                return 0
            with patch.dict(os.environ, {'GH_TOKEN': 'fixture-only', 'HF_TOKEN': 'fixture-only'}), \
                 patch.object(perf_all_smi.subprocess, 'run', side_effect=run), \
                 patch.object(perf_all_smi.platform, 'machine', return_value='x86_64'), \
                 patch.dict(perf_all_smi.PROTOC_CRATES, {'x86_64': ('linux-x86_64', tool_hash)}), \
                 patch.object(perf_all_smi, 'build', side_effect=build):
                value = perf_all_smi.reproduce(root, binary, expected, protoc, root)
            self.assertEqual(value['source_revision'], perf_all_smi.SOURCE_REVISION)
            self.assertEqual(seen[0]['CARGO_NET_OFFLINE'], 'true')
            self.assertEqual(seen[0]['SOURCE_DATE_EPOCH'], '1700000000')
            self.assertIn('--remap-path-prefix=', seen[0]['CARGO_ENCODED_RUSTFLAGS'])
            self.assertIn('-ffile-prefix-map=', seen[0]['CFLAGS'])
            self.assertNotIn('GH_TOKEN', seen[0]); self.assertNotIn('HF_TOKEN', seen[0])

    def test_prepare_requires_a_second_identical_build(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            argv = ['prepare', '--checkout', str(root), '--protoc-crate', str(root / 'crate'),
                    '--output', str(root / 'binary'), '--evidence', str(root / 'evidence')]
            with patch('sys.argv', argv), patch('sys.stderr', new_callable=io.StringIO), \
                 patch.object(perf_all_smi_prepare.perf_all_smi, 'reproduce',
                    side_effect=[{'binary_sha256': 'one'}, ValueError('mismatch')]):
                self.assertEqual(perf_all_smi_prepare.main(), 1)
            self.assertFalse((root / 'evidence/verified-provenance.json').exists())

    def test_timed_out_build_signals_its_process_group(self):
        with tempfile.TemporaryDirectory() as tmp:
            process = unittest.mock.Mock(pid=1234)
            process.communicate.side_effect = [perf_all_smi.subprocess.TimeoutExpired('fixture', 1800), (b'', b'')]
            process.poll.return_value = 0
            with patch.object(perf_all_smi.subprocess, 'Popen', return_value=process), \
                 patch.object(perf_all_smi.os, 'killpg') as kill:
                code = perf_all_smi.build(['fixture'], Path(tmp), {}, Path(tmp))
            self.assertEqual(code, 124)
            kill.assert_called_once_with(1234, perf_all_smi.signal.SIGTERM)

    def test_wrong_binary_hash_fails_before_processes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); binary = root / 'binary'; binary.write_bytes(b'not-pinned'); binary.chmod(0o700)
            with patch.object(perf_all_smi.subprocess, 'run') as run:
                with self.assertRaises(ValueError): perf_all_smi.reproduce(root, binary, '0' * 64, root, root)
                run.assert_not_called()


if __name__ == '__main__': unittest.main()
