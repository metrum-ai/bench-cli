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
from execution_evidence import digest


class FreezeCaseTests(unittest.TestCase):
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
            lane = root / "lane.json"
            lane.write_text(json.dumps({"candidate_sha": "a" * 40, "candidate_binary_sha256": digest(binary), "baseline_binary_sha256": digest(binary)}))
            args = SimpleNamespace(selection=selection, catalog=catalog, case="BC-GPU-001", lane_manifest=lane,
                candidate=binary, baseline=binary, sut=sut, tokenizer=binary, tokenizer_revision="b" * 40,
                all_smi=binary, input=[], command_json=None, out=root / "frozen.json", run_out=root / "run")
            with patch.object(freeze_case.manifest_tool, "load_manifest", return_value=({}, {"revision": "b" * 40})) as validation:
                frozen = freeze_case.freeze(args)
                validation.assert_called_once()
                self.assertIn("--out", frozen["cases"][args.case]["command"])
                self.assertGreater(len(frozen["inputs"]), 10)
                with self.assertRaises(FileExistsError):
                    freeze_case.freeze(args)
                args.tokenizer_revision = "c" * 40
                with self.assertRaisesRegex(ValueError, "tokenizer revision"):
                    freeze_case.freeze(args)
                args.tokenizer_revision = "b" * 40
            index = root / "index.json"
            entries = [{"path": str(args.out), "sha256": digest(args.out)}]
            data = {"candidate_sha": "a" * 40, "selection_sha256": digest(selection), "catalog_sha256": digest(catalog), "manifests": entries}
            index.write_text(json.dumps(data))
            self.assertEqual(selected_set.load_set(selection, catalog, index)[0][0], args.case)
            data["manifests"] = []
            index.write_text(json.dumps(data))
            with self.assertRaisesRegex(ValueError, "missing"):
                selected_set.load_set(selection, catalog, index)
            data["manifests"] = entries + entries
            index.write_text(json.dumps(data))
            with self.assertRaisesRegex(ValueError, "duplicate"):
                selected_set.load_set(selection, catalog, index)
            data["manifests"] = entries
            index.write_text(json.dumps(data))
            args.out.write_text("modified")
            with self.assertRaisesRegex(ValueError, "hash mismatch"):
                selected_set.load_set(selection, catalog, index)


if __name__ == "__main__":
    unittest.main()
