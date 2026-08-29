"""Focused driver tests; run only after the batch review and static prerequisites."""

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace

from comparison import compare_case, output_after_load, split_setup_diagnostics, validate_rust_evidence
from recompare import recorded_steps


spec = importlib.util.spec_from_file_location("oracle_driver", Path(__file__).with_name("run.py"))
driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(driver)


class DriverTests(unittest.TestCase):
    def test_oracle_process_uses_each_case_working_directory_and_keeps_prior_records(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            args = SimpleNamespace(output=root, wine=None, exe=root / "reference.exe")
            with patch.object(driver.subprocess, "Popen") as popen, patch.object(driver.threading, "Thread"):
                first = driver.Oracle(args, "a" * 40, 0, root / "0000")
                second = driver.Oracle(args, "a" * 40, 0, root / "0001")
            try:
                self.assertEqual([call.kwargs["cwd"] for call in popen.call_args_list],
                                 [root / "0000", root / "0001"])
                first.stderr.write("first case")
                second.stderr.write("second case")
            finally:
                first.stderr.close()
                second.stderr.close()
            self.assertEqual((root / "stderr-0000.log").read_text(), "first case")
            self.assertEqual((root / "stderr-0001.log").read_text(), "second case")
        evidence = {}
        first_case = SimpleNamespace(case="first", records=[{"case": "first"}], close=lambda: None)
        second_case = SimpleNamespace(case="second", records=[{"case": "second"}], close=lambda: None)
        self.assertIsNone(driver.close_oracle(first_case, evidence, None))
        self.assertIsNone(driver.close_oracle(second_case, evidence, None))
        self.assertEqual(evidence["requests"], [{"case": "first"}, {"case": "second"}])

    def test_each_case_starts_with_pristine_files_and_independent_storage(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            template = root / "template"
            template.mkdir()
            (template / "emuera.config").write_text("Drawing interface:SKIASHARP\n")
            (template / "seed.txt").write_text("original")
            expected = driver.identity(template)
            first = driver.prepare_case_game(template, root, 0, expected)
            (first / "global.sav").write_bytes(b"saved in the first case")
            (first / "seed.txt").write_text("overlay")
            second = driver.prepare_case_game(template, root, 1, expected)
            self.assertEqual(driver.identity(second), expected)
            self.assertFalse((second / "global.sav").exists())
            self.assertEqual((first / "global.sav").read_bytes(), b"saved in the first case")
            with self.assertRaises(FileExistsError):
                driver.prepare_case_game(template, root, 1, expected)
            (template / "seed.txt").write_text("unexpected template change")
            with self.assertRaisesRegex(ValueError, "effective fixture changed"):
                driver.prepare_case_game(template, root, 2, expected)

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
        request = {"op": "run", "entry": "DIVIDE_ZERO", "watch": ["RESULT:10"]}
        case = {"id": "divide", "group": "arithmetic", "targetBatch": 2,
                "snakeTargetStatus": "deferred_semantics", "requests": [{"request": request}]}
        rust = {"steps": [{"request": request, "status": "executed", "result": {
            "ok": False, "termination": "faulted", "watches": {"RESULT:10": 777},
            "diagnostics": [{"code": "vm_fault"}]}}]}
        response = {"ok": True, "diagnostics": [], "result": {
            "termination": "error", "watches": {"RESULT:10": 777}}}
        result = compare_case(case, [{"request": request, "response": response}], rust)
        self.assertEqual(result["status"], "incomparable")
        self.assertEqual(result["steps"][0]["differences"], [])
        self.assertTrue(result["steps"][0]["oracleRequestAccepted"])
        self.assertEqual(result["steps"][0]["oracle"]["result"]["termination"], "error")
        self.assertEqual(result["steps"][0]["compared"], ["ok", "executionOutcome", "watches"])
        self.assertEqual(result["steps"][0]["rejectionComparison"]["status"], "matched_observed_rejection")
        self.assertEqual(result["steps"][0]["diagnosticComparison"]["status"], "incomparable_schema")
        actual = rust["steps"][0]["result"]
        for watches in ({"RESULT:10": 0}, {"RESULT:10": "777"}, {"RESULT:10": 777.0}, {}):
            with self.subTest(watches=watches):
                actual["watches"] = watches
                changed = compare_case(case, [{"request": request, "response": response}], rust)
                self.assertEqual(changed["status"], "different")
                self.assertEqual(changed["steps"][0]["differences"][0]["field"], "watches")
        actual.pop("watches")
        self.assertEqual(compare_case(case, [{"request": request, "response": response}], rust)["status"], "different")
        actual["watches"] = {"RESULT:10": 777}
        for termination in ("timeout", "instructionLimit", "quit", None):
            with self.subTest(termination=termination):
                actual["termination"] = termination
                changed = compare_case(case, [{"request": request, "response": response}], rust)
                self.assertEqual(changed["status"], "different")
                self.assertEqual(changed["steps"][0]["rejectionComparison"]["status"], "not_established")

    def test_rust_compile_error_and_oracle_run_error_preserve_the_rejection_stage_difference(self):
        request = {"op": "run", "entry": "PROFILE_GATE", "watch": ["FLAG:9"]}
        case = {"id": "gate", "group": "METHODS", "targetBatch": 2,
                "requireSuccessfulLoad": False, "snakeTargetStatus": "pending_actual_capture",
                "requests": [{"request": request}]}
        # Synthetic comparator inputs only. They do not declare the C# fixture's
        # unobserved load/run outcome or create a golden for either engine.
        diagnostic = {"code": "analyzer.unknowninstruction", "level": "error",
                      "source": {"relative_path": "methods.erb", "byte_start": 0, "byte_end": 20}}
        rust = {"load": {"success": False, "diagnostics": [diagnostic]}, "steps": [{
            "request": request, "status": "executed", "result": {"ok": False,
            "termination": "compileError", "output": [], "watches": {}, "diagnostics": [diagnostic]}}]}
        load = {"ok": True, "diagnostics": [{"level": 2,
                "position": {"file": "methods.erb", "line": 2}, "message": "localized load warning"}],
                "result": {"termination": "waitingInput", "output": ["ready"]}}
        response = {"ok": True, "diagnostics": [], "result": {
            "termination": "error", "output": ["ready", "localized execution error"], "watches": {"FLAG:9": 0}}}
        driver.validate_load(load)
        result = compare_case(case, [{"request": request, "response": response}], rust, load)
        self.assertEqual(result["status"], "different")
        self.assertEqual(result["oracleLoad"], load)
        step = result["steps"][0]
        outcome = next(item for item in step["differences"] if item["field"] == "executionOutcome")
        self.assertEqual((outcome["rust"], outcome["oracle"]), ("compileError", "script_error"))
        self.assertEqual(step["rejectionComparison"]["status"], "not_established")
        self.assertFalse(step["rejectionComparison"]["diagnosticEquivalence"])
        self.assertEqual(step["diagnosticComparison"]["status"], "incomparable_schema")
        self.assertEqual(step["diagnosticComparison"]["rust"], [diagnostic])
        self.assertEqual(step["rust"]["result"]["termination"], "compileError")
        self.assertEqual(step["oracle"]["result"]["termination"], "error")

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

    def test_offline_record_selection_validates_optional_capability_prefix(self):
        request = {"op": "eval", "source": "1"}
        case = {"id": "case", "requests": [{"request": request}]}
        envelope = {"ok": True, "schemaVersion": 2, "referenceCommit": "a" * 40}
        handshake = {"case": "case", "request": {"op": "capabilities"}, "response": {
            **envelope, "result": {"observationVersions": {
                "presentationSnapshot": 1, "headlessInputTrace": 1}}}}
        load = {"case": "case", "request": {"op": "load"}, "response": envelope}
        step = {"case": "case", "request": request, "response": envelope}
        evidence = {"semanticBaseline": "a" * 40, "requests": [handshake, load, step]}
        self.assertEqual(recorded_steps(evidence, case), (envelope, [step]))
        for records in ([handshake], [handshake, handshake, load, step],
                        [handshake, load, step, step], [load, handshake, step],
                        [{**handshake, "request": {"op": "reset"}}, load, step]):
            with self.subTest(records=records), self.assertRaises(ValueError):
                recorded_steps({**evidence, "requests": records}, case)
        for response in ({**envelope, "ok": False},
                         {**handshake["response"], "referenceCommit": "b" * 40},
                         {**handshake["response"], "schemaVersion": 3}):
            with self.subTest(response=response), self.assertRaises(AssertionError):
                recorded_steps({**evidence, "requests": [
                    {**handshake, "response": response}, load, step]}, case)

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
        for oracle, sample in (("original", evidence), ("snake", snake)):
            encoded = {**sample, "profile": {**sample["profile"]}}
            for field in ("semantic_version", "policy_version", "rng_state_version"):
                encoded["profile"][field] = str(encoded["profile"][field])
            validate_rust_evidence(encoded, oracle, fixture, 1, required if oracle == "snake" else None)
            self.assertIsInstance(encoded["profile"]["policy_version"], str)
            for invalid in (True, 1.0, "01", "+1", "1.0", " 1", "4294967296"):
                with self.subTest(oracle=oracle, invalid=invalid), self.assertRaises(ValueError):
                    validate_rust_evidence({**encoded, "profile": {**encoded["profile"], "rng_state_version": invalid}}, oracle, fixture, 1)

        historical = {**snake, "profile": {**snake["profile"], "semantic_version": 1, "policy_version": 1}}
        validate_rust_evidence(historical, "snake", fixture, 1)
        with self.assertRaises(ValueError):
            validate_rust_evidence(historical, "snake", fixture, 1, required)
        unknown = {**snake, "profile": {**snake["profile"], "policy_version": 3}}
        with self.assertRaises(ValueError):
            validate_rust_evidence(unknown, "snake", fixture, 1)
        current = {**snake, "profile": {**snake["profile"], "semantic_version": 3,
                   "policy_version": 3, "arithmetic": "snake_saturating_i64_v1"}}
        current_required = {"semantic_version": 3, "policy_version": 3,
                            "arithmetic": "snake_saturating_i64_v1"}
        validate_rust_evidence(current, "snake", fixture, 1, current_required)
        for fields in ({"arithmetic": "wrapping_i64_v1"}, {"rng_algorithm": "mt19937"},
                       {"rng_state_version": 2}, {"semantic_version": 6, "policy_version": 5}):
            with self.subTest(fields=fields), self.assertRaises(ValueError):
                validate_rust_evidence({**current, "profile": {**current["profile"], **fields}},
                                       "snake", fixture, 1)
        call_policy = {**current, "profile": {**current["profile"], "semantic_version": 4, "policy_version": 4}}
        validate_rust_evidence(call_policy, "snake", fixture, 1, {"semantic_version": 4, "policy_version": 4})
        restructure_policy = {**current, "profile": {**current["profile"], "semantic_version": 5, "policy_version": 5}}
        validate_rust_evidence(restructure_policy, "snake", fixture, 1, {"semantic_version": 5, "policy_version": 5})
        data_policy = {**current, "profile": {**current["profile"], "semantic_version": 6, "policy_version": 6}}
        validate_rust_evidence(data_policy, "snake", fixture, 1, {"semantic_version": 6, "policy_version": 6})
        with self.assertRaises(ValueError):
            validate_rust_evidence(call_policy, "snake", fixture, 1, {"semantic_version": 5, "policy_version": 5})
        with self.assertRaises(ValueError):
            validate_rust_evidence(call_policy, "snake", fixture, 1, current_required)
        with self.assertRaises(ValueError):
            validate_rust_evidence(current, "snake", fixture, 1, required)
        with self.assertRaises(ValueError):
            validate_rust_evidence({**historical, "profile": {**historical["profile"],
                                   "arithmetic": "snake_saturating_i64_v1"}}, "snake", fixture, 1)

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




class FrontendCaptureTests(unittest.TestCase):
    """Synthetic validator-input tests, never evidence of frontend measurement."""

    def make_capture(self, root):
        import copy
        import gzip
        import hashlib
        from frontend_capture_io import fixture_inventory, source_identity, frontend_files, project_payload_hashes

        fixture = root / "fixture"
        (fixture / "erb").mkdir(parents=True)
        (fixture / "erb/base.erb").write_text("@SYSTEM_TITLE\nINPUT\nRETURN\n")
        request = {"op": "run", "entry": "S04_CASE_EMPTY", "watch": ["RESULT:10"]}
        case = {"id": "s04-empty-lazy", "group": "SERVICES", "requests": [{"request": request,
                "expect": {"original": {"result": {"watches": {"RESULT:10": 999}}}}}]}
        plan = {"version": 1, "seed": 123456, "cases": [case]}
        (fixture / "cases.json").write_text(json.dumps(plan))
        profile = {"profile": "emuera.em", "semantic_version": 1, "policy_version": 1,
                   "arithmetic": "wrapping_i64_v1", "rng_algorithm": "sfmt19937",
                   "rng_state_version": 1, "layout": "unicode_column_v1",
                   "save_codec": "emuera1808", "services": []}
        inventory = fixture_inventory(fixture)
        artifacts = {}
        artifact_hashes = {}
        for role in ("runtime", "frontend", "client"):
            path = root / (role + ".synthetic-unit-input")
            payload = ("validator unit input " + role).encode()
            path.write_bytes(payload)
            artifacts[role] = path
            artifact_hashes[role] = {"bytes": len(payload), "sha256": hashlib.sha256(payload).hexdigest()}
        frontend_root = root / "frontend-root"
        (frontend_root / "src").mkdir(parents=True)
        (frontend_root / "src/main.ts").write_text("// validator input, not executed\n")
        (frontend_root / "package.json").write_text("{}")
        wasm_root = frontend_root / "public/wasm"
        wasm_root.mkdir(parents=True)
        wasm_hash, wasm_files = hashlib.sha256(), []
        for name in ("era_web_wasm.js", "era_web_wasm_bg.wasm"):
            payload = ("validator input, not executed: " + name).encode()
            (wasm_root / name).write_bytes(payload)
            wasm_hash.update(name.encode() + b"\0" + payload)
            wasm_files.append({"path": name, "bytes": len(payload),
                               "sha256": hashlib.sha256(payload).hexdigest()})
        wasm_assets = {"revision": wasm_hash.hexdigest(), "files": wasm_files}
        artifacts["runtime"] = wasm_root / "era_web_wasm_bg.wasm"
        artifact_hashes["runtime"] = {key: wasm_files[1][key] for key in ("bytes", "sha256")}
        source_manifest = {"version": 1, "kind": "frontend_source_manifest",
                           "files": frontend_files(frontend_root, "frontend_source_manifest")}
        source_bytes = json.dumps(source_manifest).encode()
        artifacts["frontend"].write_bytes(source_bytes)
        artifact_hashes["frontend"] = {"bytes": len(source_bytes), "sha256": hashlib.sha256(source_bytes).hexdigest()}
        identity = {"frontend": "browser", "coreSha": "a" * 40, "corePin": "a" * 40,
                    "wasmAssets": wasm_assets,
                    "frontendRuntime": {"mode": "vite-dev", "artifactRole": "frontend", "artifactKind": "source-manifest"},
                    "dirty": False, "frontendSha": "b" * 40, "frontendDirty": False,
                    "profile": profile, "seed": 123456, "fixtureInventory": inventory,
                    "sourceFixture": source_identity(inventory), "artifacts": artifact_hashes,
                    "submittedPayloads": [{"path": row["path"], "rawSha256": row["sha256"],
                                           "decodedUtf8Sha256": row["decodedUtf8Sha256"]}
                                          for row in inventory if row["path"].endswith(".erb")],
                    "provenance": {"synthetic": False, "captureMode": "real_client",
                                   "clientFamily": "chromium", "clientVersion": "unit-schema-only",
                                   "runtimeBackend": "wasm", "htmlProvider": "html_node_dom",
                                   "canvasProvider": "canvas_replay_renderer",
                                   "pointerProvider": "viewport_pointer"}}
        # A test can author these claims. The adapter explicitly does not call
        # hashes cryptographic proof of an executing real process.
        records = []
        def record(direction, channel, message_id, epoch, kind, value, sequence=None, correlation=None):
            item = {"index": len(records), "direction": direction, "channel": channel,
                    "messageId": message_id, "epoch": epoch, "message": {"type": kind, "value": value}}
            if sequence is not None:
                item["sequence"] = sequence
            if correlation is not None:
                item["correlationId"] = correlation
            records.append(item)
        record("receive", "runtime", 1, 0, "server_hello",
               {"epoch": 0, "selected_capabilities": {"services": []}}, sequence=1)
        record("receive", "runtime", 2, 0, "project_load_report",
               {"success": True, "compatibility": profile, "diagnostics": []}, sequence=2)
        record("send", "runtime", 10, 0, "start", {"mode": {"type": "new_game", "seed": 123456}})
        record("receive", "runtime", 3, 1, "runtime_state_changed", {"phase": "waiting_input"}, sequence=3)
        def snapshot(output):
            return {"bridgeKind": "browser", "runtimeEpoch": 1, "buildIdentity": {"corePin": "a" * 40, "wasmRevision": wasm_assets["revision"]},
                    "serviceEvidence": {"version": 1, "enabled": True, "overflow": False,
                                        "failure": None, "bytes": 0, "records": copy.deepcopy(records)},
                    "output": output, "phase": "waiting_input", "fault": None}
        loaded = snapshot(["S04_ORACLE_READY"])
        record("send", "runtime", 11, 1, "input", {"intent": {"type": "commit_text", "value": "1"}})
        stop = {"program_generation": 1, "session_epoch": 1}
        variable = {"name": "RESULT", "symbol_key": [1], "storage": "global", "dimensions": [100], "value_kind": "integer"}
        listing = {"type": "list_variables", "stop": stop, "cursor": None, "limit": 256}
        record("send", "debug", 12, 1, "request", {"grant": {"id": 1}, "command": listing})
        record("receive", "debug", 5, 1, "response",
               {"type": "variable_page", "value": {"stop": stop, "variables": [variable], "next_cursor": None}},
               sequence=1, correlation=12)
        reference = {"symbol_key": [1], "storage": "global", "fiber_id": None, "frame_id": None,
                     "generation": 1, "character": None, "indices": [10]}
        command = {"type": "read_variable", "stop": stop, "value": reference}
        value = {"type": "integer", "value": 0}
        response = {"type": "variable_value", "value": {"reference": reference, "value": value, "revision": 0}}
        record("send", "debug", 13, 1, "request", {"grant": {"id": 1}, "command": command})
        record("receive", "debug", 6, 1, "response", response, sequence=2, correlation=13)
        final = snapshot(["S04_ORACLE_READY", "S04_ENTRY_BEGIN", "S04_CASE_COMPLETE"])
        inspected = {"version": 1, "stop": stop, "values": {"RESULT:10": {
            "present": True, "value": value, "command": command, "response": response}}}
        project_hashes = project_payload_hashes(fixture, inventory)
        exported = []
        for name, hashes in project_hashes.items():
            if name.endswith(".erb"):
                exported.append({"relativePath": name, "category": "erb", "payloadKind": "utf8",
                                 "contentHash": hashes["decodedUtf8Blake3"], "byteLength": hashes["decodedUtf8Bytes"]})
        identity_snapshot = copy.deepcopy(final)
        identity_snapshot["lastDownload"] = {"projectIdentityFiles": exported}
        packets = [{"type": "header", "identity": identity, "caseIds": [case["id"]]},
                   {"type": "case_begin", "case": case["id"], "requests": [request]},
                   {"type": "observation", "case": case["id"], "stage": "loaded", "snapshot": loaded},
                   {"type": "observation", "case": case["id"], "stage": "complete", "snapshot": final,
                    "request": request, "inspect": inspected},
                   {"type": "observation", "case": case["id"], "stage": "identity", "snapshot": identity_snapshot},
                   {"type": "case_end", "case": case["id"], "captureComplete": True},
                   {"type": "footer", "captureComplete": True}]
        capture = {"version": 1, "kind": "rustyera_real_frontend_capture", "identity": identity,
                   "caseIds": [case["id"]]}
        def write():
            lines = []
            for index, packet in enumerate(packets):
                lines.append(json.dumps({"index": index, **packet}).encode() + b"\n")
            raw = b"".join(lines)
            stored = gzip.compress(raw, mtime=0)
            (root / "trace.ndjson.gz").write_bytes(stored)
            capture["trace"] = {"path": "trace.ndjson.gz", "compression": "gzip",
                                "storedBytes": len(stored), "storedSha256": hashlib.sha256(stored).hexdigest(),
                                "decodedBytes": len(raw), "decodedSha256": hashlib.sha256(raw).hexdigest()}
            path = root / "capture.json"
            path.write_text(json.dumps(capture))
            return path
        return fixture, artifacts, capture, packets, write

    def test_actual_typed_value_is_not_filled_from_fixture_expectation(self):
        from frontend_capture import build_evidence
        with tempfile.TemporaryDirectory() as directory:
            fixture, artifacts, _, _, write = self.make_capture(Path(directory))
            evidence = build_evidence(write(), fixture, artifacts, Path(directory) / "frontend-root")
            self.assertEqual(evidence["cases"][0]["steps"][0]["result"]["watches"], {"RESULT:10": 0})
            self.assertEqual(evidence["frontendCapture"]["status"], "validated_observations_not_comparison_verdict")

    def test_source_and_fixture_inventory_match_full_js_path_order(self):
        from frontend_capture_io import frontend_files, fixture_inventory
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            names = ["package.json", "src/audio.ts", "src/audio/item.ts", "src/\U00010000.ts", "src/\ue000.ts"]
            for name in reversed(names):
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("{}")
            self.assertEqual([item["path"] for item in frontend_files(root, "frontend_source_manifest")], names)
            self.assertEqual([item["path"] for item in fixture_inventory(root)], names)

    def test_frontend_decimal_policy_versions_preserve_the_raw_capture_identity(self):
        from frontend_capture import build_evidence
        with tempfile.TemporaryDirectory() as directory:
            fixture, artifacts, capture, packets, write = self.make_capture(Path(directory))
            def encode_versions(value):
                if isinstance(value, dict):
                    for key, child in value.items():
                        if key in ("semantic_version", "policy_version", "rng_state_version"):
                            value[key] = str(child)
                        else:
                            encode_versions(child)
                elif isinstance(value, list):
                    for child in value:
                        encode_versions(child)
            encode_versions(capture)
            encode_versions(packets)
            evidence = build_evidence(write(), fixture, artifacts, Path(directory) / "frontend-root")
            self.assertEqual(evidence["profile"]["policy_version"], "1")
            validate_rust_evidence(evidence, "original", evidence["sourceFixture"], 123456)

    def test_capture_rejects_hash_identity_order_and_provenance_tampering(self):
        from frontend_capture import build_evidence
        mutations = [
            lambda capture, packets: capture["identity"].update(coreSha="a" * 39),
            lambda capture, packets: capture["identity"].update(dirty="false"),
            lambda capture, packets: capture["identity"].update(seed=1),
            lambda capture, packets: capture["identity"]["provenance"].update(synthetic=True),
            lambda capture, packets: capture["identity"]["fixtureInventory"][0].update(sha256="0" * 64),
            lambda capture, packets: packets[1]["requests"][0].update(entry="UNRELATED"),
            lambda capture, packets: packets[3]["snapshot"]["serviceEvidence"].update(overflow=True),
            lambda capture, packets: packets[4]["snapshot"]["lastDownload"]["projectIdentityFiles"][0].update(contentHash="0" * 64),
            lambda capture, packets: packets[3]["snapshot"]["serviceEvidence"]["records"][0].update(epoch=12),
            lambda capture, packets: packets[3]["inspect"]["values"]["RESULT:10"].update(value={"type": "integer", "value": 55}),
        ]
        for mutate in mutations:
            with self.subTest(mutation=mutate), tempfile.TemporaryDirectory() as directory:
                fixture, artifacts, capture, packets, write = self.make_capture(Path(directory))
                mutate(capture, packets)
                with self.assertRaises((ValueError, KeyError)):
                    build_evidence(write(), fixture, artifacts, Path(directory) / "frontend-root")

    def test_truncated_trace_or_changed_artifact_never_emits_complete_evidence(self):
        from frontend_capture import build_evidence
        for mode in ("footer", "stored_hash", "artifact"):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                fixture, artifacts, capture, packets, write = self.make_capture(Path(directory))
                if mode == "footer":
                    packets.pop()
                path = write()
                if mode == "stored_hash":
                    capture["trace"]["storedSha256"] = "0" * 64
                    path.write_text(json.dumps(capture))
                if mode == "artifact":
                    artifacts["runtime"].write_bytes(b"changed")
                with self.assertRaises(ValueError):
                    build_evidence(path, fixture, artifacts, Path(directory) / "frontend-root")

    def test_returned_debug_references_allow_wire_integer_and_omitted_option_encoding_only(self):
        from frontend_capture import build_evidence
        for mode in ("same", "index", "generation", "fiber"):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                fixture, artifacts, _, packets, write = self.make_capture(root)
                def encode_response(value):
                    if isinstance(value, dict):
                        if value.get("type") == "variable_value":
                            reference = dict(value["value"]["reference"])
                            for field in ("fiber_id", "frame_id", "character"):
                                reference.pop(field, None)
                            reference["indices"] = ["11" if mode == "index" else "10"]
                            reference["generation"] = "2" if mode == "generation" else "1"
                            reference["symbol_key"] = [str(byte) for byte in reference["symbol_key"]]
                            if mode == "fiber":
                                reference["fiber_id"] = "1"
                            value["value"]["reference"] = reference
                        else:
                            for child in value.values():
                                encode_response(child)
                    elif isinstance(value, list):
                        for child in value:
                            encode_response(child)
                encode_response(packets)
                path = write()
                if mode == "same":
                    evidence = build_evidence(path, fixture, artifacts, root / "frontend-root")
                    self.assertEqual(evidence["cases"][0]["steps"][0]["result"]["watches"], {"RESULT:10": 0})
                else:
                    with self.assertRaisesRegex(ValueError, "typed watch value differs"):
                        build_evidence(path, fixture, artifacts, root / "frontend-root")

    def test_watch_reply_pair_only_normalizes_omitted_reference_options(self):
        from copy import deepcopy
        from frontend_capture import build_evidence
        modes = ("same", "reverse", "index", "fiber_id", "frame_id", "character",
                 "value", "value_encoding", "revision", "correlation", "epoch")
        for mode in modes:
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                fixture, artifacts, _, packets, write = self.make_capture(root)
                item = packets[3]["inspect"]["values"]["RESULT:10"]
                item["response"] = deepcopy(item["response"])
                if mode == "reverse":
                    for field in ("fiber_id", "frame_id", "character"):
                        item["response"]["value"]["reference"].pop(field)
                for packet in (packets[3], packets[4]):
                    for row in packet["snapshot"]["serviceEvidence"]["records"]:
                        response = row["message"].get("value", {})
                        if response.get("type") != "variable_value":
                            continue
                        value = response["value"]
                        # make_capture shares the command/response reference in memory;
                        # real wire JSON has independent objects. Mutate only the reply.
                        reference = dict(value["reference"])
                        value["reference"] = reference
                        if mode != "reverse":
                            for field in ("fiber_id", "frame_id", "character"):
                                reference.pop(field)
                        if mode == "index":
                            reference["indices"] = [11]
                        elif mode in ("fiber_id", "frame_id", "character"):
                            reference[mode] = 1
                        elif mode == "value":
                            value["value"] = {"type": "integer", "value": 1}
                        elif mode == "value_encoding":
                            value["value"] = {"type": "integer", "value": "0"}
                        elif mode == "revision":
                            value["revision"] = 1
                        elif mode == "correlation":
                            row["correlationId"] = 99
                        elif mode == "epoch":
                            row["epoch"] = 2
                if mode in ("same", "reverse"):
                    evidence = build_evidence(write(), fixture, artifacts, root / "frontend-root")
                    self.assertEqual(evidence["cases"][0]["steps"][0]["result"]["watches"],
                                     {"RESULT:10": 0})
                else:
                    with self.assertRaises(ValueError):
                        build_evidence(write(), fixture, artifacts, root / "frontend-root")

    def test_missing_typed_inspection_stays_blocked(self):
        from frontend_capture import build_evidence
        with tempfile.TemporaryDirectory() as directory:
            fixture, artifacts, _, packets, write = self.make_capture(Path(directory))
            packets[3].pop("inspect")
            evidence = build_evidence(write(), fixture, artifacts, Path(directory) / "frontend-root")
            self.assertEqual(evidence["cases"][0]["steps"][0]["status"], "blocked")
            self.assertEqual(evidence["cases"][0]["steps"][0]["result"]["watches"], {})

    def test_wasm_asset_revision_is_distinct_from_core_sha_and_checks_both_files(self):
        from frontend_capture import build_evidence
        for mode in ("core_sha", "wrapper", "runtime_path", "source_manifest"):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                fixture, artifacts, _, packets, write = self.make_capture(root)
                if mode == "core_sha":
                    packets[2]["snapshot"]["buildIdentity"]["wasmRevision"] = "a" * 40
                elif mode == "wrapper":
                    (root / "frontend-root/public/wasm/era_web_wasm.js").write_text("changed wrapper")
                elif mode == "runtime_path":
                    replacement = root / "copied-but-not-loaded.wasm"
                    replacement.write_bytes(artifacts["runtime"].read_bytes())
                    artifacts["runtime"] = replacement
                else:
                    (root / "frontend-root/src/main.ts").write_text("changed source")
                with self.assertRaises(ValueError):
                    build_evidence(write(), fixture, artifacts, root / "frontend-root")

    def test_later_identity_export_cannot_supply_completion_debug_replies(self):
        from frontend_capture import build_evidence
        with tempfile.TemporaryDirectory() as directory:
            fixture, artifacts, _, packets, write = self.make_capture(Path(directory))
            # Keep only load + menu input at completion. The later identity
            # snapshot still has all debug replies and must not fill this gap.
            packets[3]["snapshot"]["serviceEvidence"]["records"] = (
                packets[3]["snapshot"]["serviceEvidence"]["records"][:5])
            with self.assertRaises(ValueError):
                build_evidence(write(), fixture, artifacts, Path(directory) / "frontend-root")

    def test_raw_and_decoded_utf8_hashes_do_not_conflate_a_bom(self):
        from frontend_capture_io import fixture_inventory
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "source.erb").write_bytes(b"\xef\xbb\xbf@A\nRETURN\n")
            item = fixture_inventory(root)[0]
            self.assertNotEqual(item["sha256"], item["decodedUtf8Sha256"])

    def test_strict_cbor_rejects_duplicate_keys_trailing_data_tags_and_depth(self):
        import cbor2
        from frontend_capture_io import decode_payload
        payloads = [bytes.fromhex("a200010002"), bytes.fromhex("a000"), bytes.fromhex("c20101"),
                    bytes.fromhex("9fff"), b"\x81" * 129 + b"\x00"]
        for payload in payloads:
            with self.subTest(payload=payload[:20]), self.assertRaises((ValueError, cbor2.CBORDecodeError)):
                decode_payload(list(payload))

    def test_service_context_probe_unicode_cut_and_versions_are_checked(self):
        import cbor2
        from frontend_capture_services import check_pair, negotiated
        context = {0: 10, 1: 11, 2: 12}
        document = {0: [[0, ["😀", 0, 4]]]}
        query = {0: context, 1: {}, 2: [{0: 3, 1: document, 2: 0,
                                      3: [{0: 7, 1: [0], 2: 4, 3: 2}]}]}
        answer = {0: context, 1: [{0: 3, 1: [0, [1000, [{0: 7, 1: 1000}]]]}]}
        request = {"operation": "html_string_len", "payload": list(cbor2.dumps(query))}
        response = {"result": {"type": "ready", "payload": list(cbor2.dumps(answer))}}
        self.assertEqual(check_pair(request, response), "ready")
        for cut in ({0: 7, 1: [0], 2: 1, 3: 1}, {0: 7, 1: [0], 2: 4, 3: 1}):
            query[2][0][3] = [cut]
            request["payload"] = list(cbor2.dumps(query))
            with self.assertRaises(ValueError):
                check_pair(request, response)
        with self.assertRaises(ValueError):
            negotiated({"services": [{"kind": "presentation_query", "operation": "html_string_len",
                        "versions": {"minimum": {"major": 1, "minor": 0},
                                     "maximum": {"major": 1, "minor": 0}}}]})

    def test_canvas_reply_revision_and_argb_are_not_invented_or_clamped(self):
        import cbor2
        from frontend_capture_services import check_pair
        request = {"operation": "sample_canvas_pixel", "payload": list(cbor2.dumps({
            0: {0: 1, 1: 2, 2: 3}, 1: 751, 2: 9, 3: {0: 0, 1: 0}}))}
        for context, revision, argb in (({0: 1, 1: 2, 2: 4}, 9, 0),
                                        ({0: 1, 1: 2, 2: 3}, 8, 0),
                                        ({0: 1, 1: 2, 2: 3}, 9, -1)):
            response = {"result": {"type": "ready", "payload": list(cbor2.dumps({
                0: context, 1: revision, 2: argb}))}}
            with self.assertRaises(ValueError):
                check_pair(request, response)

    def test_actual_payload_inventory_does_not_accept_raw_bom_hash_as_utf8(self):
        from frontend_capture_io import fixture_inventory, project_payload_hashes, validate_project_files
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "case.erb").write_bytes(b"\xef\xbb\xbf@A\nRETURN\n")
            hashes = project_payload_hashes(root, fixture_inventory(root))
            row = hashes["case.erb"]
            exported = [{"relativePath": "case.erb", "category": "erb", "payloadKind": "utf8",
                         "contentHash": row["rawBlake3"], "byteLength": row["rawBytes"]}]
            with self.assertRaises(ValueError):
                validate_project_files(exported, hashes, ["case.erb"])
            exported[0].update(contentHash=row["decodedUtf8Blake3"], byteLength=row["decodedUtf8Bytes"])
            self.assertEqual(validate_project_files(exported, hashes, ["case.erb"]), exported)

    def test_json_duplicate_keys_and_fixture_symlinks_are_rejected(self):
        from frontend_capture_io import fixture_inventory, json_bytes
        with self.assertRaises(ValueError):
            json_bytes(b'{"version":1,"version":2}')
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "data.txt").write_text("data")
            (root / "link.txt").symlink_to(root / "data.txt")
            with self.assertRaises(ValueError):
                fixture_inventory(root)


if __name__ == "__main__":
    unittest.main()
