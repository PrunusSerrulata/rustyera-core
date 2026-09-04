#!/usr/bin/env python3
"""Freeze, capture, and classify the snake batch-4.0 compatibility baseline."""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict, deque
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import shutil
import sqlite3
import subprocess
import tempfile
import time
from typing import Any, Iterator

from snake_batch3_sql_oracle import (
    OracleProcess,
    diagnostic_locations,
    file_identity,
    safe_relative_path,
    sha256,
    without_response_id,
    wrapper_identity,
)


SEMANTIC_BASELINE = "fc4fb21416768c17256d0e82f997e5f99c9bba91"
IMPLEMENTATION = "emuera_lazyloading_selfmodified_version"
TOOL_ROOT = Path(__file__).resolve().parent
FIXTURE_ROOT = TOOL_ROOT / "fixture-snake-batch4-baseline"
PLAN_PATH = FIXTURE_ROOT / "cases.json"
GOLDEN_PATH = FIXTURE_ROOT / "oracle.json"
REPORT_KIND = "snake_batch4_diagnostic_classification_v1"
RESOURCE_SUFFIXES = {
    ".bmp", ".gif", ".ico", ".jpeg", ".jpg", ".mp3", ".ogg", ".png",
    ".svg", ".wav", ".webm", ".webp", ".xml",
}
FUTURE_MARKERS = (
    "test", "debug", "测试", "測試", "魔改版更新记录文档", "開発", "开发",
)


def read_object(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def write_object(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def dependency_source(relative: str) -> Path:
    candidate = (FIXTURE_ROOT / relative).resolve(strict=True)
    return candidate


def fixture_manifest(plan: dict[str, Any]) -> list[dict[str, Any]]:
    paths = [FIXTURE_ROOT / name for name in ("cases.json", "emuera.config", "setting.json")]
    paths.extend(
        path
        for directory in ("csv", "erb")
        for path in (FIXTURE_ROOT / directory).rglob("*")
        if path.is_file()
    )
    result = [
        {"path": str(path.relative_to(FIXTURE_ROOT)), **file_identity(path)}
        for path in sorted(paths)
        if path != GOLDEN_PATH
    ]
    for item in plan["resourceDependencies"]:
        source = dependency_source(item["source"])
        identity = file_identity(source)
        if identity.get("sha256") != item["sha256"]:
            raise AssertionError(f"resource digest mismatch: {source}")
        result.append(
            {"path": f"external:{item['source']}", "destination": item["destination"], **identity}
        )
    return result


def assemble_fixture(destination: Path, plan: dict[str, Any]) -> Path:
    game = destination / "game"
    shutil.copytree(FIXTURE_ROOT, game, ignore=shutil.ignore_patterns("oracle.json"))
    for item in plan["resourceDependencies"]:
        source = dependency_source(item["source"])
        target = safe_relative_path(game, item["destination"])
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
    return game


def fixed_font(plan: dict[str, Any]) -> dict[str, Any]:
    matches = [
        item for item in plan["resourceDependencies"]
        if str(item["destination"]).replace("\\", "/").startswith("font/")
    ]
    if len(matches) != 1:
        raise AssertionError("case plan must declare exactly one presentation font")
    return matches[0]


def load_request(oracle: OracleProcess, game: Path, plan: dict[str, Any]) -> dict[str, Any]:
    font = fixed_font(plan)
    font_path = safe_relative_path(game, font["destination"])
    return {
        "op": "load",
        "gameDir": oracle.windows_path(game),
        "seed": plan["seed"],
        "observePresentation": True,
        "presentationFont": {
            "family": plan["viewport"]["font"],
            "file": oracle.windows_path(font_path),
            "sha256": font["sha256"],
        },
    }


def capture_preflight(args: argparse.Namespace, plan: dict[str, Any]) -> dict[str, Any]:
    if plan.get("semanticBaseline") != SEMANTIC_BASELINE:
        raise AssertionError("case plan semantic baseline mismatch")
    for item in plan["resourceDependencies"]:
        relative = Path(item["destination"])
        if relative.is_absolute() or ".." in relative.parts:
            raise ValueError(f"unsafe fixture resource destination: {item['destination']!r}")
    return {
        "referenceExecutable": file_identity(args.exe),
        "wrapper": wrapper_identity(args.exe),
        "fixtureManifest": fixture_manifest(plan),
    }


def project_response(response: dict[str, Any]) -> dict[str, Any]:
    result = response.get("result")
    if not isinstance(result, dict):
        result = {}
    projected: dict[str, Any] = {
        "ok": response.get("ok"),
        "schemaVersion": response.get("schemaVersion"),
        "referenceCommit": response.get("referenceCommit"),
        "state": result.get("state"),
        "termination": result.get("termination"),
        "watches": result.get("watches"),
        "output": result.get("output"),
        "diagnostics": diagnostic_locations(response),
    }
    if "presentation" in result:
        projected["presentation"] = result["presentation"]
    if result.get("termination") == "error":
        position = result.get("position")
        if isinstance(position, dict):
            projected["position"] = {
                "file": Path(str(position.get("Filename", position.get("file", "")))).name,
                "line": position.get("LineNo", position.get("line")),
            }
    return projected


def capture_case(
    args: argparse.Namespace,
    plan: dict[str, Any],
    case: dict[str, Any],
    deadline: float,
) -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix=f"snake-b4-{case['id']}-") as temporary:
        directory = Path(temporary)
        game = assemble_fixture(directory, plan)
        oracle = OracleProcess(args, directory, deadline, case["id"])
        record: dict[str, Any] = {"runnerFailure": None}
        try:
            record["capabilities"] = without_response_id(oracle.capabilities())
            load = oracle.request(load_request(oracle, game, plan))
            record["load"] = without_response_id(load)
            if not load.get("ok") or load.get("result", {}).get("termination") != "waitingInput":
                raise AssertionError("fixture did not reach the title wait")
            request: dict[str, Any] = {
                "op": "run",
                "entry": case["entry"],
                "watch": case["watches"],
            }
            if case.get("presentation"):
                request["observePresentation"] = True
            response = oracle.request(request)
            record["run"] = without_response_id(response)
            termination = response.get("result", {}).get("termination")
            if termination != case["termination"]:
                raise AssertionError(
                    f"expected {case['termination']}, got {termination}"
                )
        except Exception as error:
            record["runnerFailure"] = f"{type(error).__name__}: {error}"
        finally:
            exit_code, stderr = oracle.close()
            record["processExitCode"] = exit_code
            record["stderr"] = stderr
            if exit_code != 0 and record["runnerFailure"] is None:
                record["runnerFailure"] = f"reference process exited with code {exit_code}"
        return record


def stable_capture_projection(capture: dict[str, Any], plan: dict[str, Any]) -> dict[str, Any]:
    expected = {case["id"] for case in plan["cases"]}
    if set(capture.get("cases", {})) != expected:
        raise AssertionError("capture does not contain every planned case")
    projected_cases = {}
    load_presentations = []
    for case_id, record in sorted(capture["cases"].items()):
        if record.get("runnerFailure"):
            raise AssertionError(f"{case_id}: {record['runnerFailure']}")
        if record.get("processExitCode") != 0:
            raise AssertionError(f"{case_id}: reference process did not exit cleanly")
        load = record.get("load", {}).get("result", {})
        presentation = load.get("presentation")
        if not isinstance(presentation, dict):
            raise AssertionError(f"{case_id}: fixed-font load presentation is missing")
        load_presentations.append(presentation)
        projected_cases[case_id] = project_response(record["run"])
        projected_cases[case_id]["processExitCode"] = 0
    if any(item != load_presentations[0] for item in load_presentations[1:]):
        raise AssertionError("fixed-font load presentation differs between cases")
    return {
        "formatVersion": capture["formatVersion"],
        "semanticBaseline": capture["semanticBaseline"],
        "implementation": capture["implementation"],
        "referenceExecutable": capture["referenceExecutable"],
        "wrapper": capture["wrapper"],
        "fixtureManifest": capture["fixtureManifest"],
        "viewport": plan["viewport"],
        "loadPresentationEvidence": load_presentations[0],
        "cases": projected_cases,
    }


def capture(args: argparse.Namespace) -> None:
    plan = read_object(PLAN_PATH)
    preflight = capture_preflight(args, plan)
    deadline = time.monotonic() + args.safety_ceiling_seconds
    records = {}
    for case in plan["cases"]:
        records[case["id"]] = capture_case(args, plan, case, deadline)
        print(f"CAPTURED {case['id']}", flush=True)
        if records[case["id"]].get("runnerFailure"):
            break
    result = {
        "formatVersion": 1,
        "semanticBaseline": SEMANTIC_BASELINE,
        "implementation": IMPLEMENTATION,
        **preflight,
        "cases": records,
    }
    failures = [
        f"{case_id}: {record['runnerFailure']}"
        for case_id, record in records.items()
        if record.get("runnerFailure")
    ]
    if not failures:
        result["stableProjection"] = stable_capture_projection(result, plan)
    write_object(args.output, result)
    if failures:
        raise AssertionError("; ".join(failures))


def verify(args: argparse.Namespace) -> None:
    plan = read_object(PLAN_PATH)
    capture_value = read_object(args.capture)
    golden = read_object(GOLDEN_PATH)
    if golden.get("status") != "fixed-snake-reference-golden":
        raise AssertionError("oracle.json is not a fixed golden")
    projection = stable_capture_projection(capture_value, plan)
    if projection != golden.get("projection"):
        raise AssertionError("capture projection differs from oracle.json")
    if args.output:
        write_object(args.output, projection)
    print("PASS snake batch-4.0 reference oracle (offline)")


class JsonStream:
    """Bounded incremental JSON reader for the 4.7 GiB coverage report."""

    def __init__(self, source: io.TextIOBase, chunk_size: int = 1024 * 1024) -> None:
        self.source = source
        self.chunk_size = chunk_size
        self.buffer = ""
        self.position = 0
        self.eof = False
        self.decoder = json.JSONDecoder()
        self.raw_sha256 = hashlib.sha256()
        self.raw_bytes = 0

    def _read(self) -> bool:
        chunk = self.source.read(self.chunk_size)
        if not chunk:
            self.eof = True
            return False
        encoded = chunk.encode("utf-8")
        self.raw_sha256.update(encoded)
        self.raw_bytes += len(encoded)
        if self.position:
            self.buffer = self.buffer[self.position :]
            self.position = 0
        self.buffer += chunk
        return True

    def seek(self, marker: str) -> None:
        while True:
            found = self.buffer.find(marker, self.position)
            if found >= 0:
                self.position = found + len(marker)
                return
            keep = max(0, len(self.buffer) - len(marker) + 1)
            if keep > self.position:
                self.position = keep
            if not self._read():
                raise EOFError(f"marker not found: {marker}")

    def _skip_space_and_commas(self) -> None:
        while True:
            while self.position < len(self.buffer) and self.buffer[self.position] in " \r\n\t,":
                self.position += 1
            if self.position < len(self.buffer) or self.eof:
                return
            self._read()

    def array(self) -> Iterator[Any]:
        while True:
            self._skip_space_and_commas()
            if self.position < len(self.buffer) and self.buffer[self.position] == "]":
                self.position += 1
                return
            while True:
                try:
                    value, end = self.decoder.raw_decode(self.buffer, self.position)
                    self.position = end
                    yield value
                    break
                except json.JSONDecodeError:
                    if not self._read():
                        raise EOFError("incomplete JSON array")

    def value(self) -> Any:
        self._skip_space_and_commas()
        while True:
            try:
                value, end = self.decoder.raw_decode(self.buffer, self.position)
                self.position = end
                return value
            except json.JSONDecodeError:
                if not self._read():
                    raise EOFError("incomplete JSON value")

    def drain(self, progress: "Progress") -> None:
        while self._read():
            progress.update("hash_raw_report", self.raw_bytes, "coverage report suffix")


class Progress:
    def __init__(self, source: Path, interval: float = 5.0) -> None:
        self.source = str(source)
        self.interval = interval
        self.last_emit = time.monotonic()
        self.previous: dict[str, Any] | None = None

    def update(self, stage: str, completed: int, pending: str) -> None:
        now = time.monotonic()
        if now - self.last_emit < self.interval:
            return
        snapshot = {
            "stage": stage,
            "completed": completed,
            "pending": pending,
            "source": self.source,
        }
        print(
            "WATCHDOG " + json.dumps(snapshot, ensure_ascii=False, sort_keys=True),
            file=os.sys.stderr,
            flush=True,
        )
        if snapshot == self.previous:
            raise TimeoutError(f"batch-4 classifier made no progress in {stage}")
        self.previous = snapshot
        self.last_emit = now


def ascii_upper(value: str) -> str:
    return "".join(
        chr(ord(character) - 32) if "a" <= character <= "z" else character
        for character in value
    )


def segment_value(segment: dict[str, Any]) -> tuple[str, str]:
    return str(segment.get("kind", "")), str(segment.get("value", ""))


def exact_pattern(target: dict[str, Any]) -> str | None:
    segments = target.get("pattern", {}).get("segments", [])
    values = []
    for segment in segments:
        kind, value = segment_value(segment)
        if kind != "literal":
            return None
        values.append(value)
    return "".join(values)


def pattern_matches(target: dict[str, Any], candidate: str) -> bool:
    segments = target.get("pattern", {}).get("segments", [])
    candidate = ascii_upper(candidate)
    literals = [ascii_upper(value) for kind, value in map(segment_value, segments) if kind == "literal"]
    if all(segment_value(segment)[0] == "literal" for segment in segments):
        return "".join(literals) == candidate
    offset = 0
    end = len(candidate)
    if segments and segment_value(segments[0])[0] == "literal":
        prefix = literals.pop(0) if literals else ""
        if not candidate.startswith(prefix):
            return False
        offset = len(prefix)
    if segments and segment_value(segments[-1])[0] == "literal":
        suffix = literals.pop() if literals else ""
        if not candidate.endswith(suffix):
            return False
        end -= len(suffix)
    for literal in literals:
        found = candidate.find(literal, offset, end)
        if found < 0:
            return False
        offset = found + len(literal)
    return offset <= end


def compact_target(target: dict[str, Any]) -> str:
    return json.dumps(target, ensure_ascii=False, separators=(",", ":"), sort_keys=True)


def flush_api_counts(database: sqlite3.Connection, counts: Counter[tuple[int, str]]) -> None:
    if not counts:
        return
    database.executemany(
        "INSERT INTO owner_api(owner, api, count) VALUES (?, ?, ?) "
        "ON CONFLICT(owner, api) DO UPDATE SET count = count + excluded.count",
        ((owner, api, count) for (owner, api), count in counts.items()),
    )
    counts.clear()


def rows_to_database(
    stream: JsonStream, database: sqlite3.Connection, progress: Progress
) -> dict[str, int]:
    database.executescript(
        """
        CREATE TABLE owner_api(owner INTEGER NOT NULL, api TEXT NOT NULL, count INTEGER NOT NULL,
            PRIMARY KEY(owner, api));
        CREATE TABLE direct_edge(owner INTEGER NOT NULL, target TEXT NOT NULL, row_id INTEGER NOT NULL);
        CREATE INDEX direct_edge_owner ON direct_edge(owner);
        CREATE TABLE dynamic_target(owner INTEGER NOT NULL, row_id INTEGER NOT NULL, api TEXT NOT NULL,
            target TEXT NOT NULL);
        CREATE INDEX dynamic_target_owner ON dynamic_target(owner);
        """
    )
    api_counts: Counter[tuple[int, str]] = Counter()
    totals = Counter()
    for row_id, row in enumerate(stream.array()):
        totals["rows"] += 1
        appearance = row.get("appearance", {})
        owner = appearance.get("owning_function")
        if not isinstance(owner, int):
            totals["unownedRows"] += 1
            continue
        api = str(appearance.get("api", ""))
        valid = (
            appearance.get("activity") == "active_ast"
            and appearance.get("span_status") == "valid_decoded_utf8"
            and appearance.get("ownership_status") == "parser_function_membership_not_execution"
        )
        if valid:
            api_counts[(owner, api)] += 1
            totals["activeValidOwnedRows"] += 1
        else:
            totals["unverifiedOwnedRows"] += 1
        target = appearance.get("target")
        if valid and isinstance(target, dict):
            exact = exact_pattern(target)
            if (
                target.get("executes_body")
                and target.get("namespace") == "function"
                and str(target.get("dispatch", "")).startswith("direct_")
                and exact is not None
            ):
                database.execute(
                    "INSERT INTO direct_edge VALUES (?, ?, ?)",
                    (owner, ascii_upper(exact), row_id),
                )
                totals["directEdges"] += 1
            elif target.get("namespace") == "function":
                database.execute(
                    "INSERT INTO dynamic_target VALUES (?, ?, ?, ?)",
                    (owner, row_id, api, compact_target(target)),
                )
                totals["dynamicTargets"] += 1
        if len(api_counts) >= 50_000:
            flush_api_counts(database, api_counts)
            database.commit()
        progress.update("coverage_rows", row_id + 1, appearance.get("path", ""))
    flush_api_counts(database, api_counts)
    database.commit()
    return dict(totals)


def function_index(functions: list[dict[str, Any]]) -> tuple[dict[str, list[int]], dict[int, dict[str, Any]]]:
    names: dict[str, list[int]] = defaultdict(list)
    by_id = {}
    for function in functions:
        function_id = function.get("id")
        name = function.get("name")
        if isinstance(function_id, int) and isinstance(name, str):
            names[ascii_upper(name)].append(function_id)
            by_id[function_id] = function
    return names, by_id


def route_closure(
    seeds: list[str],
    names: dict[str, list[int]],
    database: sqlite3.Connection,
    progress: Progress,
) -> tuple[set[int], list[dict[str, int]]]:
    roots = {function for seed in seeds for function in names.get(ascii_upper(seed), [])}
    visited = set(roots)
    queue = deque(roots)
    edges = []
    while queue:
        owner = queue.popleft()
        for target, row_id in database.execute(
            "SELECT target, row_id FROM direct_edge WHERE owner = ?", (owner,)
        ):
            for next_function in names.get(target, []):
                edges.append({"from": owner, "to": next_function, "rowId": row_id})
                if next_function not in visited:
                    visited.add(next_function)
                    queue.append(next_function)
        progress.update("static_route_closure", len(visited), f"owner={owner}")
    return visited, edges


def route_api_counts(database: sqlite3.Connection, functions: set[int]) -> dict[str, int]:
    result: Counter[str] = Counter()
    ids = sorted(functions)
    for offset in range(0, len(ids), 800):
        chunk = ids[offset : offset + 800]
        placeholders = ",".join("?" for _ in chunk)
        for api, count in database.execute(
            f"SELECT api, SUM(count) FROM owner_api WHERE owner IN ({placeholders}) GROUP BY api",
            chunk,
        ):
            result[api] += count
    return dict(sorted(result.items()))


def dynamic_targets(
    database: sqlite3.Connection,
    functions: set[int],
    all_functions: dict[int, dict[str, Any]],
    progress: Progress,
) -> tuple[list[dict[str, Any]], set[int]]:
    result = []
    candidates: set[int] = set()
    names = [(function_id, str(function["name"])) for function_id, function in all_functions.items()]
    for owner in sorted(functions):
        for row_id, api, encoded in database.execute(
            "SELECT row_id, api, target FROM dynamic_target WHERE owner = ?", (owner,)
        ):
            target = json.loads(encoded)
            matched = [
                function_id for function_id, name in names if pattern_matches(target, name)
            ] if target.get("namespace") == "function" else []
            candidates.update(matched)
            result.append(
                {
                    "owner": owner,
                    "rowId": row_id,
                    "api": api,
                    "target": target,
                    "candidateCount": len(matched),
                    "candidateFunctionIds": matched[:200],
                    "candidateListTruncated": len(matched) > 200,
                    "policy": "candidate only; never promoted into static closure",
                }
            )
            progress.update("dynamic_targets", len(result), f"owner={owner} row={row_id}")
    return result, candidates


def containing_function(
    diagnostic: dict[str, Any], by_path: dict[str, list[dict[str, Any]]]
) -> int | None:
    path = diagnostic.get("path")
    span = diagnostic.get("span")
    if not isinstance(path, str) or not isinstance(span, dict):
        return None
    start = span.get("start")
    if not isinstance(start, int):
        return None
    matches = []
    for function in by_path.get(path, []):
        function_span = function.get("span", {})
        if function_span.get("start", start + 1) <= start < function_span.get("end", start):
            matches.append(function)
    if not matches:
        return None
    return min(matches, key=lambda item: item["span"]["end"] - item["span"]["start"])["id"]


def diagnostic_classification(
    diagnostics: list[dict[str, Any]],
    functions: dict[int, dict[str, Any]],
    route_sets: dict[str, set[int]],
    dynamic_sets: dict[str, set[int]],
) -> tuple[list[dict[str, Any]], dict[str, int]]:
    by_path: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for function in functions.values():
        by_path[str(function.get("path", ""))].append(function)
    decorated = []
    root_statuses: dict[int, str] = {}
    grouped: dict[tuple[Any, ...], list[tuple[int, dict[str, Any]]]] = defaultdict(list)
    for diagnostic_id, diagnostic in enumerate(diagnostics):
        if not diagnostic.get("error"):
            root_statuses[diagnostic_id] = "non_error"
            continue
        span = diagnostic.get("span")
        if not isinstance(diagnostic.get("path"), str) or not isinstance(span, dict):
            root_statuses[diagnostic_id] = "independent_unlocated_error"
            continue
        group = (diagnostic.get("stage"), diagnostic.get("path"))
        grouped[group].append((diagnostic_id, diagnostic))
    for values in grouped.values():
        values.sort(key=lambda item: (item[1].get("span") or {}).get("start", -1))
        clusters: list[list[tuple[int, dict[str, Any]]]] = []
        cluster_end = -1
        for item in values:
            span = item[1]["span"]
            start, end = span["start"], span["end"]
            overlaps = bool(clusters) and (
                start < cluster_end
                or span == clusters[-1][-1][1]["span"]
            )
            if not overlaps:
                clusters.append([item])
                cluster_end = end
            else:
                clusters[-1].append(item)
                cluster_end = max(cluster_end, end)
        for cluster in clusters:
            if len(cluster) == 1:
                root_statuses[cluster[0][0]] = "independent_error"
                continue
            root_statuses[cluster[0][0]] = "first_root_at_overlapping_span"
            for diagnostic_id, _ in cluster[1:]:
                root_statuses[diagnostic_id] = "cascade_candidate_overlapping_span"
    counts: Counter[str] = Counter()
    for diagnostic_id, diagnostic in enumerate(diagnostics):
        function_id = containing_function(diagnostic, by_path)
        static_routes = sorted(
            route for route, members in route_sets.items() if function_id in members
        )
        candidate_routes = sorted(
            route for route, members in dynamic_sets.items() if function_id in members
        )
        path = str(diagnostic.get("path") or "")
        function_name = str(functions.get(function_id, {}).get("name", ""))
        future = any(marker.casefold() in f"{path}/{function_name}".casefold() for marker in FUTURE_MARKERS)
        if static_routes:
            reachability = "static_reachable"
        elif candidate_routes:
            reachability = "dynamic_target_candidate"
        elif future:
            reachability = "future_or_test_code"
        elif function_id is None:
            reachability = "unlocated"
        else:
            reachability = "outside_frozen_routes"
        root_status = root_statuses[diagnostic_id]
        counts[reachability] += 1
        counts[root_status] += 1
        decorated.append(
            {
                "id": diagnostic_id,
                "stage": diagnostic.get("stage"),
                "path": diagnostic.get("path"),
                "span": diagnostic.get("span"),
                "code": diagnostic.get("code"),
                "error": diagnostic.get("error"),
                "functionId": function_id,
                "functionName": function_name or None,
                "rootStatus": root_status,
                "reachability": reachability,
                "staticRoutes": static_routes,
                "dynamicCandidateRoutes": candidate_routes,
            }
        )
    return decorated, dict(sorted(counts.items()))


def validate_report_identity(report: Path, expected: dict[str, Any]) -> dict[str, Any]:
    identity = file_identity(report)
    stored = expected["storedFile"]
    if identity.get("size") != stored["bytes"] or identity.get("sha256") != stored["sha256"]:
        raise AssertionError("coverage report stored-file identity mismatch")
    manifest_path = Path(str(report) + ".manifest.json")
    if not manifest_path.is_file():
        raise AssertionError("coverage report digest manifest is missing")
    manifest = read_object(manifest_path)
    if manifest.get("status") != "complete":
        raise AssertionError("coverage report manifest is not complete")
    if manifest.get("stored_file") != {
        "blake3": manifest.get("stored_file", {}).get("blake3"),
        "bytes": stored["bytes"],
        "sha256": stored["sha256"],
    }:
        raise AssertionError("coverage report manifest stored identity mismatch")
    raw = manifest.get("raw_json", {})
    if raw.get("bytes") != expected["rawJson"]["bytes"] or raw.get("sha256") != expected["rawJson"]["sha256"]:
        raise AssertionError("coverage report manifest raw identity mismatch")
    return {"file": identity, "manifest": manifest}


def report_marker_value(report: Path, marker: str) -> Any:
    with gzip.open(report, "rb") as compressed:
        stream = JsonStream(io.TextIOWrapper(compressed, encoding="utf-8"))
        stream.seek(marker)
        return stream.value()


def classify_report(
    report: Path,
    output: Path,
    routes: dict[str, list[str]],
    expected: dict[str, Any],
) -> None:
    stored_identity = validate_report_identity(report, expected)
    for marker, key in (
        ('"kind":', "kind"),
        ('"project":', "project"),
        ('"profile_override":', "profile"),
    ):
        if report_marker_value(report, marker) != expected[key]:
            raise AssertionError(f"coverage report {key} mismatch")
    output.parent.mkdir(parents=True, exist_ok=True)
    progress = Progress(report)
    database_path = output.with_suffix(output.suffix + ".work.sqlite3")
    if database_path.exists():
        database_path.unlink()
    database = sqlite3.connect(database_path)
    try:
        with gzip.open(report, "rb") as compressed:
            text = io.TextIOWrapper(compressed, encoding="utf-8")
            stream = JsonStream(text)
            stream.seek('"pipeline":')
            stream.seek('"diagnostics":[')
            diagnostics = []
            for index, diagnostic in enumerate(stream.array(), 1):
                diagnostics.append(diagnostic)
                progress.update("diagnostics", index, str(diagnostic.get("path", "")))
            if len(diagnostics) != expected["diagnostics"]:
                raise AssertionError("coverage report diagnostic count mismatch")
            stream.seek('"rows":[')
            totals = rows_to_database(stream, database, progress)
            stream.seek('"parser_functions":[')
            functions = []
            for index, function in enumerate(stream.array(), 1):
                functions.append(function)
                progress.update("parser_functions", index, str(function.get("path", "")))
            stream.drain(progress)
            raw_identity = {"bytes": stream.raw_bytes, "sha256": stream.raw_sha256.hexdigest()}
            if raw_identity != expected["rawJson"]:
                raise AssertionError("coverage report decompressed identity mismatch")
        names, functions_by_id = function_index(functions)
        route_sets = {}
        route_reports = {}
        dynamic_sets = {}
        for route, seeds in routes.items():
            closure, edges = route_closure(seeds, names, database, progress)
            dynamic, dynamic_candidates = dynamic_targets(
                database, closure, functions_by_id, progress
            )
            route_sets[route] = closure
            dynamic_sets[route] = dynamic_candidates
            route_reports[route] = {
                "seeds": seeds,
                "foundSeedFunctionIds": {
                    seed: names.get(ascii_upper(seed), [])
                    for seed in seeds
                    if names.get(ascii_upper(seed))
                },
                "missingSeeds": [seed for seed in seeds if not names.get(ascii_upper(seed))],
                "staticFunctionCount": len(closure),
                "staticFunctionIds": sorted(closure),
                "staticEdgeCount": len(edges),
                "staticEdges": edges,
                "reachableApiCounts": route_api_counts(database, closure),
                "dynamicTargets": dynamic,
            }
        classified, counts = diagnostic_classification(
            diagnostics, functions_by_id, route_sets, dynamic_sets
        )
        result = {
            "formatVersion": 1,
            "kind": REPORT_KIND,
            "sourceReport": {
                "path": str(report),
                **stored_identity["file"],
                "digestManifest": stored_identity["manifest"],
                "verifiedRawJson": raw_identity,
            },
            "policy": {
                "static": "only active, valid, function-owned direct exact-name edges enter a route closure",
                "dynamic": "pattern matches are candidate lists and never become static reachability",
                "rootCause": "only same or overlapping spans in the same stage/path form a root/cascade candidate cluster; non-overlapping and unlocated errors remain independent",
                "futureCode": f"path or function name contains one of {list(FUTURE_MARKERS)} and is outside a frozen static/dynamic route",
                "limitation": "static reference evidence, not execution proof",
            },
            "reportTotals": {**totals, "diagnostics": len(diagnostics), "functions": len(functions)},
            "classificationCounts": counts,
            "routes": route_reports,
            "diagnostics": classified,
            "implementationCandidateDiagnosticIds": [
                item["id"]
                for item in classified
                if item["error"]
                and item["rootStatus"] != "cascade_candidate_overlapping_span"
                and item["reachability"] in {"static_reachable", "dynamic_target_candidate"}
            ],
        }
        write_object(output, result)
    finally:
        database.close()
        database_path.unlink(missing_ok=True)


def classify(args: argparse.Namespace) -> None:
    plan = read_object(PLAN_PATH)
    classify_report(
        args.report,
        args.output,
        plan["routes"],
        plan["coverageReport"],
    )


def git_identity(path: Path) -> dict[str, Any]:
    def git(*arguments: str) -> str:
        return subprocess.check_output(
            ["git", "-C", str(path), *arguments], text=True, timeout=30
        ).strip()

    status = git("status", "--porcelain=v1")
    return {
        "path": str(path.resolve()),
        "commit": git("rev-parse", "HEAD"),
        "branch": git("branch", "--show-current"),
        "dirty": bool(status),
        "status": status.splitlines(),
    }


def command_version(command: list[str]) -> dict[str, Any]:
    try:
        output = subprocess.check_output(
            command, text=True, stderr=subprocess.STDOUT, timeout=30
        ).strip()
        return {"command": command, "output": output}
    except (OSError, subprocess.SubprocessError) as error:
        return {"command": command, "error": f"{type(error).__name__}: {error}"}


def config_values(path: Path) -> dict[str, str]:
    wanted = {
        "DRAWING INTERFACE", "WINDOW WIDTH", "WINDOW HEIGHT", "FONT NAME",
        "FONT SIZE", "LINE HEIGHT", "ITEMS PER LINE FOR PRINTC",
        "NUMBER OF ITEM CHARACTERS FOR PRINTC", "RENDERING BACKEND",
        "SKIASHARP IMAGE QUALITY", "SKIASHARP FONT HINTING", "SKIASHARP FONT EDGING",
    }
    result = {}
    for line in path.read_text(encoding="utf-8-sig", errors="strict").splitlines():
        if ":" not in line:
            continue
        key, value = line.split(":", 1)
        if key.upper() in wanted:
            result[key.upper()] = value
    return result


def resource_summary(root: Path) -> dict[str, Any]:
    counts: Counter[str] = Counter()
    sizes: Counter[str] = Counter()
    manifest = hashlib.sha256()
    files = []
    for path in sorted(item for item in root.rglob("*") if item.is_file()):
        suffix = path.suffix.lower()
        if suffix not in RESOURCE_SUFFIXES:
            continue
        relative = path.relative_to(root).as_posix()
        digest = sha256(path)
        size = path.stat().st_size
        counts[suffix or "<none>"] += 1
        sizes[suffix or "<none>"] += size
        manifest.update(f"{relative}\0{size}\0{digest}\n".encode())
        files.append({"path": relative, "size": size, "sha256": digest})
    return {
        "count": len(files),
        "bytes": sum(item["size"] for item in files),
        "countsByExtension": dict(sorted(counts.items())),
        "bytesByExtension": dict(sorted(sizes.items())),
        "manifestSha256": manifest.hexdigest(),
        "files": files,
    }


def freeze(args: argparse.Namespace) -> None:
    plan = read_object(PLAN_PATH)
    bbas_paths = [
        "plugins/schema.xml", "plugins/bbas_dataset.xml",
        "plugins/bbas_map_schema.xml", "plugins/bbas_map.xml",
    ]
    result = {
        "formatVersion": 1,
        "kind": "snake_batch4_baseline_freeze_v1",
        "capturedAtUnixSeconds": int(time.time()),
        "repositories": {
            "core": git_identity(args.core),
            "tui": git_identity(args.tui),
            "web": git_identity(args.web),
            "snakeEngine": git_identity(args.snake_engine),
            "snakeTw": git_identity(args.snake_tw),
        },
        "fixture": {
            "manifest": fixture_manifest(plan),
            "viewport": plan["viewport"],
            "contracts": file_identity(FIXTURE_ROOT / "contracts.json"),
        },
        "coverageReport": plan["coverageReport"],
        "snakeTwConfiguration": config_values(args.snake_tw / "emuera.config"),
        "snakeTwResources": resource_summary(args.snake_tw),
        "bbasPreflight": {
            relative: file_identity(args.snake_tw / relative) for relative in bbas_paths
        },
        "toolchain": {
            "platform": platform.platform(),
            "python": command_version([os.sys.executable, "--version"]),
            "rustc": command_version(["rustc", "-Vv"]),
            "cargo": command_version(["cargo", "-V"]),
            "node": command_version(["node", "--version"]),
            "npm": command_version(["npm", "--version"]),
            "wine": command_version([args.wine, "--version"]) if args.wine else None,
        },
        "contracts": read_object(FIXTURE_ROOT / "contracts.json"),
    }
    write_object(args.output, result)


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    commands = result.add_subparsers(dest="command", required=True)

    capture_parser = commands.add_parser("capture")
    capture_parser.add_argument("--exe", type=Path, required=True)
    capture_parser.add_argument("--wine")
    capture_parser.add_argument("--winepath", default="winepath")
    capture_parser.add_argument("--request-timeout", type=float, default=60.0)
    capture_parser.add_argument(
        "--safety-ceiling-seconds", type=float, default=86_400.0,
        help="infrastructure safety ceiling only; not the removed 60-minute test budget",
    )
    capture_parser.add_argument("--output", type=Path, required=True)
    capture_parser.set_defaults(handler=capture)

    verify_parser = commands.add_parser("verify")
    verify_parser.add_argument("--capture", type=Path, required=True)
    verify_parser.add_argument("--output", type=Path)
    verify_parser.set_defaults(handler=verify)

    classify_parser = commands.add_parser("classify")
    classify_parser.add_argument("--report", type=Path, required=True)
    classify_parser.add_argument("--output", type=Path, required=True)
    classify_parser.set_defaults(handler=classify)

    freeze_parser = commands.add_parser("freeze")
    freeze_parser.add_argument("--core", type=Path, required=True)
    freeze_parser.add_argument("--tui", type=Path, required=True)
    freeze_parser.add_argument("--web", type=Path, required=True)
    freeze_parser.add_argument("--snake-engine", type=Path, required=True)
    freeze_parser.add_argument("--snake-tw", type=Path, required=True)
    freeze_parser.add_argument("--wine")
    freeze_parser.add_argument("--output", type=Path, required=True)
    freeze_parser.set_defaults(handler=freeze)
    return result


def main() -> int:
    args = parser().parse_args()
    for name in ("exe", "capture", "report", "core", "tui", "web", "snake_engine", "snake_tw"):
        value = getattr(args, name, None)
        if isinstance(value, Path):
            setattr(args, name, value.resolve(strict=True))
    if hasattr(args, "output") and args.output is not None:
        args.output = args.output.resolve()
    args.handler(args)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
