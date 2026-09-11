#!/usr/bin/env python3
"""Strict, offline acceptance of one pinned preset-ERD capture; never rewrite raw verdicts."""

import argparse
import hashlib
import json
from pathlib import Path
import re

from comparison import (compare_case, output_after_load, same_json_value,
                        split_setup_diagnostics, validate_rust_evidence)
from recompare import recorded_steps
from run import identity as fixture_identity

BASELINES = {
    "original": "7b69ebd27378c03c32b6477b74901bfc3d33223c",
    "snake": "57170459b3d5ca175a1c57933058b569088bee0e",
}
CASE_NAMES = {"main", "disabled", "named-character", "name-conflict", "price-conflict",
              "duplicate-names", "unicode-order"}
# Handwritten expectations from the frozen fixture rows and 5717045 ConstantData.cs.
# This is an observation contract, not an implementation of the production merger.
WARNINGS = {
    "name-conflict": [
        ("preset_name_conflict", "erb/a/ABL.erd", 1, 0, "csv_abl", "ignored_csv_conflict"),
        ("preset_name_conflict", "erb/b/ABL.erd", 1, 1, "first_erd", "later_erd"),
    ],
    "price-conflict": [
        ("preset_price_conflict_csv", "erb/a/ITEMPRICE.erd", 1, 0, 0, 4),
    ],
    "duplicate-names": [
        ("preset_duplicate_name", "erb/ITEM.erd", 1, 0, None, "taken"),
        ("preset_duplicate_name", "erb/ITEM.erd", 2, 1, None, "text"),
        ("preset_duplicate_name", "erb/ITEM.erd", 4, 3, None, "unique"),
    ],
    "unicode-order": [
        ("preset_name_conflict", "erb/a/ABL.erd", 1, 12, "digit_first", "slash_later"),
        ("preset_name_conflict", "erb/×/FLAG.erd", 1, 10, "umlaut_first", "multiply_later"),
        ("preset_name_conflict", "erb/𐐁/TFLAG.erd", 1, 11, "deseret_first", "deseret_later"),
    ],
}

# Fixed upstream resources: emuera-eng.xml / emuera-zhs.xml Error.PresetErd*;
# Japanese fallback: Runtime/Utils/EvilMask/Lang.cs. The English console envelope
# is observed in the existing headless capture and matches Error.Warning1.
BODY_TEMPLATES = [
    ("en", "preset_name_conflict", r'Preset ERD extension \((?P<file>[^)]+)\) has a different name at index (?P<index>[0-9]+) than CSV \(CSV="(?P<old>[^"]*)", ERD="(?P<new>[^"]*)"\); CSV takes precedence'),
    ("en", "preset_price_conflict_csv", r'Preset ERD extension \((?P<file>[^)]+)\) has a different price at index (?P<index>[0-9]+) than CSV \(CSV=(?P<old>-?[0-9]+), ERD=(?P<new>-?[0-9]+)\); CSV takes precedence'),
    ("en", "preset_duplicate_name", r'Preset ERD extension \((?P<file>[^)]+)\) defines name "(?P<new>[^"]*)" at index (?P<index>[0-9]+) which already exists in another slot; skipped'),
    ("zh", "preset_name_conflict", r'预设变量ERD扩展\((?P<file>[^)]+)\)的第(?P<index>[0-9]+)个元素名称与CSV中已有名称不同\(CSV="(?P<old>[^"]*)", ERD="(?P<new>[^"]*)"\);以CSV为准'),
    ("zh", "preset_price_conflict_csv", r'预设变量ERD扩展\((?P<file>[^)]+)\)的第(?P<index>[0-9]+)个元素价格与CSV中已有价格不同\(CSV=(?P<old>-?[0-9]+), ERD=(?P<new>-?[0-9]+)\);以CSV为准'),
    ("zh", "preset_duplicate_name", r'预设变量ERD扩展\((?P<file>[^)]+)\)的第(?P<index>[0-9]+)个元素名称"(?P<new>[^"]*)"已存在于其他槽位;已跳过'),
    ("ja", "preset_name_conflict", r'プリセット変数ERDの拡張\((?P<file>[^)]+)\)で、インデックス(?P<index>[0-9]+)の名前がCSVと異なります\(CSV="(?P<old>[^"]*)", ERD="(?P<new>[^"]*)"\);CSVを優先します'),
    ("ja", "preset_price_conflict_csv", r'プリセット変数ERDの拡張\((?P<file>[^)]+)\)で、インデックス(?P<index>[0-9]+)の価格がCSVと異なります\(CSV=(?P<old>-?[0-9]+), ERD=(?P<new>-?[0-9]+)\);CSVを優先します'),
    ("ja", "preset_duplicate_name", r'プリセット変数ERDの拡張\((?P<file>[^)]+)\)で、インデックス(?P<index>[0-9]+)の名前"(?P<new>[^"]*)"は既に他のスロットに存在します;スキップします'),
]


def require(condition, message):
    if not condition:
        raise ValueError(message)


def exact(actual, expected, message):
    require(same_json_value(actual, expected), message)


def expected_warnings(case, oracle, manifest):
    name = case["id"].removeprefix("preset-erd-")
    require(name in CASE_NAMES and case["id"] == f"preset-erd-{name}", "unknown preset case")
    warnings = WARNINGS.get(name, []) if oracle == "snake" else []
    expected = [{"kind": kind, "file": file, "sourceLineOneBased": line,
                 "rustCode": "csv.duplicateuserindex" if kind == "preset_duplicate_name" else "csv.duplicateindex"}
                for kind, file, line, *_ in warnings]
    contract = manifest.get("expectedLoadDiagnosticsSourceContract", {})
    exact(contract.get(oracle), expected, "fixture warning inventory differs from fixed contract")
    return warnings


def split_diagnostics(raw, policy, where):
    require(isinstance(raw, list) and all(isinstance(item, dict) for item in raw), f"{where}: missing diagnostics")
    setup, source = split_setup_diagnostics(raw, policy)
    require(len(setup) == (1 if policy["profile"] == "emuera.skia.snake" else 0), f"{where}: incorrect profile setup diagnostics")
    for item in setup:
        exact(item.get("context", {}).get("identity"), policy, f"{where}: setup identity differs")
        require(item.get("notification") == "default", f"{where}: setup notification differs")
        require(isinstance(item.get("message"), str), f"{where}: setup message absent")
    return setup, source


def rust_load_diagnostics(raw, policy, expected):
    setup, diagnostics = split_diagnostics(raw, policy, "Rust load")
    require(len(diagnostics) == len(expected), "Rust load: extra or missing source diagnostic")
    for actual, (kind, file, line, *_rest) in zip(diagnostics, expected):
        code = "csv.duplicateuserindex" if kind == "preset_duplicate_name" else "csv.duplicateindex"
        require(actual.get("code") == code and actual.get("level") == "warning", "Rust load: diagnostic code/level differs")
        require(actual.get("notification") == "log_only", "Rust load: diagnostic notification differs")
        source, context = actual.get("source"), actual.get("context")
        require(isinstance(source, dict) and isinstance(context, dict), "Rust load: missing diagnostic source/context")
        exact(source.get("relative_path"), file, "Rust load: diagnostic path differs")
        exact(source.get("line"), line - 1, "Rust load: zero-based source line differs")
        require(type(source.get("byte_start")) is int and type(source.get("byte_end")) is int
                and 0 <= source["byte_start"] <= source["byte_end"], "Rust load: invalid source byte span")
        require(context.get("stage") == "csv" and context.get("api") is None
                and context.get("required_capability") is None, "Rust load: diagnostic context differs")
        exact(context.get("identity"), policy, "Rust load: diagnostic identity differs")
        require(isinstance(actual.get("message"), str) and actual["message"], "Rust load: message absent")
    return setup


def parse_warning(text):
    envelope = re.fullmatch(r'Warning Lv(?P<level>[0-9]+):(?P<file>[^:]+): at line (?P<line>[0-9]+):(?P<body>.+)', text)
    require(envelope is not None, "unrecognized oracle loading output or warning envelope")
    fields = envelope.groupdict()
    for locale, kind, pattern in BODY_TEMPLATES:
        body = re.fullmatch(pattern, fields["body"])
        if body is None:
            continue
        values = body.groupdict()
        require(values["file"] == fields["file"], "oracle warning filename disagrees with its body")
        old, new = values.get("old"), values["new"]
        if kind == "preset_price_conflict_csv":
            old, new = int(old), int(new)
        return {"kind": kind, "level": int(fields["level"]), "file": fields["file"],
                "line": int(fields["line"]), "index": int(values["index"]),
                "old": old, "new": new, "templateLocale": locale, "raw": text}
    raise ValueError("unrecognized preset ERD warning body")


def oracle_load_diagnostics(load, expected):
    exact(load.get("diagnostics"), [], "oracle load: unexpected structured diagnostics")
    output = load.get("result", {}).get("output")
    require(isinstance(output, list) and all(isinstance(line, str) for line in output), "oracle load output absent")
    require(output.count("Now Loading...") == 1, "oracle loading marker missing or repeated")
    elapsed = 0
    warnings = []
    for text in output:
        if text == "Now Loading...":
            continue
        if re.fullmatch(r'Elapsed time:[0-9]+(?:\.[0-9]+)?ms', text):
            elapsed += 1
            # Existing original Debug-NAudio captures emit three load-phase timers.
            # These numeric presentation records are not source diagnostics.
            require(elapsed <= 3, "extra oracle elapsed-time loading output")
            continue
        warnings.append(parse_warning(text))
    expected_values = [{"kind": kind, "level": 1, "file": Path(file).name, "line": line,
                        "index": index, "old": old, "new": new}
                       for kind, file, line, index, old, new in expected]
    exact([{key: value for key, value in item.items() if key not in {"templateLocale", "raw"}}
           for item in warnings], expected_values, "oracle warning level/file/line/index/conflict values differ")
    return warnings


def effective_fixture_identity(fixture, oracle, drawing_mode):
    result = fixture_identity(fixture)
    if oracle == "snake":
        data = (fixture / "emuera.config").read_text().replace(
            "Drawing interface:TEXTRENDERER", "Drawing interface:" + drawing_mode).encode()
        for item in result["files"]:
            if item["path"] == "emuera.config":
                item.update(bytes=len(data), sha256=hashlib.sha256(data).hexdigest())
        encoded = json.dumps(result["files"], sort_keys=True, separators=(",", ":")).encode()
        result["sha256"] = hashlib.sha256(encoded).hexdigest()
    return result


NLS_FAILURE = {"case": "preset-erd-unicode-order",
               "error": "response.result.watches.RESULTS:11: expected 'deseret_first', got 'deseret_later'"}


def nls_execution(record, evidence_path, rust_path):
    """Bind the exception to the actual failed Wine command, not a provider guess."""
    command = record.get("command", [])
    require(record.get("exitCode") == 1 and record.get("env", {}).get(
        "DOTNET_SYSTEM_GLOBALIZATION_USENLS") == "1", "NLS execution record missing")
    for option, expected in (("--output", str(evidence_path.parent)), ("--rust-evidence", str(rust_path)),
                             ("--oracle", "snake"), ("--case", "preset-erd-unicode-order")):
        require(command.count(option) == 1 and command[command.index(option) + 1] == expected,
                "NLS execution does not bind this capture")
    require("--wine" in command, "NLS exception requires observed Wine execution")
    return record


def validate(evidence, rust, manifest, source_identity, effective_identity, nls_record=None):
    exact(evidence.get("version"), 1, "oracle capture version differs")
    exact(rust.get("version"), 1, "Rust capture version differs")
    nls = nls_record is not None
    if nls:
        exact(evidence.get("status"), "failed", "NLS raw failure must remain unchanged")
        exact(evidence.get("failure"), NLS_FAILURE, "unregistered oracle failure")
        exact(evidence.get("cases"), [], "unexpected completed cases in failed capture")
        exact(evidence.get("rustComparison"), {"status": "incomplete", "cases": []},
              "unexpected raw comparison in failed capture")
        exact(evidence.get("oracle"), "snake", "NLS exception is snake-only")
    else:
        require(evidence.get("status") == "completed_observations"
                and "failure" not in evidence, "incomplete oracle capture")
    oracle = evidence.get("oracle")
    require(oracle in BASELINES, "unknown oracle")
    exact(manifest.get("semanticBaselines"), BASELINES, "fixture baseline differs from fixed contract")
    exact(evidence.get("semanticBaseline"), BASELINES[oracle], "incorrect semantic baseline")
    exact(manifest.get("requiredRustPolicy"), {"original": {"semantic_version": 3, "policy_version": 3},
                                              "snake": {"semantic_version": 15, "policy_version": 15}},
          "fixture policy differs from final C contract")
    require(len(manifest.get("cases", [])) == 1 and len(rust.get("cases", [])) == 1
            and len(evidence.get("cases", [])) == (0 if nls else 1), "validate exactly one case before the next")
    case, rust_case = manifest["cases"][0], rust["cases"][0]
    if nls:
        exact(case["id"], NLS_FAILURE["case"], "NLS exception belongs to another case")
    require(case.get("group") == "PRESET_ERD" and oracle in case.get("allowedOracles", [])
            and oracle in manifest.get("allowedOracles", []), "case belongs to another group/profile")
    require(oracle == "snake" or case["id"] != "preset-erd-named-character", "new-name character case is snake-only")
    exact(rust_case.get("id"), case["id"], "Rust case identity differs")
    if not nls:
        completed = evidence["cases"][0]
        exact(completed.get("id"), case["id"], "completed oracle case differs")
        require(completed.get("group") == "PRESET_ERD" and completed.get("status") == "passed", "case capture did not complete")
        exact(completed.get("findings"), [], "extra oracle findings")
    exact(rust_case.get("group"), "PRESET_ERD", "Rust group differs")
    exact(evidence.get("seed"), manifest.get("seed"), "oracle seed differs")
    exact(evidence.get("sourceFixture", {}).get("files"), source_identity["files"], "oracle fixture identity differs")
    validate_rust_evidence(rust, oracle, source_identity, manifest["seed"], manifest["requiredRustPolicy"][oracle])
    exact(rust.get("seed"), manifest["seed"], "Rust seed differs")
    exact(evidence.get("sourceFixture"), source_identity, "oracle source aggregate differs")
    exact(evidence.get("effectiveFixture"), effective_identity, "effective fixture differs")
    require(evidence.get("drawingMode") in ({"SKIASHARP", "TEXTRENDERER"} if oracle == "snake"
            else {"TEXTRENDERER"}), "unsupported drawing mode")
    exact(evidence.get("presentationObservation"), "not_requested", "unexpected presentation contract")
    policy = rust["profile"]
    exact(evidence.get("rust"), {key: rust[key] for key in ("coreSha", "dirty", "profile")}, "oracle used another Rust capture identity")
    require(re.fullmatch(r'[0-9a-f]{40}', evidence.get("wrapperSha", "")) is not None, "wrapper SHA absent")
    records = evidence.get("requests")
    require(isinstance(records, list) and len(records) == 3, "capture must contain one handshake, load, and run")
    require(all(record.get("case") == case["id"] for record in records), "extra or foreign request record")
    exact(records[0].get("request"), {"op": "capabilities"}, "missing capabilities handshake")
    for ordinal, record in enumerate(records, 1):
        response = record.get("response", {})
        exact(response.get("schemaVersion"), 2, "oracle schema differs")
        exact(response.get("referenceCommit"), BASELINES[oracle], "request baseline differs")
        exact(response.get("id"), ordinal, "request identity is incomplete or repeated")
        exact(response.get("ok"), True, "oracle request was not accepted")
    exact(records[0]["response"].get("diagnostics"), [], "capabilities diagnostics present")
    load_request = records[1]["request"]
    exact(load_request.get("seed"), manifest["seed"], "load seed differs")
    require(isinstance(load_request.get("gameDir"), str) and load_request["gameDir"], "isolated oracle game path missing")
    exact(load_request.get("observePresentation"), False, "unexpected presentation request")
    case_fixtures = evidence.get("caseFixtures")
    require(isinstance(case_fixtures, list) and len(case_fixtures) == 1, "case isolation evidence missing")
    isolated = case_fixtures[0]
    exact(isolated.get("case"), case["id"], "isolated case differs")
    exact(isolated.get("gameDir"), load_request["gameDir"], "isolated load path differs")
    exact(isolated.get("initialSha256"), effective_identity["sha256"], "isolated fixture digest differs")
    exact(isolated.get("isolation"), "fresh process and working directory per case; requests within a case share both", "isolation contract differs")
    exact(isolated.get("capabilities"), records[0]["response"], "isolated handshake differs")
    exact(evidence.get("capabilities"), records[0]["response"], "global handshake differs")
    load, steps = recorded_steps(evidence, case, manifest.get("loadExpect"))
    require(len(steps) == 1 and len(case.get("requests", [])) == 1, "expected exactly one operation")
    expected = expected_warnings(case, oracle, manifest)
    actual_load = rust_case.get("load", {})
    exact(actual_load.get("success"), True, "Rust fixture loading failed")
    exact(actual_load.get("compatibility"), policy, "loaded Rust identity differs")
    setup = rust_load_diagnostics(actual_load.get("diagnostics"), policy, expected)
    exact(rust_case.get("setupDiagnostics"), actual_load["diagnostics"], "Rust setup diagnostics differ from load")
    reference_expected = list(expected)
    if nls:
        reference_expected[-1] = ("preset_name_conflict", "erb/𐐨/TFLAG.erd", 1, 11,
                                  "deseret_later", "deseret_first")
    oracle_warnings = oracle_load_diagnostics(load, reference_expected)
    require(len(rust_case.get("steps", [])) == 1, "missing Rust operation")
    actual_step, reference = rust_case["steps"][0], steps[0]["response"]
    request = case["requests"][0]["request"]
    require(actual_step.get("status") == "executed" and actual_step.get("reason") in (None, ""), "Rust step was not executed")
    exact(actual_step.get("request"), request, "Rust operation request differs")
    actual = actual_step.get("result", {})
    exact(actual.get("ok"), True, "Rust operation failed")
    exact(actual.get("termination"), "completed", "Rust operation did not complete")
    exact(reference.get("result", {}).get("termination"), "completed", "oracle operation did not complete")
    exact(actual.get("observationBlocks"), [], "Rust observation blocked")
    require(actual.get("inputObservation") == "consumed", "Rust input observation incomplete")
    operation_setup, operation_diagnostics = split_diagnostics(actual.get("diagnostics"), policy, "Rust operation")
    exact(operation_diagnostics, [], "unexpected Rust operation diagnostic")
    exact(operation_setup, setup, "profile setup notification changed during operation")
    exact(reference.get("diagnostics"), [], "unexpected oracle operation diagnostic")
    expected_watches = case["requests"][0]["expect"][oracle]["result"]["watches"]
    exact(sorted(request.get("watch", [])), sorted(expected_watches), "planned watch inventory differs")
    exact(actual.get("watches"), expected_watches, "Rust watches differ from explicit expectations")
    reference_watches = dict(expected_watches)
    if nls:
        reference_watches.update({"RESULTS:11": "deseret_later", "RESULT:12": -1, "RESULT:13": 11})
    exact(reference.get("result", {}).get("watches"), reference_watches, "oracle watches differ from explicit expectations")
    exact(actual.get("output"), [], "extra Rust operation output")
    reference_output, _ = output_after_load(reference["result"].get("output"), load)
    exact(reference_output, [], "extra or changed oracle operation output")
    for where, logs in (("setup", rust_case.get("setupHostLogs")), ("operation", actual.get("hostLogs"))):
        require(isinstance(logs, list) and all(isinstance(log, dict) and log.get("level") == "debug"
                and "code" not in log for log in logs), f"unexpected Rust {where} host diagnostic")
    recomputed = compare_case(case, steps, rust_case, load, policy)
    if nls:
        raw = recomputed
        exact(raw.get("status"), "different", "expected raw provider difference")
        exact(raw["steps"][0].get("status"), "different", "expected step provider difference")
        exact(raw["steps"][0].get("differences"), [{"field": "watches", "rustPresent": True,
              "oraclePresent": True, "rust": expected_watches, "oracle": reference_watches}],
              "additional unregistered provider difference")
    else:
        comparison = evidence.get("rustComparison", {})
        require(comparison.get("status") == "compared" and len(comparison.get("cases", [])) == 1, "incomplete comparison")
        raw = comparison["cases"][0]
        exact(raw, recomputed, "raw comparator evidence differs from its recorded inputs or has extra differences")
        require(raw.get("status") == "matched_observables" and raw["steps"][0].get("status") == "matched_observables", "unregistered raw verdict")
        exact(raw["steps"][0].get("differences"), [], "unregistered differences")
    exact(raw["steps"][0].get("registeredDifferences"), [], "unrequested registered differences")
    return {"case": case["id"], "oracle": oracle, "status": "accepted_registered_provider_difference" if nls else "accepted_observables",
            "providerDifference": {"rust": "fixed_dotnet8_icu72", "oracle": "Wine NLS",
                "rawCaptureStatus": evidence["status"], "rawFailure": evidence.get("failure"),
                "execution": nls_record} if nls else None,
            "rawVerdict": raw["status"], "rawComparison": raw,
            "loadDiagnosticValidation": {"status": "separately_checked_schemas", "diagnosticEquivalence": False,
                "schemaComparison": "incomparable_schema", "rust": actual_load["diagnostics"],
                "oracle": load["diagnostics"], "oracleLoadOutput": load["result"]["output"],
                "oracleConsoleWarnings": oracle_warnings, "profileSetupDiagnostics": setup,
                "pathBoundary": "Rust retains relative paths; oracle EraStreamReader retains basename only."},
            "expectedSource": case.get("expectedSource"), "semanticBaseline": BASELINES[oracle]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("fixture", "evidence", "rust", "output"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    parser.add_argument("--nls-execution-records", type=Path)
    args = parser.parse_args()
    oracle_bytes, rust_bytes = args.evidence.read_bytes(), args.rust.read_bytes()
    evidence = json.loads(oracle_bytes)
    nls_record = None
    if args.nls_execution_records:
        records = [json.loads(line) for line in args.nls_execution_records.read_text().splitlines()]
        matches = [record for record in records if "--output" in record.get("command", [])
                   and record["command"][record["command"].index("--output") + 1] == str(args.evidence.parent)]
        require(len(matches) == 1, "expected exactly one matching execution record")
        nls_record = nls_execution(matches[0], args.evidence, args.rust)
    result = validate(evidence, json.loads(rust_bytes),
                      json.loads((args.fixture / "cases.json").read_text()), fixture_identity(args.fixture),
                      effective_fixture_identity(args.fixture, evidence.get("oracle"), evidence.get("drawingMode")), nls_record)
    result["provenance"] = {"evidenceSha256": hashlib.sha256(oracle_bytes).hexdigest(),
                            "rustSha256": hashlib.sha256(rust_bytes).hexdigest(),
                            "sources": {name: hashlib.sha256(Path(__file__).with_name(name).read_bytes()).hexdigest()
                                        for name in (Path(__file__).name, "comparison.py", "recompare.py", "run.py")}}
    with args.output.open("x", encoding="utf-8") as stream:
        json.dump(result, stream, ensure_ascii=False, indent=2)
        stream.write("\n")
    print(json.dumps({"case": result["case"], "oracle": result["oracle"], "status": result["status"],
                      "rawVerdict": result["rawVerdict"], "output": str(args.output)}))


if __name__ == "__main__":
    main()
