#!/usr/bin/env python3
"""Recompare immutable, completed observations without reexecuting either engine."""

import argparse
import hashlib
import json
from pathlib import Path

from comparison import compare_case, validate_rust_evidence
from run import FIXTURE, identity, subset, validate_load


def recorded_steps(evidence, case, load_expect=None):
    records = [record for record in evidence["requests"] if record["case"] == case["id"]]
    if not records or records[0]["request"].get("op") != "load":
        raise ValueError(f"missing initial load for {case['id']}")
    load = records[0]["response"]
    validate_load(load, load_expect)
    count = len(case["requests"])
    steps = records[1:1 + count]
    if len(steps) != count:
        raise ValueError(f"missing steps for {case['id']}")
    for planned, recorded in zip(case["requests"], steps):
        if planned["request"] != recorded["request"]:
            raise ValueError(f"recorded input differs for {case['id']}")
    # Validate actual protocol envelopes before applying any normalization.
    for record in records:
        subset(record["response"], {"schemaVersion": 2, "referenceCommit": evidence["semanticBaseline"]})
    return load, steps


def recompare(evidence, rust, manifest, fixture):
    if evidence.get("version") != 1 or evidence.get("status") != "completed_observations":
        raise ValueError("only completed version-1 oracle observations can be recomputed")
    oracle = evidence["oracle"]
    if evidence["semanticBaseline"] != manifest["semanticBaselines"][oracle]:
        raise ValueError("oracle semantic baseline differs")
    if evidence["sourceFixture"]["files"] != fixture["files"] or evidence["seed"] != manifest["seed"]:
        raise ValueError("oracle fixture/seed differs")
    validate_rust_evidence(rust, oracle, fixture, manifest["seed"],
                           manifest.get("requiredRustPolicy", {}).get(oracle))
    planned = {case["id"]: case for case in manifest["cases"]}
    observed = {case["id"]: case for case in rust["cases"]}
    completed = evidence["cases"]
    if len({case["id"] for case in completed}) != len(completed):
        raise ValueError("duplicate completed oracle case")
    comparisons = []
    for completed_case in completed:
        case = planned[completed_case["id"]]
        load, steps = recorded_steps(evidence, case, manifest.get("loadExpect"))
        comparisons.append(compare_case(case, steps, observed.get(case["id"]), load, rust["profile"]))
    return {
        "version": 2, "status": "recompared_observations", "oracle": oracle,
        "semanticBaseline": evidence["semanticBaseline"], "wrapperSha": evidence["wrapperSha"],
        "seed": evidence["seed"], "sourceFixture": fixture,
        "drawingMode": evidence["drawingMode"],
        "rust": {key: rust[key] for key in ["coreSha", "dirty", "profile"]},
        "cases": completed, "rustComparison": {"status": "compared", "cases": comparisons},
        "normalization": "exact captured load prefix; exact configuration-profile warning retained separately; raw responses unchanged",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", type=Path, default=FIXTURE)
    parser.add_argument("--oracle-evidence", type=Path, required=True)
    parser.add_argument("--rust-evidence", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    oracle_bytes = args.oracle_evidence.read_bytes()
    rust_bytes = args.rust_evidence.read_bytes()
    result = recompare(json.loads(oracle_bytes), json.loads(rust_bytes),
                       json.loads((args.fixture / "cases.json").read_text()), identity(args.fixture))
    result["provenance"] = {
        "oracle": {"path": str(args.oracle_evidence.resolve()), "sha256": hashlib.sha256(oracle_bytes).hexdigest()},
        "rust": {"path": str(args.rust_evidence.resolve()), "sha256": hashlib.sha256(rust_bytes).hexdigest()},
        "comparatorSources": {name: hashlib.sha256(Path(__file__).with_name(name).read_bytes()).hexdigest()
                              for name in ("comparison.py", "recompare.py")},
    }
    with args.output.open("x", encoding="utf-8") as stream:
        json.dump(result, stream, ensure_ascii=False, indent=2)
        stream.write("\n")
    print(json.dumps({"output": str(args.output), "cases": len(result["cases"]), "status": result["status"]}))


if __name__ == "__main__":
    main()
