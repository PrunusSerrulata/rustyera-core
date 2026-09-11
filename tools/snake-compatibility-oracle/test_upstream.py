"""Fixed B acceptance gates consume the real comparator's exact report schema."""

import copy
import unittest

from comparison import compare_case
from validate_upstream_observations import validate


class UpstreamAcceptanceTests(unittest.TestCase):
    def sample(self, replacement=False, mutate=None):
        request = {"op": "run", "entry": "SAMPLE", "watch": ["RESULT:10"]}
        case = {"id": "sample", "group": "UPSTREAM_B", "allowedOracles": ["original"],
                "targetBatch": "upstream-B", "snakeTargetStatus": "unchanged_regression",
                "requireSuccessfulLoad": True, "requests": [{"request": request}]}
        rust = {"ok": True, "termination": "completed", "output": [],
                "watches": {"RESULT:10": 1}, "diagnostics": []}
        response = {"ok": True, "diagnostics": [], "result": {
            "termination": "completed", "output": [], "watches": {"RESULT:10": 1}}}
        if replacement:
            case["intentionalUtf16Replacement"] = {"watch": "RESULT:10", "rust": 1, "oracle": 0}
            response["result"]["watches"]["RESULT:10"] = 0
        if mutate:
            mutate(rust, response)
        actual = compare_case(
            case, [{"request": request, "response": response}],
            {"load": {"success": True, "diagnostics": []}, "steps": [
                {"request": request, "status": "executed", "result": rust}]},
            {"ok": True, "diagnostics": [], "result": {"output": []}},
            {"profile": "emuera.em", "semantic_version": 3, "policy_version": 3},
        )
        manifest = {"cases": [case], "semanticBaselines": {"original": "baseline"}}
        evidence = {"status": "completed_observations", "oracle": "original",
                    "semanticBaseline": "baseline", "rustComparison": {"cases": [actual]}}
        return evidence, manifest

    def test_matching_case_checks_recorded_widths_and_completion(self):
        evidence, manifest = self.sample()
        manifest["cases"][0]["recordedProviderWidthExpectations"] = {"original": {"RESULT:10": 1}}
        self.assertEqual(validate(evidence, manifest)["status"], "matched_observables")
        # Both engines can agree on a wrong value. The fixed provider assertion
        # must still reject it independently of the comparator's matched verdict.
        for value in (2, True, None):
            def change_both(rust, response):
                rust["watches"]["RESULT:10"] = value
                response["result"]["watches"]["RESULT:10"] = value
            changed, _ = self.sample(mutate=change_both)
            with self.assertRaises(ValueError):
                validate(changed, manifest)

    def test_registered_surrogate_difference_retains_the_raw_verdict(self):
        evidence, manifest = self.sample(True)
        difference = evidence["rustComparison"]["cases"][0]["steps"][0]["differences"][0]
        self.assertIs(difference["rustPresent"], True)
        self.assertIs(difference["oraclePresent"], True)
        result = validate(evidence, manifest)
        self.assertEqual(result["status"], "accepted_registered_difference")
        self.assertEqual(result["rawVerdict"], "different")
        for field in ("rustPresent", "oraclePresent"):
            changed = copy.deepcopy(evidence)
            del changed["rustComparison"]["cases"][0]["steps"][0]["differences"][0][field]
            with self.assertRaises(ValueError):
                validate(changed, manifest)
        evidence["rustComparison"]["cases"][0]["status"] = "matched_observables"
        with self.assertRaises(ValueError):
            validate(evidence, manifest)

    def test_extra_output_or_watch_difference_is_never_registered(self):
        mutations = [
            lambda rust, response: response["result"]["output"].append("unexpected"),
            lambda rust, response: rust["watches"].update({"RESULT:11": 7}),
            lambda rust, response: rust["watches"].update({"RESULT:10": 2}),
        ]
        for replacement in (False, True):
            for mutate in mutations:
                evidence, manifest = self.sample(replacement, mutate)
                with self.assertRaises(ValueError):
                    validate(evidence, manifest)

    def test_missing_fields_wrong_termination_and_diagnostics_are_rejected(self):
        mutations = [
            lambda rust, response: rust.pop("watches"),
            lambda rust, response: response["result"].pop("watches"),
            lambda rust, response: rust["watches"].pop("RESULT:10"),
            lambda rust, response: response["result"]["watches"].pop("RESULT:10"),
            lambda rust, response: rust.pop("termination"),
            lambda rust, response: response["result"].update({"termination": "error"}),
            lambda rust, response: rust["diagnostics"].append({"code": "unexpected", "level": "warning"}),
            lambda rust, response: response["diagnostics"].append({"level": 1, "message": "unexpected"}),
        ]
        for replacement in (False, True):
            for mutate in mutations:
                evidence, manifest = self.sample(replacement, mutate)
                with self.assertRaises(ValueError):
                    validate(evidence, manifest)


if __name__ == "__main__":
    unittest.main()
