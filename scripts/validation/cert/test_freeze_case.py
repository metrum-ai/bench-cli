#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch
import freeze_case
import selected_set
from build_role_test_fixture import bind
from execution_evidence import digest


class FreezeCaseTests(unittest.TestCase):
    def test_gpu010_freeze_pins_shipped_build_and_rechecks_provenance(self):
        # Synthetic workflow/API and tokenizer-probe fixtures, not real preflight.
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / 'binary'
            binary.write_text('#!/bin/sh\n# SYNTHETIC SHIPPED BUILD\nexit 0\n')
            binary.chmod(0o755)
            fixture = bind({'candidate_sha': 'a' * 40, 'candidate': {'path': str(binary), 'sha256': digest(binary)}}, root)
            sut = root / 'sut.json'
            sut.write_text(json.dumps({'host_os': 'Linux', 'memory_gb': 128, 'gpu': {'memory_gb': 48}}))
            catalog = root / 'catalog.csv'
            catalog.write_text('ID,Tier,Parallel group\nBC-GPU-010,RC-GPU,G4\n')
            selection = root / 'selection.json'
            selection.write_text(json.dumps({'candidate_sha': 'a' * 40, 'catalog_sha256': digest(catalog), 'selected': {'BC-GPU-010': ['core']}}))
            lane = root / 'lane.json'
            config = {'revision': 'b' * 40, 'tokenizer': {'path': str(binary), 'sha256': digest(binary)}}
            lane.write_text(json.dumps({'candidate_sha': 'a' * 40, 'candidate_binary_sha256': digest(binary),
                                       'baseline_binary_sha256': digest(binary), 'lanes': {'G4': config}}))
            args = SimpleNamespace(build_receipt=Path(fixture['build_receipt']['path']), selection=selection, catalog=catalog,
                case='BC-GPU-010', lane_manifest=lane, candidate=binary, baseline=binary, sut=sut, tokenizer=binary,
                tokenizer_revision='b' * 40, all_smi=binary, input=[], command_json=None,
                out=root / 'frozen.json', run_out=root / 'run')
            with patch.object(freeze_case.manifest_tool, 'load_manifest', return_value=({}, config)), patch('tokenizer_probe.probe'):
                frozen = freeze_case.freeze(args)
                self.assertEqual(frozen['candidate'], fixture['candidate'])
                self.assertNotIn('feature_variant_tokenizer', frozen)
                self.assertIn(fixture['candidate']['path'], {item['path'] for item in frozen['inputs']})
                self.assertEqual(freeze_case.case_launcher.preflight(frozen), ['BC-GPU-010'])
                # Post-freeze tampering of a pinned provenance file still blocks.
                Path(frozen['candidate']['origin']['checksum']['path']).write_text('changed')
                with self.assertRaises(ValueError):
                    freeze_case.case_launcher.preflight(frozen)

    def test_freeze_and_complete_set_fail_closed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "binary"
            binary.write_text("#!/bin/sh\n# Copyright (c) 2026 Metrum AI, Inc.\nexit 0\n")
            binary.chmod(0o755)
            sut = root / "sut.json"
            sut.write_text(json.dumps({"host_os": "Linux", "memory_gb": 128, "gpu": {"memory_gb": 48}}))
            selection = root / "selection.json"
            selection.write_text(json.dumps({"candidate_sha": "a" * 40, "selected": {"BC-GPU-001": ["core"]}}))
            catalog = root / "catalog.csv"
            catalog.write_text("ID,Tier,Parallel group\nBC-GPU-001,RC-GPU,G1\n")
            selection.write_text(json.dumps({"candidate_sha": "a" * 40, "catalog_sha256": digest(catalog),
                                             "selected": {"BC-GPU-001": ["core"]}}))
            lane = root / "lane.json"
            lane.write_text(json.dumps({"candidate_sha": "a" * 40, "candidate_binary_sha256": digest(binary), "baseline_binary_sha256": digest(binary)}))
            fixture = bind({'candidate_sha': 'a' * 40,
                            'candidate': {'path': str(binary), 'sha256': digest(binary)}}, root)
            probe = patch('tokenizer_probe.probe')
            probe.start()
            self.addCleanup(probe.stop)
            args = SimpleNamespace(build_receipt=Path(fixture['build_receipt']['path']), selection=selection, catalog=catalog, case="BC-GPU-001", lane_manifest=lane,
                candidate=binary, baseline=binary, sut=sut, tokenizer=binary, tokenizer_revision="b" * 40,
                all_smi=binary, input=[], command_json=None, out=root / "frozen.json", run_out=root / "run")
            with patch.object(freeze_case.manifest_tool, "load_manifest", return_value=({}, {"revision": "b" * 40})) as validation:
                frozen = freeze_case.freeze(args)
                # Freeze validates once, then launcher preflight independently rechecks.
                self.assertEqual(validation.call_count, 2)
                self.assertIn("--out", frozen["cases"][args.case]["command"])
                self.assertGreater(len(frozen["inputs"]), 10)
                with self.assertRaises(FileExistsError):
                    freeze_case.freeze(args)
                args.tokenizer_revision = "c" * 40
                with self.assertRaisesRegex(ValueError, "tokenizer revision"):
                    freeze_case.freeze(args)
                args.tokenizer_revision = "b" * 40
                custom = root / 'command.json'
                custom.write_text(json.dumps([str(binary)]))
                args.command_json = custom
                with self.assertRaisesRegex(ValueError, 'restricted'):
                    freeze_case.freeze(args)
                args.command_json = None
            index = root / "index.json"
            entries = [{"path": str(args.out), "sha256": digest(args.out)}]
            data = {"candidate_sha": "a" * 40, "selection_sha256": digest(selection), "catalog_sha256": digest(catalog), "manifests": entries}
            index.write_text(json.dumps(data))
            with patch.object(freeze_case.manifest_tool, 'load_manifest', return_value=({}, {'revision': 'b' * 40})):
                self.assertEqual(selected_set.load_set(selection, catalog, index)[0][0], args.case)
            data["manifests"] = []
            index.write_text(json.dumps(data))
            with self.assertRaisesRegex(ValueError, "missing"):
                selected_set.load_set(selection, catalog, index)
            data["manifests"] = entries + entries
            index.write_text(json.dumps(data))
            with patch.object(freeze_case.manifest_tool, 'load_manifest', return_value=({}, {'revision': 'b' * 40})):
                with self.assertRaisesRegex(ValueError, "duplicate"):
                    selected_set.load_set(selection, catalog, index)
            data["manifests"] = entries
            index.write_text(json.dumps(data))
            args.out.write_text("modified")
            with self.assertRaisesRegex(ValueError, "hash mismatch"):
                selected_set.load_set(selection, catalog, index)


if __name__ == "__main__":
    unittest.main()
