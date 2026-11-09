#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
import copy
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
import case_launcher
from build_role_test_fixture import bind
from execution_evidence import digest


class CaseLauncherTests(unittest.TestCase):
    def test_direct_launcher_rejects_duplicate_manifest_keys_before_preflight(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'case.json'
            path.write_text('{"candidate_sha":"a","candidate_sha":"b"}')
            with patch('sys.argv', ['case_launcher.py', '--manifest', str(path)]), \
                 patch.object(case_launcher, 'preflight') as preflight:
                with self.assertRaisesRegex(ValueError, 'duplicate'):
                    case_launcher.main()
                preflight.assert_not_called()

    def test_pins_and_preflight_fields(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "binary"
            binary.write_text("#!/bin/sh\nexit 0\n")
            binary.chmod(0o755)
            sut = root / "sut.json"
            sut.write_text(json.dumps({"host_os": "Linux", "memory_gb": 128, "gpu": {"memory_gb": 48}}))
            def pin(path):
                return {"path": str(path), "sha256": digest(path)}
            lane_manifest = root / 'lane.json'
            lane_manifest.write_text('{}')
            loader = patch('manifest_tool.load_manifest', return_value=({}, {'revision': 'b' * 40}))
            loader.start()
            self.addCleanup(loader.stop)
            manifest = {"candidate_sha": "a" * 40, "candidate": pin(binary), "baseline": pin(binary),
                        "lane": "G1", "lane_manifest": pin(lane_manifest), "sut": pin(sut),
                        "all_smi": pin(binary), "tokenizer": {**pin(binary), "revision": "b" * 40},
                        "cases": {"BC-GPU-001": {"launcher": pin(binary), "out": str(root / "attempt"),
                                                    "command": [str(binary), "--out", str(root / "attempt")]}}}
            bind(manifest, root)
            with patch.object(case_launcher.subprocess, "run") as run, patch('tokenizer_probe.probe'):
                self.assertEqual(case_launcher.preflight(manifest), ["BC-GPU-001"])
                run.assert_called_once()
            guarded = copy.deepcopy(manifest)
            guarded["cases"] = {"BC-GPU-010": manifest["cases"]["BC-GPU-001"]}
            with patch.object(case_launcher.subprocess, "run"), patch("tokenizer_probe.probe", side_effect=ValueError("missing tokenizer capability")) as probe:
                with self.assertRaisesRegex(ValueError, "missing tokenizer capability"):
                    case_launcher.preflight(guarded)
                probe.assert_called_once_with(Path(manifest['tokenizer_validation']['path']).resolve(), str(binary))
            with patch.object(case_launcher.subprocess, 'run'), patch('tokenizer_probe.probe') as probe:
                with self.assertRaisesRegex(ValueError, 'frozen input pins'):
                    case_launcher.preflight(guarded)
                from gpu010_builds import ORIGIN_PINS
                guarded['inputs'] = [guarded['build_receipt'], pin(binary)] + [guarded['candidate']['origin'][key] for key in ORIGIN_PINS]
                probe.reset_mock()
                self.assertEqual(case_launcher.preflight(guarded), ['BC-GPU-010'])
                self.assertEqual(probe.call_count, 2)
                probe.assert_any_call(binary.resolve(), str(binary))
            sut.write_text('{"host_os":"Linux","host_os":"Other","memory_gb":128,"gpu":{"memory_gb":48}}')
            invalid = copy.deepcopy(manifest)
            invalid['sut'] = pin(sut)
            with patch.object(case_launcher.subprocess, 'run'), patch('tokenizer_probe.probe'):
                with self.assertRaisesRegex(ValueError, 'duplicate'):
                    case_launcher.preflight(invalid)
            for field in ("host_os", "memory_gb"):
                bad = {"host_os": "Linux", "memory_gb": 128, "gpu": {"memory_gb": 48}}
                bad.pop(field)
                sut.write_text(json.dumps(bad))
                invalid = copy.deepcopy(manifest)
                invalid["sut"] = pin(sut)
                with self.assertRaisesRegex(ValueError, "SUT"):
                    case_launcher.preflight(invalid)
            with self.assertRaisesRegex(ValueError, "hash mismatch"):
                case_launcher.preflight(manifest)
            sut.write_text(json.dumps({"host_os": "Linux", "memory_gb": 128, "gpu": {"memory_gb": 48}}))
            for field in ("all_smi", "tokenizer", "candidate"):
                invalid = copy.deepcopy(manifest)
                invalid[field]["sha256"] = "0" * 64
                with self.assertRaisesRegex(ValueError, "hash mismatch"):
                    case_launcher.preflight(invalid)
            invalid = copy.deepcopy(manifest)
            invalid["tokenizer"]["revision"] = "main"
            with self.assertRaisesRegex(ValueError, "revision"):
                case_launcher.preflight(invalid)


    def test_r4_result_identity_is_mandatory(self):
        import r4_model_pins as pins
        import r4_image_provenance as provenance
        with tempfile.TemporaryDirectory() as temporary:
            lane = Path(temporary) / 'lane.json'
            config = {'image': pins.IMAGE, 'model': pins.GENERAL_MODEL, 'revision': pins.GENERAL_REVISION}
            lane.write_text(json.dumps({'lanes': {'G1': config}}))
            manifest = {'lane': 'G1', 'lane_manifest': {'path': str(lane), 'sha256': digest(lane)}}
            versions = {'server_image': pins.IMAGE, 'model': pins.GENERAL_MODEL, 'model_revision': pins.GENERAL_REVISION,
                        'image_index': provenance.INDEX, 'platform_manifest': provenance.AMD64,
                        'image_config': provenance.CONFIG, 'image_platform': 'linux/amd64'}
            case_launcher.validate_r4_record(manifest, 'BC-GPU-001', {'versions': versions})
            for key in versions:
                bad = dict(versions)
                del bad[key]
                with self.subTest(key=key), self.assertRaisesRegex(ValueError, 'serving provenance'):
                    case_launcher.validate_r4_record(manifest, 'BC-GPU-001', {'versions': bad})

    def test_successful_child_with_mutated_binary_cannot_validate_pass(self):
        import shlex
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / 'candidate'
            binary.write_text('#!/bin/sh\nexit 0\n')
            binary.chmod(0o755)
            out = root / 'attempt'
            launcher = root / 'launcher'
            record = json.dumps({'ID': 'BC-GPU-001', 'candidate_sha': 'a' * 40, 'result': 'PASS'})
            launcher.write_text('#!/bin/sh\nmkdir -p ' + shlex.quote(str(out / 'results')) + '\nprintf %s ' +
                shlex.quote(record) + ' > ' + shlex.quote(str(out / 'results/BC-GPU-001.json')) +
                '\nprintf changed >> ' + shlex.quote(str(binary)) + '\n')
            launcher.chmod(0o755)
            def pin(path):
                return {'path': str(path), 'sha256': digest(path)}
            manifest = {'candidate_sha': 'a' * 40, 'candidate': pin(binary), 'sut': pin(binary),
                        'all_smi': pin(binary), 'tokenizer': pin(binary),
                        'cases': {'BC-GPU-001': {'out': str(out), 'launcher': pin(launcher),
                                  'command': [str(launcher), '--out', str(out)]}}}
            bind(manifest, root)
            frozen = root / 'frozen.json'
            frozen.write_text(json.dumps(manifest))
            with self.assertRaisesRegex(ValueError, 'hash mismatch'):
                case_launcher.run_case(frozen, manifest, 'BC-GPU-001')
            self.assertTrue((out / 'results/lane-error.json').is_file())
            from execution_evidence import report_rows
            with self.assertRaisesRegex(ValueError, 'lane error prevents acceptance'):
                report_rows(out)
            phases = (root / 'attempt-lifecycle/phases.jsonl').read_text().splitlines()
            self.assertNotIn('record-validated', [json.loads(line)['phase'] for line in phases])

    def test_failed_launch_writes_typed_record(self):
        import subprocess
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            launcher = root / "launcher"
            launcher.write_text("#!/bin/sh\nexit 143\n")
            launcher.chmod(0o755)
            def pin():
                return {"path": str(launcher), "sha256": digest(launcher)}
            out = root / "attempt"
            manifest = {"candidate_sha": "a" * 40, "candidate": pin(), "sut": pin(), "all_smi": pin(),
                        "tokenizer": pin(), "cases": {"BC-GPU-001": {"out": str(out), "launcher": pin(),
                        "command": [str(launcher), "--out", str(out)]}}}
            bind(manifest, root)
            frozen = root / "frozen.json"
            frozen.write_text(json.dumps(manifest))
            with self.assertRaises(subprocess.CalledProcessError):
                case_launcher.run_case(frozen, manifest, "BC-GPU-001")
            record = json.loads((out / "results/BC-GPU-001.json").read_text())
            self.assertEqual(record["result"], "FAIL")
            self.assertIsNone(record["observed_values"]["request_count"])
            self.assertEqual(record["infra_history"][0]["type"], "infra_interrupted")
            self.assertEqual(record["input_hashes"]["execution_manifest"], digest(frozen))
            with self.assertRaises(FileExistsError):
                case_launcher.run_case(frozen, manifest, "BC-GPU-001")


if __name__ == "__main__":
    unittest.main()
