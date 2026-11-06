#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Fail-closed pre-tag evidence inventory; does not execute GPU tests."""
import argparse
import csv
import datetime as dt
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

REQUIRED_TIERS = {"PR", "RC", "RC-GPU"}
ID = re.compile(r"BC-[A-Z]+-[0-9]{3}\Z")
SHA = re.compile(r"[0-9a-f]{40}\Z")
SHA256 = re.compile(r"[0-9a-f]{64}\Z")
MUST_RERUN = {"BC-CI-001", "BC-CI-006", "BC-CI-007", "BC-PACK-001",
              "BC-PACK-002", "BC-REL-001", "BC-REL-002"}
REPO = Path(__file__).resolve().parents[3]


def unique_object(pairs):
    out = {}
    for key, value in pairs:
        if key in out:
            raise ValueError(f"duplicate JSON key: {key}")
        out[key] = value
    return out


def timestamp(value):
    if not isinstance(value, str) or not value.endswith("Z"):
        raise ValueError("timestamp must be UTC ISO-8601 ending in Z")
    parsed = dt.datetime.fromisoformat(value[:-1] + "+00:00")
    return parsed


def load_catalog(path):
    with path.open(newline="", encoding="utf-8") as stream:
        reader = csv.DictReader(stream)
        if not {"ID", "Tier", "Test case"} <= set(reader.fieldnames or []):
            raise ValueError("catalog missing required columns")
        rows = {}
        for row in reader:
            case = row["ID"]
            if not ID.fullmatch(case) or case in rows:
                raise ValueError(f"invalid or duplicate catalog ID: {case}")
            if row["Tier"] not in REQUIRED_TIERS | {"POST", "PERIODIC"}:
                raise ValueError(f"invalid tier: {case}")
            rows[case] = row
        return rows


def check_evidence(base, paths):
    if not isinstance(paths, list) or not paths:
        raise ValueError("evidence_paths must be a nonempty list")
    for item in paths:
        if not isinstance(item, str) or not item:
            raise ValueError("invalid evidence path")
        p = (base / item).resolve()
        if not p.is_relative_to(base.resolve()) or not p.is_file():
            raise ValueError(f"missing or outside evidence directory: {item}")


def check_record(record, candidate, root, as_of):
    if record.get("candidate_sha") != candidate:
        raise ValueError("candidate SHA mismatch")
    for key in ("owner", "evidence_link"):
        if not isinstance(record.get(key), str) or not record[key].strip():
            raise ValueError(f"missing {key}")
    result = record.get("result")
    if result == "PASS":
        if not isinstance(record.get("observed_values"), dict) or not record["observed_values"]:
            raise ValueError("missing observed values")
        if not isinstance(record.get("thresholds"), dict) or not record["thresholds"]:
            raise ValueError("missing thresholds")
        check_evidence(root, record.get("evidence_paths"))
        if timestamp(record.get("end_utc")) < timestamp(record.get("start_utc")):
            raise ValueError("end precedes start")
        if timestamp(record["end_utc"]) > as_of:
            raise ValueError("result is in the future")
        if not isinstance(record.get("attempt"), int) or isinstance(record["attempt"], bool) or record["attempt"] < 1:
            raise ValueError("invalid attempt")
    elif result == "N-A":
        approval = record.get("approval")
        if not isinstance(approval, dict):
            raise ValueError("missing N/A approval")
        for key in ("reason", "requester", "approver", "scope"):
            if not isinstance(approval.get(key), str) or not approval[key].strip():
                raise ValueError(f"missing approval {key}")
        if approval["requester"].strip().casefold() == approval["approver"].strip().casefold():
            raise ValueError("approver must be independent")
        approved = timestamp(approval.get("approved_utc"))
        expires = timestamp(approval.get("expires_utc"))
        freeze = timestamp(record.get("case_start_utc"))
        if not (approved <= freeze <= as_of <= expires):
            raise ValueError("approval must precede case and be unexpired")
        check_evidence(root, record.get("evidence_paths"))
    else:
        raise ValueError("result is not PASS or approved N-A")
    return result


def load_selection(path, catalog, rows, candidate):
    # Recompute the whole selection rather than trusting an edited selected list.
    import select_cases
    with path.open(encoding="utf-8") as stream:
        selection = json.load(stream, object_pairs_hook=unique_object,
                              parse_constant=lambda x: (_ for _ in ()).throw(ValueError(f"invalid JSON number: {x}")))
    if not isinstance(selection, dict):
        raise ValueError("selection must be an object")
    try:
        expected = select_cases.make_selection(rows, select_cases.catalog_hash(catalog), candidate,
                                               selection["sample_rate"], selection["rotation_releases"],
                                               selection["changed_files"], selection["history_newest_first"], selection["seed"])
    except (KeyError, TypeError) as exc:
        raise ValueError(f"invalid selection: {exc}") from exc
    if selection != expected:
        raise ValueError("selection does not match candidate, catalog, inputs and deterministic policy")
    return selection


class CarryOver:
    """Authorize only hash-bound, reviewed source records from actual ancestors."""

    def __init__(self, manifest, root, candidate, as_of):
        self.root = root.resolve()
        self.candidate = candidate
        self.as_of = as_of
        self.entries = {}
        self.used = set()
        with manifest.open(encoding="utf-8") as stream:
            data = json.load(stream, object_pairs_hook=unique_object,
                             parse_constant=lambda x: (_ for _ in ()).throw(ValueError(f"invalid JSON number: {x}")))
        if not isinstance(data, dict) or data.get("target_candidate_sha") != candidate:
            raise ValueError("carry-over target SHA mismatch")
        entries = data.get("entries")
        if not isinstance(entries, list) or not entries:
            raise ValueError("carry-over entries must be a nonempty list")
        for entry in entries:
            if not isinstance(entry, dict):
                raise ValueError("invalid carry-over entry")
            case, attempt = entry.get("ID"), entry.get("attempt")
            if (not isinstance(case, str) or not ID.fullmatch(case) or not isinstance(attempt, int)
                    or isinstance(attempt, bool) or attempt < 1):
                raise ValueError("invalid carry-over case or attempt")
            key = (case, attempt)
            if key in self.entries:
                raise ValueError("duplicate carry-over case/attempt")
            self.entries[key] = entry

    def within(self, value, directory=False):
        if not isinstance(value, str) or not value or Path(value).is_absolute():
            raise ValueError("invalid carry-over path")
        path = (self.root / value).resolve()
        if not path.is_relative_to(self.root) or not (path.is_dir() if directory else path.is_file()):
            raise ValueError("missing or outside carry-over path")
        return path

    def authorize(self, record, record_path):
        case, attempt = record.get("ID"), record.get("attempt")
        if case in MUST_RERUN:
            raise ValueError("case requires a target-candidate rerun")
        entry = self.entries.get((case, attempt))
        if entry is None:
            raise ValueError("missing carry-over case/attempt")
        source = entry.get("source_sha")
        if not isinstance(source, str) or not SHA.fullmatch(source) or source == self.candidate or source != record.get("candidate_sha"):
            raise ValueError("carry-over source SHA mismatch")
        record_hash = entry.get("record_sha256")
        raw = record_path.read_bytes()
        if (not isinstance(record_hash, str) or not SHA256.fullmatch(record_hash)
                or hashlib.sha256(raw).hexdigest() != record_hash
                or json.loads(raw, object_pairs_hook=unique_object) != record):
            raise ValueError("carry-over original record hash mismatch")
        if subprocess.run(["git", "-C", str(REPO), "merge-base", "--is-ancestor", source, self.candidate],
                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False).returncode != 0:
            raise ValueError("carry-over source is not an ancestor")
        disposition = entry.get("disposition")
        if not isinstance(disposition, dict) or disposition.get("applicable") is not True:
            raise ValueError("carry-over applicability not approved")
        for field in ("reviewer", "reason"):
            if not isinstance(disposition.get(field), str) or not disposition[field].strip():
                raise ValueError(f"missing carry-over {field}")
        if timestamp(disposition.get("utc")) > self.as_of:
            raise ValueError("carry-over review is in the future")
        proofs = entry.get("proofs")
        if not isinstance(proofs, list) or not proofs:
            raise ValueError("carry-over proofs must be nonempty")
        for proof in proofs:
            if not isinstance(proof, dict):
                raise ValueError("invalid carry-over proof")
            digest = proof.get("sha256")
            if not isinstance(digest, str) or not SHA256.fullmatch(digest):
                raise ValueError("invalid carry-over proof hash")
            if hashlib.sha256(self.within(proof.get("path")).read_bytes()).hexdigest() != digest:
                raise ValueError("carry-over proof hash mismatch")
        evidence_root = self.within(entry.get("evidence_root"), directory=True)
        self.used.add((case, attempt))
        return evidence_root

    def unused_errors(self):
        return [f"unused carry-over entry {case} attempt {attempt}"
                for case, attempt in sorted(self.entries.keys() - self.used)]


def validate(catalog, results, candidate, as_of, cadence=None, history=None, selection=None, include_post=False,
             carry_over_manifest=None):
    rows = load_catalog(catalog)
    universe = {case for case, row in rows.items()
                if (cadence in row.get("Cadence", "").split(";") if cadence
                    else row["Tier"] in REQUIRED_TIERS | ({"POST"} if include_post else set()))}
    expected = universe & set(selection["selected"]) if selection is not None else universe
    if not expected:
        raise ValueError("no required cases in catalog")
    records = {}
    record_paths = {}
    errors = []
    carry = (CarryOver(carry_over_manifest, results.parent, candidate, as_of)
             if carry_over_manifest is not None else None)
    for path in sorted(results.glob("*.json")):
        try:
            with path.open(encoding="utf-8") as stream:
                record = json.load(stream, object_pairs_hook=unique_object, parse_constant=lambda x: (_ for _ in ()).throw(ValueError(f"invalid JSON number: {x}")))
            case = record.get("ID")
            if case not in expected or path.name != f"{case}.json":
                raise ValueError("unexpected ID or filename")
            if case in records:
                raise ValueError("duplicate ID")
            records[case] = record
            record_paths[case] = path
        except (ValueError, OSError, AttributeError) as exc:
            errors.append(f"{path.name}: {exc}")
    histories = {case: [] for case in expected}
    if history is not None:
        if not history.is_dir():
            errors.append("history directory does not exist")
        else:
            for path in sorted(history.rglob("*.json")):
                try:
                    with path.open(encoding="utf-8") as stream:
                        previous = json.load(stream, object_pairs_hook=unique_object,
                                             parse_constant=lambda x: (_ for _ in ()).throw(ValueError(f"invalid JSON number: {x}")))
                    case = previous.get("ID")
                    attempt = previous.get("attempt")
                    if (case not in expected or not isinstance(attempt, int) or isinstance(attempt, bool) or attempt < 1 or
                        previous.get("result") not in ("PASS", "FAIL", "N-A")):
                        raise ValueError("invalid history ID, SHA, result or attempt")
                    if previous.get("candidate_sha") != candidate:
                        if carry is None:
                            raise ValueError("invalid history ID, SHA, result or attempt")
                        carry.authorize(previous, path)
                    histories[case].append(previous)
                except (ValueError, OSError, AttributeError) as exc:
                    errors.append(f"history {path.name}: {exc}")
    table = []
    for case in sorted(universe):
        if case not in expected:
            table.append({"ID": case, "Test case": rows[case]["Test case"], "Tier": rows[case]["Tier"],
                          "Result": "NOT-SELECTED", "Notes": "outside frozen selection", "Catalog": rows[case],
                          "Record": {}, "History": []})
            continue
        record = records.get(case)
        try:
            if record is None:
                raise ValueError("missing result")
            source = record.get("candidate_sha")
            if source != candidate and carry is not None:
                evidence_root = carry.authorize(record, record_paths[case])
                result = check_record(record, source, evidence_root, as_of)
            else:
                result = check_record(record, candidate, results.parent, as_of)
            attempt = record.get("attempt", 1)
            old_attempts = [entry["attempt"] for entry in histories[case]]
            if sorted(old_attempts) != list(range(1, attempt)):
                raise ValueError("missing, duplicate or unexpected historical attempts")
            if result == "N-A" and any(entry["result"] == "FAIL" for entry in histories[case]):
                raise ValueError("a failed case cannot be reclassified N-A")
            note = (f"carried from {source} to {candidate}" if source != candidate else "")
        except (ValueError, TypeError, KeyError) as exc:
            result = "FAIL"
            note = str(exc)
            errors.append(f"{case}: {note}")
        table.append({"ID": case, "Test case": rows[case]["Test case"], "Tier": rows[case]["Tier"],
                      "Result": result, "Notes": note, "Catalog": rows[case],
                      "Record": record or {}, "History": sorted(histories[case], key=lambda x: x["attempt"])})
    if carry is not None:
        errors.extend(carry.unused_errors())
    return table, errors


RESULT_FIELDS = ["Release", "Candidate SHA", "ID", "Test case", "Tier", "Result", "Attempt",
                 "Evidence link", "Run date UTC", "Owner", "Notes"]
PERIODIC_FIELDS = ["Run ID", "Cadence", "Run date (UTC)", "Branch / SHA", "Compared against (release)",
                   "Test ID", "Result", "Observed vs threshold", "GPU class / runner", "Evidence link",
                   "Regression issue (if FAIL)", "Notes"]


def text(value):
    if isinstance(value, (dict, list)):
        value = json.dumps(value, sort_keys=True, ensure_ascii=False)
    return str(value if value is not None else "")


def md(value):
    return text(value).replace("|", "\\|").replace("\n", " ")


def export_csv(path, table, args):
    periodic = args.cadence is not None
    fields = PERIODIC_FIELDS if periodic else RESULT_FIELDS
    with path.open("w", newline="", encoding="utf-8") as stream:
        writer = csv.DictWriter(stream, fieldnames=fields)
        writer.writeheader()
        for row in table:
            rec = row["Record"]
            note = "; ".join(filter(None, (row["Notes"], text(rec.get("notes", "")))))
            if periodic:
                output = {"Run ID": args.run_id, "Cadence": args.cadence, "Run date (UTC)": args.run_date_utc,
                          "Branch / SHA": f"{args.branch} / {args.candidate_sha}",
                          "Compared against (release)": args.compared_against, "Test ID": row["ID"],
                          "Result": row["Result"],
                          "Observed vs threshold": text({"observed": rec.get("observed_values"), "thresholds": rec.get("thresholds")}),
                          "GPU class / runner": rec.get("gpu_class") or rec.get("runner", ""),
                          "Evidence link": rec.get("evidence_link", ""),
                          "Regression issue (if FAIL)": rec.get("regression_issue", "") if row["Result"] == "FAIL" else "",
                          "Notes": note}
            else:
                output = {"Release": args.release, "Candidate SHA": args.candidate_sha, "ID": row["ID"],
                          "Test case": row["Test case"], "Tier": row["Tier"], "Result": row["Result"],
                          "Attempt": rec.get("attempt", ""), "Evidence link": rec.get("evidence_link", ""),
                          "Run date UTC": rec.get("start_utc") or rec.get("case_start_utc", ""),
                          "Owner": rec.get("owner", ""), "Notes": note}
            writer.writerow(output)


def export_report(path, table, errors, args):
    counts = {key: sum(row["Result"] == key for row in table) for key in ("PASS", "FAIL", "N-A", "NOT-SELECTED")}
    lanes = {}
    for row in table:
        lane = row["Catalog"].get("Parallel group", "unassigned")
        group = lanes.setdefault(lane, {"PASS": 0, "FAIL": 0, "N-A": 0, "NOT-SELECTED": 0})
        group[row["Result"]] += 1
    with path.open("w", encoding="utf-8") as stream:
        stream.write(f"# Certification report: {md(args.release)}\n\n")
        stream.write(f"Candidate: `{md(args.candidate_sha)}`  \nVerdict: **{'FAIL' if errors else 'PASS'}**  \n")
        stream.write(f"Counts: {counts['PASS']} PASS, {counts['FAIL']} FAIL, {counts['N-A']} N-A, {counts['NOT-SELECTED']} NOT-SELECTED.\n\n")
        if not args.cadence:
            stream.write(f"Qualification stage: {md(args.stage)}. POST is not a pre-tag result.\n\n")
        if args.cadence:
            stream.write(f"Periodic run: {md(args.run_id)}; cadence {md(args.cadence)}; branch {md(args.branch)}; "
                         f"compared against {md(args.compared_against)}; UTC {md(args.run_date_utc)}.\n\n")
        stream.write("## Lane summary\n\n| Lane | PASS | FAIL | N-A | NOT-SELECTED |\n| --- | ---: | ---: | ---: | ---: |\n")
        for lane, totals in sorted(lanes.items()):
            stream.write(f"| {md(lane)} | {totals['PASS']} | {totals['FAIL']} | {totals['N-A']} | {totals['NOT-SELECTED']} |\n")
        stream.write("\n## Cases and attempt history\n")
        for row in table:
            rec, case = row["Record"], row["Catalog"]
            stream.write(f"\n### {md(row['ID'])} — {md(row['Test case'])}: {row['Result']}\n\n")
            for label, value in (("Scenario", case.get("Command or procedure", "")),
                                 ("Expected", case.get("Pass criteria", "")),
                                 ("Observed", rec.get("observed_values", "not provided")),
                                 ("Thresholds", rec.get("thresholds", "not provided")),
                                 ("Owner / evidence", f"{rec.get('owner', '')} / {rec.get('evidence_link', '')}"),
                                 ("Environment", {"gpu_class": rec.get("gpu_class"), "driver": rec.get("driver"), "versions": rec.get("versions")}),
                                 ("Deviations / approval", rec.get("approval") or rec.get("notes") or "none recorded"),
                                 ("Validation failure", row["Notes"] or "none")):
                stream.write(f"- **{label}:** {md(value)}\n")
            attempts = row["History"] + ([rec] if rec else [])
            if not attempts:
                stream.write("- **Attempts:** none provided\n")
            for attempt in attempts:
                stream.write(f"- **Attempt {md(attempt.get('attempt', '?'))}:** {md(attempt.get('result', 'unknown'))}; "
                             f"{md(attempt.get('start_utc', ''))}; reason {md(attempt.get('reason', ''))}; "
                             f"observed {md(attempt.get('observed_values', {}))}; "
                             f"thresholds {md(attempt.get('thresholds', {}))}; evidence {md(attempt.get('evidence_link', ''))}\n")
        if errors:
            stream.write("\n## Validation failures\n\n")
            for error in errors:
                stream.write(f"- {md(error)}\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--catalog", type=Path, required=True)
    parser.add_argument("--results", type=Path, required=True)
    parser.add_argument("--candidate-sha", required=True)
    parser.add_argument("--release", required=True)
    parser.add_argument("--history", type=Path)
    parser.add_argument("--carry-over-manifest", type=Path, help="reviewed opt-in carry-over manifest")
    parser.add_argument("--selection", type=Path, help="frozen selection.json (required for release)")
    parser.add_argument("--stage", choices=["pre-tag", "full"], default="pre-tag")
    parser.add_argument("--markdown", type=Path, default=Path("CERTIFICATION-REPORT.md"))
    parser.add_argument("--csv", type=Path)
    parser.add_argument("--cadence", choices=["nightly", "weekly", "on-main-change", "on-upstream-change"])
    parser.add_argument("--run-id")
    parser.add_argument("--run-date-utc")
    parser.add_argument("--branch")
    parser.add_argument("--compared-against")
    args = parser.parse_args()
    if not SHA.fullmatch(args.candidate_sha) or not args.results.is_dir():
        parser.error("candidate must be a full lowercase SHA and results must be a directory")
    if (not args.cadence and not args.selection) or (args.cadence and args.selection):
        parser.error("release mode requires --selection; periodic mode does not use it")
    if args.cadence and args.stage != "pre-tag":
        parser.error("--stage applies only to release mode")
    if args.cadence and args.carry_over_manifest:
        parser.error("carry-over is only supported in release mode")
    if args.cadence and not all((args.run_id, args.run_date_utc, args.branch, args.compared_against)):
        parser.error("periodic mode requires run ID, UTC date, branch and comparison release")
    try:
        if args.cadence:
            timestamp(args.run_date_utc)
        selection = load_selection(args.selection, args.catalog, load_catalog(args.catalog), args.candidate_sha) if args.selection else None
        table, errors = validate(args.catalog, args.results, args.candidate_sha,
                                 dt.datetime.now(dt.timezone.utc), args.cadence, args.history,
                                 selection, args.stage == "full", args.carry_over_manifest)
        export_csv(args.csv or Path("periodic.csv" if args.cadence else "results.csv"), table, args)
        export_report(args.markdown, table, errors, args)
        for error in errors:
            print(error, file=sys.stderr)
        return 1 if errors else 0
    except (OSError, ValueError, csv.Error) as exc:
        print(f"certification validation failed: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
