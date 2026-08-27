"""Focused driver tests; run only after the batch review and static prerequisites."""

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

from comparison import compare_case, validate_rust_evidence


spec = importlib.util.spec_from_file_location("oracle_driver", Path(__file__).with_name("run.py"))
driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(driver)


class DriverTests(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
