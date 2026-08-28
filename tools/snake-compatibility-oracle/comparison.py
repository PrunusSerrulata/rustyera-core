"""Compare actual observations without turning deferred differences into passes."""

import re


def policy_version_integer(value):
    """Accept exact JSON numbers or the canonical decimal encoding used by Web BigInt."""
    if isinstance(value, str) and re.fullmatch(r"0|[1-9][0-9]{0,9}", value):
        value = int(value)
    if type(value) is not int or not 0 <= value <= (1 << 32) - 1:
        raise ValueError("invalid Rust policy version integer")
    return value


PROFILES = {"original": "emuera.em", "snake": "emuera.skia.snake"}
OPERATION_FIELDS = {
    "eval": ("value",),
    "run": ("termination", "output", "watches"),
    "execute": ("termination", "output", "watches"),
}


def same_json_value(actual, expected):
    """Preserve scalar types: Python otherwise equates 777 and 777.0 (or 1 and True)."""
    if type(actual) is not type(expected):
        return False
    if isinstance(actual, dict):
        return actual.keys() == expected.keys() and all(
            same_json_value(actual[key], expected[key]) for key in actual)
    if isinstance(actual, list):
        return len(actual) == len(expected) and all(
            same_json_value(left, right) for left, right in zip(actual, expected))
    return actual == expected


def validate_rust_evidence(evidence, oracle, fixture, seed, required_policy=None):
    if evidence.get("version") != 1:
        raise ValueError("unsupported Rust evidence version")
    # Keep the raw capture identity untouched for correlated protocol/diagnostic checks.
    identity = dict(evidence.get("profile", {}))
    for field in ("semantic_version", "policy_version", "rng_state_version"):
        identity[field] = policy_version_integer(identity.get(field))
    if identity.get("profile") != PROFILES[oracle]:
        raise ValueError("Rust evidence belongs to a different compatibility profile")
    versions = (identity.get("semantic_version"), identity.get("policy_version"))
    supported = {(1, 1)} if oracle == "original" else {(1, 1), (2, 2), (3, 3)}
    if versions not in supported:
        raise ValueError(f"unsupported Rust semantic/policy versions: {versions!r}")
    for key, value in (required_policy or {}).items():
        if identity.get(key) != value:
            raise ValueError(f"fixture requires Rust policy {key}={value!r}")
    expected = {
        "arithmetic": ("snake_saturating_i64_v1"
                       if oracle == "snake" and versions == (3, 3) else "wrapping_i64_v1"),
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


def output_after_load(output, load_response):
    """Subtract only the exact observed load prefix; never match loading text patterns."""
    if not isinstance(output, list) or not all(isinstance(line, str) for line in output):
        raise ValueError("output observations must be arrays of logical lines")
    if load_response is None:
        return output, "already_operation_scoped"
    baseline = load_response.get("result", {}).get("output")
    if not isinstance(baseline, list) or not all(isinstance(line, str) for line in baseline):
        raise ValueError("load must capture the full output baseline")
    if output[:len(baseline)] != baseline:
        return None, "incomparable_load_prefix_changed"
    return output[len(baseline):], "exact_load_prefix_removed"


def split_setup_diagnostics(diagnostics, identity):
    if not isinstance(diagnostics, list):
        raise ValueError("diagnostics must be an array")
    setup, script = [], []
    for diagnostic in diagnostics:
        context = diagnostic.get("context") or {}
        source = diagnostic.get("source") or {}
        is_setup = (
            identity is not None and identity.get("profile") == "emuera.skia.snake"
            and diagnostic.get("code") == "runtime.experimental_compatibility_profile"
            and diagnostic.get("level") == "warning"
            and context.get("stage") == "configuration"
            and context.get("identity") == identity
            and context.get("api") is None and context.get("required_capability") is None
            and source.get("relative_path") == "reraconfig.toml"
            and source.get("byte_start") == 0 and source.get("byte_end") == 0
        )
        (setup if is_setup else script).append(diagnostic)
    return setup, script


def compare_case(case, oracle_steps, rust_case, load_response=None, identity=None):
    if rust_case is None:
        raise ValueError(f"Rust evidence missing case {case['id']}")
    if case.get("requireSuccessfulLoad") and not (rust_case.get("load") or {}).get("success"):
        return {
            "case": case["id"], "status": "blocked", "steps": [],
            "reason": "fixture did not load successfully; a load failure cannot satisfy an expected operation rejection",
            "rustLoad": rust_case.get("load"), "oracleLoad": load_response,
            "targetBatch": case["targetBatch"], "snakeTargetStatus": case["snakeTargetStatus"],
        }
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
        actual = dict(rust["result"])
        setup_diagnostics, actual["diagnostics"] = split_setup_diagnostics(actual.get("diagnostics"), identity)
        expected = {
            "ok": response["ok"],
            **response.get("result", {}),
            "diagnostics": response["diagnostics"],
        }
        operation = request["op"]
        if operation not in OPERATION_FIELDS:
            raise ValueError(f"unsupported comparison operation {operation}")
        # The NDJSON envelope reports whether the request was handled. A run
        # can be handled successfully while the actual console is in Error.
        # Rust's ok describes execution, so compare that outcome, not transport.
        if operation in ("run", "execute") and expected.get("termination") == "error":
            expected["ok"] = False
        # Eval executes a generated wrapper in Rust; its output/termination are
        # harness state, not observable fields of the oracle's eval endpoint.
        fields = ["ok"]
        if actual.get("ok") and expected.get("ok"):
            fields.extend(OPERATION_FIELDS[operation])
        elif operation in ("run", "execute"):
            # Error presentation has no shared schema, but a fault must not hide
            # observable state mutations. Normalize only actual script-error
            # terminations; timeout/limit/quit/missing outcomes stay distinct.
            for observation in (actual, expected):
                termination = observation.get("termination")
                observation["executionOutcome"] = (
                    "script_error" if termination in ("error", "faulted") else termination)
            fields.append("executionOutcome")
            if request.get("watch"):
                fields.append("watches")
        output_comparison = None
        output_incomparable = False
        if "output" in fields:
            expected["output"], output_comparison = output_after_load(expected.get("output"), load_response)
            output_incomparable = expected["output"] is None
            if output_incomparable:
                fields.remove("output")
        compared, differences = [], []
        for field in fields:
            compared.append(field)
            if field == "output":
                for value in (actual.get(field), expected.get(field)):
                    if not isinstance(value, list) or not all(isinstance(line, str) for line in value):
                        raise ValueError("output observations must be arrays of logical lines")
            equal = (same_json_value(actual.get(field), expected.get(field)) if field == "watches"
                     else actual.get(field) == expected.get(field))
            if field not in actual or field not in expected or not equal:
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
            "status": "different" if differences else "incomparable" if diagnostic_incomparable or output_incomparable else "matched_observables",
            "compared": compared,
            "outputComparison": output_comparison,
            "setupDiagnostics": setup_diagnostics,
            "oracleRequestAccepted": response["ok"],
            "differences": differences,
            "diagnosticComparison": {
                "status": "incomparable_schema" if diagnostic_incomparable else "matched_empty",
                **diagnostics,
            },
            "rejectionComparison": {
                "status": "matched_observed_rejection" if (
                    actual.get("executionOutcome") == "script_error"
                    and expected.get("executionOutcome") == "script_error"
                    and not differences) else "not_established",
                "watchesCompared": "watches" in compared,
                "rustTermination": actual.get("termination"),
                "oracleTermination": expected.get("termination"),
                "diagnosticEquivalence": False,
            } if "executionOutcome" in compared else None,
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
        "oracleLoad": load_response,
        "targetBatch": case["targetBatch"],
        "snakeTargetStatus": case["snakeTargetStatus"],
    }
