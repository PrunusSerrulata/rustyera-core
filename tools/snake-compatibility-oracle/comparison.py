"""Compare actual observations without turning deferred differences into passes."""

PROFILES = {"original": "emuera.em", "snake": "emuera.skia.snake"}
OPERATION_FIELDS = {
    "eval": ("value",),
    "run": ("termination", "output", "watches"),
    "execute": ("termination", "output", "watches"),
}


def validate_rust_evidence(evidence, oracle, fixture, seed):
    if evidence.get("version") != 1:
        raise ValueError("unsupported Rust evidence version")
    identity = evidence.get("profile", {})
    if identity.get("profile") != PROFILES[oracle]:
        raise ValueError("Rust evidence belongs to a different compatibility profile")
    expected = {
        "semantic_version": 1,
        "policy_version": 1,
        "arithmetic": "wrapping_i64_v1",
        "rng_algorithm": "sfmt19937",
        "rng_state_version": 1,
        "layout": "unicode_column_v1",
        "save_codec": ("emuera1808" if oracle == "original" else "rustyera_envelope_v1:emuera1808"),
        "services": [],
    }
    for key, value in expected.items():
        if identity.get(key) != value:
            raise ValueError(f"unsupported Rust policy {key}: {identity.get(key)!r}")
    sha = evidence.get("coreSha", "")
    if len(sha) != 40 or any(char not in "0123456789abcdef" for char in sha):
        raise ValueError("Rust evidence requires a full core SHA")
    if not isinstance(evidence.get("dirty"), bool):
        raise ValueError("Rust evidence must disclose its working-tree state")
    if evidence.get("seed") != seed:
        raise ValueError("Rust/oracle seed differs")
    source = evidence.get("sourceFixture", {})
    # The per-file hashes are authoritative; JSON serialization of an aggregate
    # hash can differ across runtimes without changing a single source byte.
    if source.get("files") != fixture["files"]:
        raise ValueError("Rust/oracle source fixture differs")
    cases = evidence.get("cases")
    if not isinstance(cases, list) or len({case["id"] for case in cases}) != len(cases):
        raise ValueError("Rust evidence has missing or duplicate cases")


def compare_case(case, oracle_steps, rust_case):
    if rust_case is None:
        raise ValueError(f"Rust evidence missing case {case['id']}")
    if not case["requests"] and rust_case.get("status") == "blocked":
        return {
            "case": case["id"],
            "status": "blocked",
            "steps": [],
            "reason": rust_case.get("reason"),
            "targetBatch": case["targetBatch"],
            "snakeTargetStatus": case["snakeTargetStatus"],
        }
    steps = rust_case.get("steps", [])
    requests = case["requests"]
    if len(steps) != len(requests) or len(oracle_steps) != len(requests):
        raise ValueError(f"incomplete observations for {case['id']}")
    results = []
    for index, (planned, oracle, rust) in enumerate(zip(requests, oracle_steps, steps)):
        request = planned["request"]
        if rust.get("request") != request or oracle["request"] != request:
            raise ValueError(f"different input for {case['id']} step {index}")
        if rust.get("status") == "blocked":
            results.append(
                {
                    "step": index,
                    "status": "blocked",
                    "reason": rust.get("reason"),
                    "rust": rust.get("result"),
                    "oracle": oracle["response"],
                }
            )
            continue
        if rust.get("status") != "executed":
            raise ValueError(f"unobserved Rust step {case['id']}:{index}")
        response = oracle["response"]
        actual = rust["result"]
        expected = {
            "ok": response["ok"],
            **response.get("result", {}),
            "diagnostics": response["diagnostics"],
        }
        operation = request["op"]
        if operation not in OPERATION_FIELDS:
            raise ValueError(f"unsupported comparison operation {operation}")
        # Eval executes a generated wrapper in Rust; its output/termination are
        # harness state, not observable fields of the oracle's eval endpoint.
        fields = ["ok"]
        if actual.get("ok") and expected.get("ok"):
            fields.extend(OPERATION_FIELDS[operation])
        compared, differences = [], []
        for field in fields:
            compared.append(field)
            if field == "output":
                for value in (actual.get(field), expected.get(field)):
                    if not isinstance(value, list) or not all(isinstance(line, str) for line in value):
                        raise ValueError("output observations must be arrays of logical lines")
            if field not in actual or field not in expected or actual[field] != expected[field]:
                differences.append(
                    {
                        "field": field,
                        "rustPresent": field in actual,
                        "oraclePresent": field in expected,
                        "rust": actual.get(field),
                        "oracle": expected.get(field),
                    }
                )
        # Neither localized oracle diagnostics nor Rust protocol faults have a
        # shared semantic schema yet. Empty streams are comparable; otherwise
        # retain the evidence and explicitly decline a parity claim.
        diagnostics = {
            "rust": actual.get("diagnostics"),
            "oracle": expected.get("diagnostics"),
            "oracleError": response.get("error"),
        }
        diagnostic_incomparable = (
            diagnostics["rust"] != [] or diagnostics["oracle"] != []
            or not actual.get("ok") or not expected.get("ok")
        )
        if not diagnostic_incomparable:
            compared.append("diagnostics")
        result = {
            "step": index,
            "status": "different" if differences else "incomparable" if diagnostic_incomparable else "matched_observables",
            "compared": compared,
            "differences": differences,
            "diagnosticComparison": {
                "status": "incomparable_schema" if diagnostic_incomparable else "matched_empty",
                **diagnostics,
            },
            "rust": rust,
            "oracle": response,
        }
        if case["group"] == "PRINTC":
            # Core column projection and the oracle's pixel tree are distinct
            # policies. Preserve both; a text match cannot prove layout parity.
            result["layout"] = {
                "status": "different_policy",
                "targetBatch": 4,
                "rust": actual.get("presentation"),
                "oracle": expected.get("presentation"),
            }
            result["status"] = "different"
        results.append(result)
    statuses = {result["status"] for result in results}
    status = (
        "blocked"
        if "blocked" in statuses
        else "different"
        if "different" in statuses
        else "incomparable"
        if "incomparable" in statuses
        else "matched_observables"
        if results
        else "oracle_instrumentation_only"
    )
    return {
        "case": case["id"],
        "status": status,
        "steps": results,
        "targetBatch": case["targetBatch"],
        "snakeTargetStatus": case["snakeTargetStatus"],
    }
