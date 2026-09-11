"""Synthetic protocol regression tests; no engine launch or production merge algorithm."""
import copy
import unittest

from comparison import compare_case
from validate_preset_erd_observations import BASELINES, oracle_load_diagnostics, parse_warning, validate


def fixture(oracle="snake", conflict=False):
    name = "name-conflict" if conflict else "main"
    case_id = "preset-erd-" + name
    version = 15 if oracle == "snake" else 3
    policy = {"profile": "emuera.skia.snake" if oracle == "snake" else "emuera.em",
              "semantic_version": version, "policy_version": version,
              "arithmetic": "snake_saturating_i64_v1" if oracle == "snake" else "wrapping_i64_v1",
              "rng_algorithm": "sfmt19937", "rng_state_version": 1,
              "layout": "unicode_column_v1", "save_codec": "snake_emuera1808_interop_v1" if oracle == "snake" else "emuera1808",
              "services": [{"name": name, "version": 1} for name in
                           ("rustyera.sql", "rustyera.sql.limits", "rustyera.scene", "rustyera.audio")] if oracle == "snake" else []}
    source = {"sha256": "a" * 64, "files": [{"path": "erb/case.erb", "bytes": 1, "sha256": "b" * 64}]}
    watches = {"RESULT:10": 7, "RESULTS:10": "retained"}
    request = {"op": "run", "entry": "PRESET_ERD_CASE", "watch": list(watches)}
    expect = {"ok": True, "result": {"termination": "completed", "watches": watches}}
    case = {"id": case_id, "group": "PRESET_ERD", "allowedOracles": ["original", "snake"],
            "targetBatch": "snake-upstream-C", "snakeTargetStatus": "source_derived_candidate_not_executed",
            "requireSuccessfulLoad": True, "requests": [{"request": request, "expect": {"original": expect, "snake": expect}}]}
    contract = [{"kind": "preset_name_conflict", "file": path, "sourceLineOneBased": 1,
                 "rustCode": "csv.duplicateindex"} for path in ("erb/a/ABL.erd", "erb/b/ABL.erd")] if conflict else []
    manifest = {"seed": 123456, "semanticBaselines": BASELINES,
                "requiredRustPolicy": {"original": {"semantic_version": 3, "policy_version": 3},
                                       "snake": {"semantic_version": 15, "policy_version": 15}},
                "allowedOracles": ["original", "snake"], "cases": [case],
                "loadExpect": {"ok": True, "result": {"termination": "waitingInput"}},
                "expectedLoadDiagnosticsSourceContract": {"original": [], "snake": contract}}

    def diagnostic(code, path, stage, notification, line=None):
        return {"code": code, "level": "warning", "message": "independent raw message",
                "notification": notification,
                "source": {"relative_path": path, "line": line, "byte_start": 0, "byte_end": 0, "byte_column": None},
                "context": {"stage": stage, "identity": policy, "api": None, "required_capability": None}}

    setup = [diagnostic("runtime.experimental_compatibility_profile", "reraconfig.toml", "configuration", "default")] if oracle == "snake" else []
    warnings = [diagnostic("csv.duplicateindex", row["file"], "csv", "log_only", 0) for row in contract] if oracle == "snake" else []
    load_output = ["Now Loading..."]
    if conflict and oracle == "snake":
        load_output.extend([
            'Warning Lv1:ABL.erd: at line 1:Preset ERD extension (ABL.erd) has a different name at index 0 than CSV (CSV="csv_abl", ERD="ignored_csv_conflict"); CSV takes precedence',
            'Warning Lv1:ABL.erd: at line 1:Preset ERD extension (ABL.erd) has a different name at index 1 than CSV (CSV="first_erd", ERD="later_erd"); CSV takes precedence',
        ])
    actual = {"ok": True, "termination": "completed", "watches": watches, "output": [],
              "diagnostics": setup, "observationBlocks": [], "inputObservation": "consumed", "hostLogs": []}
    rust_case = {"id": case_id, "group": "PRESET_ERD", "load": {"success": True, "compatibility": policy, "diagnostics": warnings + setup},
                 "setupDiagnostics": warnings + setup, "setupHostLogs": [],
                 "steps": [{"status": "executed", "reason": None, "request": request, "result": actual}]}
    rust = {"version": 1, "coreSha": "c" * 40, "dirty": False, "profile": policy, "seed": 123456,
            "sourceFixture": source, "cases": [rust_case]}

    def response(number, result):
        return {"id": number, "schemaVersion": 2, "referenceCommit": BASELINES[oracle],
                "ok": True, "diagnostics": [], "result": result}

    capabilities = response(1, {"observationVersions": {"presentationSnapshot": 1, "headlessInputTrace": 1}})
    load = response(2, {"termination": "waitingInput", "output": load_output})
    operation = response(3, {"termination": "completed", "output": load_output[:], "watches": watches})
    records = [{"case": case_id, "request": {"op": "capabilities"}, "response": capabilities},
               {"case": case_id, "request": {"op": "load", "seed": 123456, "gameDir": "Z:\\isolated", "observePresentation": False}, "response": load},
               {"case": case_id, "request": request, "response": operation}]
    evidence = {"version": 1, "status": "completed_observations", "oracle": oracle, "semanticBaseline": BASELINES[oracle],
                "wrapperSha": "d" * 40, "seed": 123456, "sourceFixture": source, "effectiveFixture": source,
                "drawingMode": "SKIASHARP" if oracle == "snake" else "TEXTRENDERER", "presentationObservation": "not_requested",
                "rust": {key: rust[key] for key in ("coreSha", "dirty", "profile")},
                "cases": [{"id": case_id, "group": "PRESET_ERD", "status": "passed", "findings": []}],
                "requests": records, "capabilities": capabilities,
                "caseFixtures": [{"case": case_id, "gameDir": "Z:\\isolated", "initialSha256": source["sha256"],
                                  "isolation": "fresh process and working directory per case; requests within a case share both",
                                  "capabilities": capabilities}],
                "rustComparison": {"status": "compared", "cases": [compare_case(case, records[2:], rust_case, load, policy)]}}
    # Break references shared for concise construction so each mutation targets one raw field.
    return tuple(copy.deepcopy(value) for value in (evidence, rust, manifest, source, source))


class PresetErdAcceptanceTests(unittest.TestCase):
    def reject(self, change, oracle="snake", conflict=True):
        values = fixture(oracle, conflict)
        change(*values)
        with self.assertRaises((ValueError, KeyError, TypeError)):
            validate(*values)

    def test_success_and_raw_verdict_retained(self):
        for oracle in ("original", "snake"):
            for conflict in (False, True):
                with self.subTest(oracle=oracle, conflict=conflict):
                    result = validate(*fixture(oracle, conflict))
                    self.assertEqual(result["status"], "accepted_observables")
                    self.assertEqual(result["rawVerdict"], "matched_observables")
                    self.assertFalse(result["loadDiagnosticValidation"]["diagnosticEquivalence"])
                    self.assertEqual(result["loadDiagnosticValidation"]["schemaComparison"], "incomparable_schema")

    def test_extra_diagnostic(self):
        self.reject(lambda e, r, *_: r["cases"][0]["load"]["diagnostics"].append({"code": "extra"}))

    def test_wrong_code_line_and_path(self):
        for field, value in (("code", "csv.other"), ("line", 1), ("line", False), ("relative_path", "erb/other.erd")):
            def change(e, r, *_, field=field, value=value):
                diagnostic = r["cases"][0]["load"]["diagnostics"][0]
                (diagnostic if field == "code" else diagnostic["source"])[field] = value
            with self.subTest(field=field, value=value):
                self.reject(change)

    def test_extra_original_warning(self):
        self.reject(lambda e, r, *_: r["cases"][0]["load"]["diagnostics"].append({"code": "csv.duplicateindex"}), "original")

    def test_wrong_or_extra_watch(self):
        for key, value in (("RESULT:10", 8), ("RESULT:99", 0), ("RESULT:10", True)):
            self.reject(lambda e, r, *_, key=key, value=value: r["cases"][0]["steps"][0]["result"]["watches"].update({key: value}))

    def test_wrong_baseline_or_profile(self):
        self.reject(lambda e, *_: e.update(semanticBaseline="0" * 40))
        self.reject(lambda e, r, *_: r["profile"].update(policy_version=14))
        self.reject(lambda e, r, m, *_: m.update(allowedOracles=["original"]))

    def test_capture_identity_and_completeness(self):
        self.reject(lambda e, *_: e.update(version=True))
        self.reject(lambda e, *_: e.update(status="pending"))
        self.reject(lambda e, *_: e["requests"].pop())
        self.reject(lambda e, *_: e["sourceFixture"]["files"][0].update(sha256="0" * 64))
        self.reject(lambda e, *_: e["caseFixtures"][0].update(initialSha256="0" * 64))
        self.reject(lambda e, *_: e.update(seed=42))

    def test_extra_output_difference_or_runtime_diagnostic(self):
        self.reject(lambda e, r, *_: r["cases"][0]["steps"][0]["result"]["output"].append("extra"))
        self.reject(lambda e, *_: e["rustComparison"]["cases"][0]["steps"][0]["differences"].append({"field": "extra"}))
        self.reject(lambda e, *_: e["requests"][2]["response"]["diagnostics"].append({"message": "extra"}))

    def test_windows_separator_conflict_fields(self):
        warning = parse_warning('Warning Lv1:ABL.erd: at line 1:Preset ERD extension (ABL.erd) has a different name at index 12 than CSV (CSV="digit_first", ERD="slash_later"); CSV takes precedence')
        self.assertEqual((warning["file"], warning["line"], warning["index"], warning["old"], warning["new"]),
                         ("ABL.erd", 1, 12, "digit_first", "slash_later"))

    def test_price_and_duplicate_templates(self):
        price = parse_warning('Warning Lv1:ITEMPRICE.erd: at line 1:Preset ERD extension (ITEMPRICE.erd) has a different price at index 0 than CSV (CSV=0, ERD=4); CSV takes precedence')
        self.assertEqual((price["kind"], price["index"], price["old"], price["new"]),
                         ("preset_price_conflict_csv", 0, 0, 4))
        duplicate = parse_warning('Warning Lv1:ITEM.erd: at line 4:Preset ERD extension (ITEM.erd) defines name "unique" at index 3 which already exists in another slot; skipped')
        self.assertEqual((duplicate["kind"], duplicate["line"], duplicate["index"], duplicate["new"]),
                         ("preset_duplicate_name", 4, 3, "unique"))

    def test_original_three_load_timers_are_bounded_metadata(self):
        load = {"diagnostics": [], "result": {"output": ["Now Loading..."] +
                ["Elapsed time:1.5ms", "Elapsed time:2ms", "Elapsed time:3.0ms"]}}
        self.assertEqual(oracle_load_diagnostics(load, []), [])
        load["result"]["output"].append("Elapsed time:4ms")
        with self.assertRaises(ValueError):
            oracle_load_diagnostics(load, [])

    def test_warning_fields_are_parsed(self):
        for old, new in (("Lv1", "Lv2"), ("line 1", "line 2"), ("index 0", "index 9"), ('CSV="csv_abl"', 'CSV="wrong"')):
            def change(e, *_, old=old, new=new):
                output = e["requests"][1]["response"]["result"]["output"]
                output[1] = output[1].replace(old, new)
            self.reject(change)
        with self.assertRaises(ValueError):
            parse_warning("Warning Lv1:ABL.erd: at line 1:unknown warning")


if __name__ == "__main__":
    unittest.main()

class NlsProviderBoundaryTests(unittest.TestCase):
    def test_execution_record_must_bind_provider_and_both_inputs(self):
        from pathlib import Path
        from validate_preset_erd_observations import nls_execution
        evidence, rust = Path('/capture/evidence.json'), Path('/capture/rust.json')
        record = {'exitCode': 1, 'env': {'DOTNET_SYSTEM_GLOBALIZATION_USENLS': '1'},
                  'command': ['python', 'run.py', '--oracle', 'snake', '--case',
                              'preset-erd-unicode-order', '--output', '/capture',
                              '--rust-evidence', '/capture/rust.json', '--wine', '/wine']}
        self.assertEqual(nls_execution(record, evidence, rust), record)
        for changed in [dict(record, exitCode=0), dict(record, env={}),
                        dict(record, command=record['command'][:-2])]:
            with self.assertRaises(ValueError):
                nls_execution(changed, evidence, rust)
        with self.assertRaises(ValueError):
            nls_execution(record, Path('/other/evidence.json'), rust)
        with self.assertRaises(ValueError):
            nls_execution(record, evidence, Path('/other/rust.json'))

    def test_provider_exception_cannot_accept_other_case_or_failure(self):
        from validate_preset_erd_observations import NLS_FAILURE
        values = fixture()
        evidence = values[0]
        evidence.update(status='failed', failure=dict(NLS_FAILURE), cases=[],
                        rustComparison={'status': 'incomplete', 'cases': []})
        with self.assertRaisesRegex(ValueError, 'another case'):
            validate(*values, nls_record={'provider': 'NLS'})
        evidence['failure']['error'] = 'timeout'
        with self.assertRaisesRegex(ValueError, 'unregistered oracle failure'):
            validate(*values, nls_record={'provider': 'NLS'})
