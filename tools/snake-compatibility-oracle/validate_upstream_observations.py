#!/usr/bin/env python3
"""Accept one fixed upstream B observation, retaining any registered raw difference."""

import argparse
import hashlib
import json
from pathlib import Path

from comparison import same_json_value


def validate(evidence, manifest):
    if evidence.get("status") != "completed_observations":
        raise ValueError("incomplete oracle capture")
    cases = evidence.get("rustComparison", {}).get("cases", [])
    if len(cases) != 1:
        raise ValueError("validate exactly one case before executing the next")
    actual = cases[0]
    planned = next(case for case in manifest["cases"] if case["id"] == actual["case"])
    if planned.get("group") != "UPSTREAM_B":
        raise ValueError("not an upstream B fixture")
    oracle = evidence["oracle"]
    if oracle not in planned["allowedOracles"]:
        raise ValueError("case belongs to another oracle")
    if evidence["semanticBaseline"] != manifest["semanticBaselines"][oracle]:
        raise ValueError("incorrect semantic baseline")
    if actual.get("loadDiagnosticComparison", {}).get("status") != "separately_checked_schemas":
        raise ValueError("load diagnostics were not accepted")
    if len(actual["steps"]) != 1:
        raise ValueError("expected one operation")
    step = actual["steps"][0]
    rust = step["rust"]["result"]
    reference = step["oracle"]["result"]
    if rust.get("termination") != "completed" or reference.get("termination") != "completed":
        raise ValueError("operation did not complete")
    if step.get("diagnosticComparison", {}).get("status") != "matched_empty":
        raise ValueError("unexpected operation diagnostics")
    if not isinstance(rust.get("watches"), dict) or not isinstance(reference.get("watches"), dict):
        raise ValueError("missing operation watches")
    expected_widths = planned.get("recordedProviderWidthExpectations", {}).get(oracle, {})
    for key, expected in expected_widths.items():
        for result in (rust, reference):
            if not same_json_value(result["watches"].get(key), expected):
                raise ValueError(f"recorded provider width differs: {key}")
    replacement = planned.get("intentionalUtf16Replacement")
    if replacement:
        if oracle not in {"original", "snake"} or replacement.get("rust") != 1 or replacement.get("oracle") != 0:
            raise ValueError("unrecognized replacement contract")
        key = replacement["watch"]
        expected = [{"field": "watches", "rustPresent": True, "oraclePresent": True,
                     "rust": {key: 1}, "oracle": {key: 0}}]
        if (actual["status"] != "different" or step["status"] != "different"
                or not same_json_value(step["differences"], expected)
                or not same_json_value(rust["watches"], {key: 1})
                or not same_json_value(reference["watches"], {key: 0})):
            raise ValueError("difference exceeds the registered lone-surrogate replacement")
        status = "accepted_registered_difference"
    else:
        if actual["status"] != "matched_observables" or step["status"] != "matched_observables":
            raise ValueError("unregistered observable difference")
        status = "matched_observables"
    return {"case": actual["case"], "oracle": oracle, "status": status,
            "rawVerdict": actual["status"], "registeredDifference": replacement}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    raw = args.evidence.read_bytes()
    result = validate(json.loads(raw), json.loads((args.fixture / "cases.json").read_text()))
    result["evidenceSha256"] = hashlib.sha256(raw).hexdigest()
    result["validatorSha256"] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    with args.output.open("x", encoding="utf-8") as output:
        json.dump(result, output, indent=2)
        output.write("\n")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
