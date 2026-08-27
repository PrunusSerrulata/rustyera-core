"""Focused driver tests; run only after the batch review and static prerequisites."""

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace

from comparison import compare_case, output_after_load, split_setup_diagnostics, validate_rust_evidence
from recompare import recorded_steps


spec = importlib.util.spec_from_file_location("oracle_driver", Path(__file__).with_name("run.py"))
driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(driver)


class DriverTests(unittest.TestCase):
    def test_logical_only_observation_never_claims_or_allows_presentation_validation(self):
        font = {"family": "BIZ UDGothic", "sha256": "a" * 64}
        case = {"group": "INDEX", "requests": [{"request": {"op": "eval", "source": "1"}}]}
        self.assertEqual(driver.load_observation_options(True, [case], font, "font.ttf"),
                         {"observePresentation": False})
        self.assertEqual(driver.load_observation_options(False, [case], font, "font.ttf"),
                         {"observePresentation": True, "presentationFont": {**font, "file": "font.ttf"}})
        for presentation_case in [{**case, "group": "PRINTC"},
                                  {**case, "assertions": ["presentation"]},
                                  {**case, "requests": [{"request": {"op": "observe", "observePresentation": True}}]}]:
            with self.assertRaisesRegex(ValueError, "presentation assertions"):
                driver.load_observation_options(True, [presentation_case], font, "font.ttf")

    def test_expected_operation_rejection_cannot_hide_failed_fixture_loading(self):
        with self.assertRaises(AssertionError):
            driver.validate_load({"ok": True, "result": {"termination": "error"}})
        with self.assertRaises(AssertionError):
            driver.validate_load({"ok": False, "result": {}})
        driver.validate_load({"ok": True, "result": {"termination": "waitingInput"}},
                             {"result": {"termination": "waitingInput"}})
        case = {"id": "bounds", "targetBatch": 1, "snakeTargetStatus": "requires_batch_1_acceptance",
                "requireSuccessfulLoad": True, "requests": []}
        for load in [None, {"success": False}]:
            result = compare_case(case, [], {"load": load})
            self.assertEqual(result["status"], "blocked")
            self.assertIn("load failure", result["reason"])

    def test_expected_rejection_is_recorded_without_claiming_diagnostic_equivalence(self):
        step = {"expect": {"original": {"ok": True, "result": {"termination": "error"}}},
                "expectedRejection": {"original": "unsupported user alias"}}
        findings = driver.step_expectations(step, {"ok": True, "result": {"termination": "error"}}, "original")
        self.assertEqual(findings[0]["kind"], "expected_rejection")
        self.assertEqual(findings[0]["diagnosticComparison"], "incomparable_schema")
        with self.assertRaises(AssertionError):
            driver.step_expectations(step, {"ok": True, "result": {"termination": "completed"}}, "original")

    def test_handled_oracle_request_is_not_successful_script_execution(self):
        request = {"op": "run", "entry": "DIVIDE_ZERO"}
        case = {"id": "divide", "group": "arithmetic", "targetBatch": 2,
                "snakeTargetStatus": "deferred_semantics", "requests": [{"request": request}]}
        rust = {"steps": [{"request": request, "status": "executed", "result": {
            "ok": False, "termination": "faulted", "diagnostics": [{"code": "vm_fault"}]}}]}
        response = {"ok": True, "diagnostics": [], "result": {"termination": "error"}}
        result = compare_case(case, [{"request": request, "response": response}], rust)
        self.assertEqual(result["status"], "incomparable")
        self.assertEqual(result["steps"][0]["differences"], [])
        self.assertTrue(result["steps"][0]["oracleRequestAccepted"])
        self.assertEqual(result["steps"][0]["oracle"]["result"]["termination"], "error")

    def test_output_removes_only_the_exact_load_prefix_and_preserves_script_lines(self):
        load = {"result": {"output": ["Now Loading...", "Elapsed time:3ms", "COMPAT_READY"]}}
        raw = load["result"]["output"] + ["Now Loading...", "COMPAT_READY", "script"]
        self.assertEqual(output_after_load(raw, load),
                         (["Now Loading...", "COMPAT_READY", "script"], "exact_load_prefix_removed"))
        self.assertEqual(len(raw), 6)
        self.assertEqual(output_after_load(["different buffer"], load),
                         (None, "incomparable_load_prefix_changed"))
        with self.assertRaises(ValueError):
            output_after_load([], {"result": {}})

    def test_run_comparison_separates_setup_and_declines_changed_prefix(self):
        request = {"op": "run", "entry": "CASE"}
        case = {"id": "case", "group": "REF", "targetBatch": 6,
                "snakeTargetStatus": "deferred_semantics", "requests": [{"request": request}]}
        load = {"result": {"output": ["setup"]}}
        response = {"ok": True, "diagnostics": [],
                    "result": {"termination": "returned", "watches": {}, "output": ["setup", "script"]}}
        rust = {"steps": [{"request": request, "status": "executed", "result": {
            "ok": True, "diagnostics": [], "termination": "returned", "watches": {}, "output": ["script"]}}]}
        steps = [{"request": request, "response": response}]
        compared = compare_case(case, steps, rust, load)
        self.assertEqual(compared["status"], "matched_observables")
        self.assertEqual(compared["steps"][0]["oracle"]["result"]["output"], ["setup", "script"])
        response["result"]["output"] = ["reset", "script"]
        compared = compare_case(case, steps, rust, load)
        self.assertEqual(compared["status"], "incomparable")
        self.assertNotIn("output", compared["steps"][0]["compared"])

    def test_only_exact_configuration_warning_is_setup_not_script_diagnostic(self):
        identity = {"profile": "emuera.skia.snake", "policy_version": 1}
        warning = {"code": "runtime.experimental_compatibility_profile", "level": "warning",
                   "context": {"stage": "configuration", "identity": identity},
                   "source": {"relative_path": "reraconfig.toml", "byte_start": 0, "byte_end": 0}}
        script = {**warning, "source": {"relative_path": "ERB/main.erb", "byte_start": 0, "byte_end": 0}}
        setup, remaining = split_setup_diagnostics([warning, script], identity)
        self.assertEqual(setup, [warning])
        self.assertEqual(remaining, [script])
        self.assertEqual(split_setup_diagnostics([warning], None), ([], [warning]))
        wrong_identity = {"profile": "emuera.skia.snake", "policy_version": 2}
        self.assertEqual(split_setup_diagnostics([warning], wrong_identity), ([], [warning]))

    def test_offline_record_selection_rejects_missing_load_or_changed_requests(self):
        request = {"op": "eval", "source": "1"}
        case = {"id": "case", "requests": [{"request": request}]}
        envelope = {"ok": True, "schemaVersion": 2, "referenceCommit": "a" * 40}
        evidence = {"semanticBaseline": "a" * 40, "requests": [
            {"case": "case", "request": {"op": "load"}, "response": envelope},
            {"case": "case", "request": request, "response": envelope}]}
        self.assertEqual(recorded_steps(evidence, case)[1], evidence["requests"][1:])
        evidence["requests"][1]["request"] = {"op": "eval", "source": "2"}
        with self.assertRaises(ValueError):
            recorded_steps(evidence, case)
        evidence["requests"].pop(0)
        with self.assertRaises(ValueError):
            recorded_steps(evidence, case)

    def test_rng_assertions_preserve_the_pinned_snake_state_loss(self):
        case = {"assertions": ["rng_roundtrip"]}
        snake = SimpleNamespace(args=SimpleNamespace(oracle="snake"))
        last = {"result": {"randomSeed": 123456, "randomAlgorithm": "sfmt19937",
                           "watches": {"RESULT:0": 192905, "RESULT:1": 520548,
                                       "RESULT:2": 0, "RESULT:3": 0}}}
        findings = driver.assertions(snake, case, last, {})
        self.assertFalse(findings[0]["roundtrip"])
        original = SimpleNamespace(args=SimpleNamespace(oracle="original"))
        with self.assertRaises(AssertionError):
            driver.assertions(original, case, last, {})
        last["result"]["watches"]["RESULT:2"] = 192905
        last["result"]["watches"]["RESULT:3"] = 520548
        self.assertEqual(driver.assertions(original, case, last, {}), [])
        with self.assertRaises(AssertionError):
            driver.assertions(snake, case, last, {})

    def test_cleanup_error_does_not_lose_the_primary_failure_or_requests(self):
        def denied():
            raise PermissionError("sandbox denied cleanup")
        oracle = SimpleNamespace(case="setup", records=[{"request": "capabilities"}], close=denied)
        evidence = {"failure": {"case": "setup", "error": "unchanged observations"}}
        failure = driver.close_oracle(oracle, evidence, "unchanged observations")
        self.assertEqual(failure, "unchanged observations")
        self.assertEqual(evidence["requests"], oracle.records)
        self.assertIn("PermissionError", evidence["cleanupFailure"])
    def test_comparison_preserves_differences_and_blocked_execution(self):
        request = {"op": "eval", "source": "9223372036854775807 + 1"}
        case = {
            "id": "overflow",
            "group": "arithmetic",
            "targetBatch": 2,
            "snakeTargetStatus": "deferred_semantics",
            "requests": [{"request": request}],
        }
        oracle = [
            {
                "request": request,
                "response": {
                    "ok": True,
                    "diagnostics": [],
                    "result": {"value": 9223372036854775807},
                },
            }
        ]
        rust = {
            "steps": [
                {
                    "request": request,
                    "status": "executed",
                    "result": {"ok": True, "diagnostics": [], "value": -9223372036854775808},
                }
            ]
        }
        result = compare_case(case, oracle, rust)
        self.assertEqual(result["status"], "different")
        self.assertEqual(result["steps"][0]["differences"][0]["field"], "value")
        rust["steps"][0].update(status="blocked", reason="compile diagnostic")
        self.assertEqual(compare_case(case, oracle, rust)["status"], "blocked")
        rust["steps"][0]["request"] = {"op": "eval", "source": "1"}
        with self.assertRaises(ValueError):
            compare_case(case, oracle, rust)

    def test_production_shaped_eval_and_run_ignore_only_harness_fields(self):
        for operation in ("eval", "run"):
            request = {"op": operation, "source": "7"} if operation == "eval" else {"op": operation, "entry": "CASE"}
            case = {"id": operation, "group": "arithmetic", "targetBatch": 2,
                    "snakeTargetStatus": "deferred_semantics", "requests": [{"request": request}]}
            actual = {"ok": True, "value": 7 if operation == "eval" else None,
                      "termination": "returned", "output": ["A", "B"], "watches": {}, "diagnostics": []}
            expected = {"value": 7, "expression": {"kind": "integer"}} if operation == "eval" else {
                "termination": "returned", "output": ["A", "B"], "watches": {}}
            rust = {"steps": [{"request": request, "status": "executed", "result": actual, "hostLogs": ["ready"]}]}
            oracle = [{"request": request, "response": {"ok": True, "result": expected, "diagnostics": []}}]
            self.assertEqual(compare_case(case, oracle, rust)["status"], "matched_observables")
            if operation == "run":
                actual["output"] = ["AB"]
                self.assertEqual(compare_case(case, oracle, rust)["status"], "different")
                actual["output"] = "A\nB"
                with self.assertRaises(ValueError):
                    compare_case(case, oracle, rust)

    def test_diagnostics_and_errors_cannot_be_reported_as_matches(self):
        request = {"op": "eval", "source": "bad"}
        case = {"id": "bad", "group": "TOINT", "targetBatch": 2,
                "snakeTargetStatus": "deferred_semantics", "requests": [{"request": request}]}
        rust = {"steps": [{"request": request, "status": "executed",
                           "result": {"ok": False, "diagnostics": [{"code": "fault"}]}}]}
        response = {"ok": False, "diagnostics": [], "error": {"type": "CodeEE", "message": "bad"}}
        result = compare_case(case, [{"request": request, "response": response}], rust)
        self.assertEqual(result["status"], "incomparable")
        self.assertEqual(result["steps"][0]["oracle"], response)
        rust["steps"][0]["result"]["ok"] = True
        self.assertEqual(compare_case(case, [{"request": request, "response": response}], rust)["status"], "different")

    def test_rust_evidence_rejects_stale_sources_and_wrong_profiles(self):
        fixture = {"files": [{"path": "a.erb", "bytes": 1, "sha256": "a" * 64}]}
        evidence = {
            "version": 1,
            "coreSha": "a" * 40,
            "dirty": True,
            "seed": 1,
            "sourceFixture": fixture,
            "cases": [],
            "profile": {
                "profile": "emuera.em",
                "semantic_version": 1,
                "policy_version": 1,
                "arithmetic": "wrapping_i64_v1",
                "rng_algorithm": "sfmt19937",
                "rng_state_version": 1,
                "layout": "unicode_column_v1",
                "save_codec": "emuera1808",
                "services": [],
            },
        }
        validate_rust_evidence(evidence, "original", fixture, 1)
        with self.assertRaises(ValueError):
            validate_rust_evidence(evidence, "snake", fixture, 1)
        with self.assertRaises(ValueError):
            validate_rust_evidence(evidence, "original", {"files": []}, 1)
        with self.assertRaises(ValueError):
            validate_rust_evidence(evidence, "original", fixture, 2)
        snake = {**evidence, "profile": {**evidence["profile"],
                 "profile": "emuera.skia.snake", "semantic_version": 2, "policy_version": 2,
                 "save_codec": "rustyera_envelope_v1:emuera1808"}}
        required = {"semantic_version": 2, "policy_version": 2}
        validate_rust_evidence(snake, "snake", fixture, 1, required)
        historical = {**snake, "profile": {**snake["profile"], "semantic_version": 1, "policy_version": 1}}
        validate_rust_evidence(historical, "snake", fixture, 1)
        with self.assertRaises(ValueError):
            validate_rust_evidence(historical, "snake", fixture, 1, required)
        unknown = {**snake, "profile": {**snake["profile"], "policy_version": 3}}
        with self.assertRaises(ValueError):
            validate_rust_evidence(unknown, "snake", fixture, 1)

    def test_watchdog_ignores_envelope_ids_but_keeps_script_state(self):
        first = {"request": {"op": "observe", "id": 1}, "lastAvailableResponse": {"id": 1, "result": {"id": 7}}}
        second = {"request": {"op": "observe", "id": 2}, "lastAvailableResponse": {"id": 2, "result": {"id": 7}}}
        self.assertEqual(driver.comparison_snapshot(first), driver.comparison_snapshot(second))
        second["lastAvailableResponse"]["result"]["id"] = 8
        self.assertNotEqual(driver.comparison_snapshot(first), driver.comparison_snapshot(second))

    def test_subset_keeps_numeric_and_diagnostic_differences(self):
        driver.subset(
            {"ok": True, "diagnostics": [], "result": {"value": 7, "type": "integer"}},
            {"ok": True, "result": {"value": 7}},
        )
        with self.assertRaises(AssertionError):
            driver.subset({"result": {"value": 0}}, {"result": {"value": 7}})
        with self.assertRaises(AssertionError):
            driver.subset({"result": {}}, {"result": {"value": 0}})

    def test_input_identity_includes_content_and_relative_path(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "sample.erb").write_text("PRINTL A\n")
            first = driver.identity(root)
            self.assertEqual(first, driver.identity(root))
            (root / "sample.erb").write_text("PRINTL B\n")
            self.assertNotEqual(first["sha256"], driver.identity(root)["sha256"])
            self.assertEqual(first["files"][0]["path"], "sample.erb")

    def test_manifest_preserves_raw_source_cases_and_target_dispositions(self):
        manifest = json.loads((driver.FIXTURE / "cases.json").read_text())
        cases = {case["id"]: case for case in manifest["cases"]}
        self.assertEqual(
            set(case["group"] for case in cases.values()),
            {"PRINTC", "arithmetic", "RNG", "REF", "extra_args", "TOINT", "GETKEY"},
        )
        for case in cases.values():
            self.assertEqual(case["snakeTargetStatus"], "deferred_semantics")
        click = cases["key-same-pump-click"]["requests"][0]["request"]["inputTrace"]
        self.assertEqual([event["down"] for event in click["awaitPumps"][0]], [True, False])
        shared_invalid = cases["toint-invalid"]["requests"][0]["expect"]
        self.assertEqual(shared_invalid["original"], shared_invalid["snake"])

    def test_index_fixture_keeps_extension_rejections_separate_from_successful_loads(self):
        fixture = driver.FIXTURE.with_name("fixture-snake-index-inputs")
        manifest = json.loads((fixture / "cases.json").read_text())
        cases = {case["id"]: case for case in manifest["cases"]}
        self.assertEqual(manifest["requiredRustPolicy"]["snake"],
                         {"semantic_version": 2, "policy_version": 2})
        self.assertEqual(manifest["loadExpect"]["result"]["termination"], "waitingInput")
        step = cases["index-user-alias-10-trim-first-wins"]["requests"][0]
        self.assertEqual(step["request"]["arguments"], '"alias"')
        self.assertEqual(step["expect"]["snake"]["result"]["watches"], {"RESULT:0": 110})
        self.assertIn("original", step["expectedRejection"])
        for case in cases.values():
            self.assertTrue(case["requireSuccessfulLoad"])
            for step in case["requests"]:
                self.assertNotIn("GETNUM", json.dumps(step["request"]))

    def test_index_primary_names_preserve_original_reference_success_and_rust_gap(self):
        fixture = driver.FIXTURE.with_name("fixture-snake-index-inputs")
        manifest = json.loads((fixture / "cases.json").read_text())
        cases = {case["id"]: case for case in manifest["cases"]}
        shared = cases["index-static-primary-names"]["requests"][0]
        self.assertEqual(shared["expect"]["original"], shared["expect"]["snake"])
        self.assertEqual(shared["expect"]["original"]["result"]["watches"],
                         {"RESULT:0": 110, "RESULT:1": 311, "RESULT:2": 600})
        for case_id in ["index-primary-name-precedes-alias", "index-column-primary", "index-matrix-primary-300"]:
            case = cases[case_id]
            self.assertEqual(case["rustCurrentPolicy"], "original_dynamic_user_index_existing_gap")
            self.assertEqual(case["knownRustDifference"]["original"]["status"], "existing_gap")
            step = case["requests"][0]
            self.assertNotIn("original", step.get("expectedRejection", {}))
            self.assertEqual(step["expect"]["original"]["result"]["termination"], "completed")
            self.assertEqual(step["expect"]["original"], step["expect"]["snake"])
            response = {**step["expect"]["original"], "diagnostics": []}
            rust = {"load": {"success": True}, "steps": [{
                "request": step["request"], "status": "executed",
                "result": {"ok": False, "termination": "faulted", "diagnostics": []},
            }]}
            compared = compare_case(case, [{"request": step["request"], "response": response}], rust)
            self.assertEqual(compared["status"], "different")


if __name__ == "__main__":
    unittest.main()
