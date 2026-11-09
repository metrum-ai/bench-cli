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
import result_writer
from build_role_test_fixture import bind
from execution_evidence import digest


class CaseLauncherTests(unittest.TestCase):
    def setUp(self):
        verifier = patch('harness_integrity.verify', return_value=[])
        verifier.start()
        self.addCleanup(verifier.stop)

    def test_missing_or_changed_candidate_harness_closure_fails_before_launch(self):
        manifest = {'candidate_sha': 'a' * 40}
        with patch.object(case_launcher.subprocess, 'run') as run:
            with self.assertRaisesRegex(ValueError, 'harness closure'):
                case_launcher.preflight(manifest)
            manifest['harness_closure'] = [{'path': '/fake', 'sha256': '0' * 64}]
            with self.assertRaisesRegex(ValueError, 'harness closure'):
                case_launcher.preflight(manifest)
            run.assert_not_called()

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
            binary.write_text('#!/bin/sh\n[ "${CERT_UNTRUSTED_ENV+x}" != x ] || exit 42\nexit 0\n')
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
            smoke = root / 'smoke'; smoke.mkdir()
            (smoke / result_writer.SMOKE_MARKER).write_text(json.dumps(
                {'classification': result_writer.SMOKE_CLASS, 'expectations_sha256': 'd' * 64}))
            smoke_sut = smoke / 'sut.json'; smoke_sut.write_text(sut.read_text())
            contaminated = copy.deepcopy(manifest)
            contaminated['sut'] = pin(smoke_sut)
            with patch.object(case_launcher.subprocess, 'run') as version, patch('tokenizer_probe.probe') as probe:
                with self.assertRaises(result_writer.NonCertificationEvidence):
                    case_launcher.preflight(contaminated)
                version.assert_not_called(); probe.assert_not_called()
            with self.assertRaises(result_writer.NonCertificationEvidence):
                case_launcher.check_input_origins({'deep': [{'nested': pin(smoke_sut)}]}, root / 'cert-out')
            case_launcher.check_input_origins({'sut': pin(sut)}, smoke / 'isolated-out')
            case_launcher.check_input_origins({'sut': pin(smoke_sut)}, smoke / 'isolated-out')
            with patch.object(case_launcher.subprocess, "run") as run, patch('tokenizer_probe.probe'):
                self.assertEqual(case_launcher.preflight(manifest), ["BC-GPU-001"])
                run.assert_called_once()
                self.assertEqual(run.call_args.kwargs['env'], {})
            # A real preflight subprocess must not inherit this sentinel either.
            with patch.dict('os.environ', {'CERT_UNTRUSTED_ENV': 'sentinel'}), patch('tokenizer_probe.probe'):
                self.assertEqual(case_launcher.preflight(manifest), ['BC-GPU-001'])
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
            manifest = {'candidate_sha': 'a' * 40, 'lane': 'G1', 'candidate': pin(binary), 'sut': pin(binary),
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

    def test_record_must_bind_manifest_and_tools(self):
        import copy
        tools = {'python3': {'path': '/fixture/python3', 'sha256': 'b' * 64}}
        record = {'input_hashes': {'execution_manifest': 'a' * 64},
                  'versions': {'execution_tools': tools}}
        case_launcher.validate_execution_record(record, 'a' * 64, tools)
        for field in ('input_hashes', 'versions'):
            for bad in (None, {}, 'invalid'):
                changed = copy.deepcopy(record)
                changed[field] = bad
                with self.subTest(field=field, bad=bad), self.assertRaises(ValueError):
                    case_launcher.validate_execution_record(changed, 'a' * 64, tools)
        with self.assertRaisesRegex(ValueError, 'manifest'):
            case_launcher.validate_execution_record(record, 'c' * 64, tools)
        with self.assertRaisesRegex(ValueError, 'tools'):
            case_launcher.validate_execution_record(record, 'a' * 64, {})

    def test_changed_execution_manifest_cannot_accept_child_pass(self):
        import shlex
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            out = root / 'attempt'
            frozen = root / 'frozen.json'
            launcher = root / 'launcher'
            launcher.write_text('#!/bin/sh\nmkdir -p ' + shlex.quote(str(out / 'results')) +
                '\nprintf "{}" > ' + shlex.quote(str(out / 'results/BC-GPU-001.json')) +
                '\nprintf changed >> ' + shlex.quote(str(frozen)) + '\n')
            launcher.chmod(0o755)
            def pin(path):
                return {'path': str(path), 'sha256': digest(path)}
            manifest = {'candidate_sha': 'a' * 40, 'lane': 'G1', 'candidate': pin(launcher),
                        'sut': pin(launcher), 'all_smi': pin(launcher), 'tokenizer': pin(launcher),
                        'cases': {'BC-GPU-001': {'out': str(out), 'launcher': pin(launcher),
                                                 'command': [str(launcher)]}}}
            bind(manifest, root)
            frozen.write_text(json.dumps(manifest))
            with self.assertRaisesRegex(ValueError, 'executed manifest changed'):
                case_launcher.run_case(frozen, manifest, 'BC-GPU-001')
            self.assertTrue((out / 'results/lane-error.json').is_file())

    def test_real_launch_uses_pinned_tools_not_shadowed_path(self):
        import os
        import shlex
        import subprocess
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            shadow = root / 'shadow'
            shadow.mkdir()
            marker = root / 'shadow-executed'
            fake = shadow / 'python3'
            fake.write_text('#!/bin/sh\necho shadow > ' + shlex.quote(str(marker)) + '\nexit 0\n')
            fake.chmod(0o755)
            out = root / 'attempt'
            child_env = root / 'child-env.json'
            launcher = root / 'launcher'
            program = 'import json,os; print(json.dumps(dict(os.environ)))'
            launcher.write_text('#!/usr/bin/env bash\npython3 -c ' + shlex.quote(program) +
                                ' > ' + shlex.quote(str(child_env)) + '\nexit 7\n')
            launcher.chmod(0o755)
            def pin(path):
                return {'path': str(path), 'sha256': digest(path)}
            manifest = {'candidate_sha': 'a' * 40, 'lane': 'G1', 'candidate': pin(launcher),
                        'sut': pin(launcher), 'all_smi': pin(launcher), 'tokenizer': pin(launcher),
                        'cases': {'BC-GPU-001': {'out': str(out), 'launcher': pin(launcher),
                                                 'command': [str(launcher)]}}}
            bind(manifest, root)
            frozen = root / 'frozen.json'
            frozen.write_text(json.dumps(manifest))
            inherited = {'PATH': str(shadow), 'PYTHONPATH': str(shadow), 'BASH_ENV': str(fake),
                         'LD_PRELOAD': str(fake), 'CERT_UNREVIEWED': 'unsafe', 'HF_TOKEN': 'not-forwarded'}
            with patch.dict(os.environ, inherited), self.assertRaises(subprocess.CalledProcessError):
                case_launcher.run_case(frozen, manifest, 'BC-GPU-001')
            self.assertFalse(marker.exists())
            environment = json.loads(child_env.read_text())
            for name in ('PYTHONPATH', 'BASH_ENV', 'LD_PRELOAD', 'CERT_UNREVIEWED', 'HF_TOKEN'):
                self.assertNotIn(name, environment)
            self.assertNotIn(str(shadow), environment['PATH'])
            record = json.loads((out / 'results/BC-GPU-001.json').read_text())
            self.assertEqual(record['versions']['execution_tools']['python3'], manifest['tools']['python3'])

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
            manifest = {"candidate_sha": "a" * 40, "lane": "G1", "candidate": pin(), "sut": pin(), "all_smi": pin(),
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
