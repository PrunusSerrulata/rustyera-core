#!/usr/bin/env python3
"""Convert bounded real-client captures to existing oracle comparison evidence.

This is an offline validator, not a capture producer or measurement backend.
All missing observations remain blocked and all comparison verdicts remain unset.
"""

import argparse
import hashlib
import json
from pathlib import Path

from comparison import validate_rust_evidence
from frontend_capture_io import (
    MAX_STORED, digest, file_sha256, fixture_inventory, integer, read_manifest,
    frontend_files, project_payload_hashes, require, source_identity, trace_records,
    validate_frontend_artifact, validate_wasm_assets,
)
from frontend_capture_observations import CaseCapture


def validate_identity(identity, fixture, inventory, artifact_paths):
    require(identity.get("frontend") in ("browser", "tauri"), "not a real frontend capture")
    digest(identity["coreSha"], 40)
    digest(identity["frontendSha"], 40)
    require(type(identity.get("dirty")) is bool and type(identity.get("frontendDirty")) is bool,
            "capture must disclose core/frontend dirty state")
    require(identity.get("corePin") == identity["coreSha"], "pin is not the recorded runtime core SHA")
    require(identity["seed"] == integer(fixture["seed"]), "capture/fixture seed mismatch")
    require(identity.get("fixtureInventory") == inventory, "raw/decoded UTF-8 fixture inventory mismatch")
    require(identity.get("sourceFixture") == source_identity(inventory), "source fixture identity mismatch")
    submitted = identity.get("submittedPayloads")
    require(isinstance(submitted, list), "missing actual submitted payload inventory")
    by_path = {row["path"]: row for row in inventory}
    paths = set()
    for item in submitted:
        path = item["path"]
        require(path in by_path and path not in paths, "unknown/duplicate submitted payload path")
        paths.add(path)
        require(item["rawSha256"] == by_path[path]["sha256"] and
                item["decodedUtf8Sha256"] == by_path[path]["decodedUtf8Sha256"] and
                item["decodedUtf8Sha256"] is not None,
                "actual submitted payload hash mismatch")
    required = {row["path"] for row in inventory if row["path"].lower().endswith(
        (".erb", ".erh", ".csv", ".als", ".erd", ".config", ".toml"))}
    require(required <= paths, "submitted payload inventory omits script/data/config inputs")
    provenance = identity["provenance"]
    require(provenance.get("synthetic") is False and provenance.get("captureMode") == "real_client",
            "synthetic/headless provider evidence is forbidden")
    allowed = ("chromium", "firefox", "safari") if identity["frontend"] == "browser" else ("tauri",)
    require(provenance.get("clientFamily") in allowed and isinstance(provenance.get("clientVersion"), str)
            and provenance["clientVersion"], "missing actual browser/host provenance")
    expected_backend = "wasm" if identity["frontend"] == "browser" else "tauri_cabi"
    require(provenance.get("runtimeBackend") == expected_backend, "wrong runtime backend provenance")
    require(provenance.get("htmlProvider") == "html_node_dom" and
            provenance.get("canvasProvider") == "canvas_replay_renderer" and
            provenance.get("pointerProvider") == "viewport_pointer", "unknown S04 provider provenance")
    require(set(identity["artifacts"]) == set(artifact_paths) and
            {"runtime", "frontend", "client"} <= set(artifact_paths),
            "actual runtime/frontend/client artifact files are required")
    for role, path in artifact_paths.items():
        expected = identity["artifacts"][role]
        require(file_sha256(Path(path), MAX_STORED) == {
            "bytes": integer(expected["bytes"], maximum=MAX_STORED),
            "sha256": digest(expected["sha256"]),
        }, f"actual {role} artifact hash mismatch")
    profile = identity["profile"].get("profile")
    oracle = {"emuera.em": "original", "emuera.skia.snake": "snake"}.get(profile)
    require(oracle is not None, "unsupported compatibility profile")
    # Reuse the unchanged headless identity validator; negotiated frontend
    # capabilities are checked from actual server_hello independently.
    validate_rust_evidence({"version": 1, "coreSha": identity["coreSha"],
                            "dirty": identity["dirty"], "profile": identity["profile"],
                            "seed": identity["seed"], "sourceFixture": identity["sourceFixture"],
                            "cases": []}, oracle, identity["sourceFixture"], fixture["seed"],
                           fixture.get("requiredRustPolicy", {}).get(oracle))
    return oracle


def build_evidence(capture_path, fixture_root, artifact_paths, frontend_root=None, wasm_root=None):
    capture_path, fixture_root = Path(capture_path), Path(fixture_root)
    capture = read_manifest(capture_path)
    require(capture.get("version") == 1 and capture.get("kind") == "rustyera_real_frontend_capture",
            "unsupported frontend capture manifest")
    fixture = read_manifest(fixture_root / "cases.json")
    require(fixture.get("version") == 1, "unsupported fixture manifest")
    inventory = fixture_inventory(fixture_root)
    identity = capture["identity"]
    oracle = validate_identity(identity, fixture, inventory, artifact_paths)
    frontend_kind, frontend_inventory = validate_frontend_artifact(
        identity["frontendRuntime"], artifact_paths["frontend"], frontend_root)
    if identity["frontend"] == "browser":
        require(identity["frontendRuntime"]["mode"] in ("vite-dev", "static-bundle"),
                "browser capture names a native frontend mode")
        wasm_root = Path(wasm_root) if wasm_root is not None else Path(frontend_root) / "public/wasm"
        validate_wasm_assets(identity, artifact_paths["runtime"], wasm_root)
    else:
        require(identity["frontendRuntime"]["mode"] in ("embedded", "tauri-test-devserver"),
                "Tauri capture names a browser-only frontend mode")
    payload_hashes = project_payload_hashes(fixture_root, inventory)
    cases = {case["id"]: case for case in fixture["cases"]}
    require(len(cases) == len(fixture["cases"]), "duplicate fixture case ID")
    selected = capture["caseIds"]
    require(isinstance(selected, list) and selected and len(selected) == len(set(selected)) and
            all(case_id in cases for case_id in selected), "invalid selected case list")
    require(selected == [case["id"] for case in fixture["cases"] if case["id"] in selected],
            "capture cases are not in fixture order")
    menu_numbers = {case["id"]: index + 1 for index, case in enumerate(fixture["cases"])}
    observations = []
    current = None
    header = False
    footer = False
    last_index = -1
    for packet in trace_records(capture_path, capture["trace"]):
        index = integer(packet["index"])
        require(index == last_index + 1 and not footer, "capture index gap or records after footer")
        last_index = index
        kind = packet["type"]
        if not header:
            require(kind == "header" and packet["identity"] == identity and
                    packet["caseIds"] == selected, "trace header identity differs from manifest")
            header = True
            continue
        if kind == "case_begin":
            require(current is None and len(observations) < len(selected), "overlapping/extra captured case")
            case_id = selected[len(observations)]
            require(packet["case"] == case_id and packet["requests"] ==
                    [step["request"] for step in cases[case_id]["requests"]],
                    "captured case/request order differs from fixture")
            current = CaseCapture(cases[case_id], identity, menu_numbers[case_id], payload_hashes)
        elif kind == "observation":
            require(current is not None, "observation outside a case")
            current.snapshot(packet)
        elif kind == "case_end":
            require(current is not None and packet["case"] == current.case["id"] and
                    packet.get("captureComplete") is True, "incomplete or mismatched case end")
            observations.append(current.finish())
            current = None
        elif kind == "footer":
            require(current is None and len(observations) == len(selected) and
                    packet.get("captureComplete") is True, "incomplete capture footer")
            footer = True
        else:
            raise ValueError(f"unknown capture packet type {kind}")
    require(header and footer and current is None, "truncated frontend trace")
    require(fixture_inventory(fixture_root) == inventory, "fixture changed during capture validation")
    require(frontend_files(frontend_root, frontend_kind) == frontend_inventory,
            "frontend source/bundle inputs changed during validation")
    if identity["frontend"] == "browser":
        validate_wasm_assets(identity, artifact_paths["runtime"], wasm_root)
    return {"version": 1, "coreSha": identity["coreSha"], "dirty": identity["dirty"],
            "profile": identity["profile"], "seed": identity["seed"],
            "sourceFixture": identity["sourceFixture"], "cases": observations,
            "frontendCapture": {"version": 1, "frontend": identity["frontend"],
                                "frontendSha": identity["frontendSha"], "identity": identity,
                                "trace": capture["trace"], "oracleProfile": oracle,
                                "status": "validated_observations_not_comparison_verdict",
                                "provenanceStatus": "reported_client_with_verified_artifact_hashes",
                                "limitations": [
                                    "Artifact hashes do not cryptographically attest which process executed them.",
                                    "Source-derived expectations are never used to fill observations.",
                                    "Exact font/platform differences remain for oracle comparison.",
                                    "External full DOM/watchdog evidence still requires behavior review.",
                                ]}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--artifact", action="append", default=[], metavar="ROLE=PATH")
    parser.add_argument("--frontend-root", type=Path, required=True,
                        help="actual Vite source root or actual embedded/static bundle directory")
    parser.add_argument("--wasm-root", type=Path,
                        help="browser WASM directory; defaults to FRONTEND_ROOT/public/wasm")
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()
    artifacts = {}
    for argument in arguments.artifact:
        role, separator, path = argument.partition("=")
        require(separator and role and path and role not in artifacts, "invalid/duplicate artifact argument")
        artifacts[role] = Path(path)
    evidence = build_evidence(arguments.capture, arguments.fixture, artifacts,
                              arguments.frontend_root, arguments.wasm_root)
    encoded = (json.dumps(evidence, ensure_ascii=True, indent=2) + "\n").encode()
    require(len(encoded) <= 64 * 1024 * 1024, "comparison evidence output limit exceeded")
    # Exclusive creation preserves failed/previous captures; incomplete input never
    # produces an apparently complete comparison file.
    with arguments.output.open("xb") as stream:
        stream.write(encoded)
    print(json.dumps({"status": evidence["frontendCapture"]["status"],
                      "output": str(arguments.output), "bytes": len(encoded),
                      "sha256": hashlib.sha256(encoded).hexdigest()}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
