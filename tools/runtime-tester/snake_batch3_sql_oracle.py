#!/usr/bin/env python3
"""Capture or verify the fixed snake Emuera batch-3 SQL oracle."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import queue
import shutil
import signal
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
from typing import Any


SEMANTIC_BASELINE = "fc4fb21416768c17256d0e82f997e5f99c9bba91"
IMPLEMENTATION = "emuera_lazyloading_selfmodified_version"
SCHEMA_VERSION = 2
SHARED_CASES = {"bbas-preflight", "snake-tw-resources"}
TOOL_ROOT = Path(__file__).resolve().parent
FIXTURE_ROOT = TOOL_ROOT / "fixture-snake-batch3-sql-oracle"


def read_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def file_identity(path: Path) -> dict[str, Any]:
    if not path.is_file():
        return {"exists": False}
    return {"exists": True, "size": path.stat().st_size, "sha256": sha256(path)}


def safe_relative_path(root: Path, relative: str) -> Path:
    candidate_relative = Path(relative)
    if candidate_relative.is_absolute() or ".." in candidate_relative.parts:
        raise ValueError(f"unsafe relative path: {relative!r}")
    resolved_root = root.resolve(strict=True)
    candidate = (resolved_root / candidate_relative).resolve(strict=False)
    if not candidate.is_relative_to(resolved_root):
        raise ValueError(f"relative path escapes root: {relative!r}")
    return candidate


def quote_identifier(identifier: str) -> str:
    return '"' + identifier.replace('"', '""') + '"'


def inspect_sqlite(path: Path, *, immutable: bool) -> dict[str, Any]:
    query = "mode=ro&immutable=1" if immutable else "mode=ro"
    uri = f"{path.resolve(strict=True).as_uri()}?{query}"
    connection = sqlite3.connect(uri, uri=True)
    try:
        inspector_sqlite_version = connection.execute(
            "SELECT sqlite_version()"
        ).fetchone()[0]
        schema_version = connection.execute("PRAGMA schema_version").fetchone()[0]
        user_version = connection.execute("PRAGMA user_version").fetchone()[0]
        schema = [
            {"type": row[0], "name": row[1], "table": row[2], "sql": row[3]}
            for row in connection.execute(
                "SELECT type, name, tbl_name, sql FROM sqlite_schema "
                "ORDER BY type, name"
            )
        ]
        tables = [
            row[0]
            for row in connection.execute(
                "SELECT name FROM sqlite_schema "
                "WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
            )
        ]
        row_counts = {
            table: connection.execute(
                f"SELECT COUNT(*) FROM {quote_identifier(table)}"
            ).fetchone()[0]
            for table in tables
        }
        meta = None
        if "_meta" in tables:
            columns = [row[1] for row in connection.execute("PRAGMA table_info('_meta')")]
            if {"key", "value"}.issubset(columns):
                meta = [
                    {"key": row[0], "value": row[1]}
                    for row in connection.execute(
                        'SELECT "key", "value" FROM "_meta" ORDER BY "key"'
                    )
                ]
        return {
            "inspectorSqliteVersion": inspector_sqlite_version,
            "schemaVersion": schema_version,
            "userVersion": user_version,
            "schema": schema,
            "rowCounts": row_counts,
            "meta": meta,
        }
    finally:
        connection.close()


class OracleProcess:
    def __init__(
        self,
        args: argparse.Namespace,
        directory: Path,
        deadline: float,
        case_id: str,
    ) -> None:
        self.args = args
        self.directory = directory
        self.deadline = deadline
        self.case_id = case_id
        self.responses: queue.Queue[str | Exception | None] = queue.Queue()
        self.sequence = 0
        self.last_response: dict[str, Any] | None = None
        self.stderr_path = directory / "oracle-stderr.log"
        self.stderr = self.stderr_path.open("w", encoding="utf-8")
        command = ([args.wine] if args.wine else []) + [str(args.exe)]
        self.process = subprocess.Popen(
            command,
            cwd=directory,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=self.stderr,
            text=True,
            encoding="utf-8",
            errors="strict",
            start_new_session=os.name != "nt",
        )
        threading.Thread(target=self._read_stdout, daemon=True).start()

    def _read_stdout(self) -> None:
        try:
            assert self.process.stdout is not None
            for line in self.process.stdout:
                self.responses.put(line)
        except Exception as error:  # pragma: no cover - infrastructure failure
            self.responses.put(error)
        finally:
            self.responses.put(None)

    def remaining(self) -> float:
        remaining = self.deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("batch-3.0 oracle budget exhausted")
        return remaining

    def windows_path(self, path: Path) -> str:
        if not self.args.wine:
            return str(path)
        return subprocess.check_output(
            [self.args.winepath, "-w", str(path)],
            text=True,
            timeout=min(10.0, self.remaining()),
        ).strip()

    def _snapshot(self, request: dict[str, Any]) -> dict[str, Any]:
        return {
            "case": self.case_id,
            "phase": "request",
            "processAlive": self.process.poll() is None,
            "pending": request,
            "lastFullResponse": self.last_response,
        }

    def request(self, request: dict[str, Any]) -> dict[str, Any]:
        self.sequence += 1
        wire_request = {"id": self.sequence, **request}
        wire = json.dumps(wire_request, ensure_ascii=False)
        assert self.process.stdin is not None
        self.process.stdin.write(wire + "\n")
        self.process.stdin.flush()

        response_deadline = min(
            self.deadline, time.monotonic() + self.args.request_timeout
        )
        previous_snapshot: dict[str, Any] | None = None
        while True:
            remaining = min(self.remaining(), response_deadline - time.monotonic())
            if remaining <= 0:
                raise TimeoutError(f"no response to request {wire}")
            try:
                line = self.responses.get(timeout=min(5.0, remaining))
            except queue.Empty:
                snapshot = self._snapshot(wire_request)
                print(
                    "WATCHDOG " + json.dumps(snapshot, ensure_ascii=False, sort_keys=True),
                    file=sys.stderr,
                    flush=True,
                )
                if snapshot == previous_snapshot:
                    raise TimeoutError(
                        f"two identical 5-second snapshots for {self.case_id}"
                    )
                previous_snapshot = snapshot
                continue

            if line is None:
                raise RuntimeError(f"oracle exited before responding to {wire}")
            if isinstance(line, Exception):
                raise line
            response = json.loads(line)
            if not isinstance(response, dict):
                raise AssertionError("oracle response must be a JSON object")
            self.last_response = response
            if response.get("id") != self.sequence:
                raise AssertionError("oracle response id does not match request")
            if response.get("schemaVersion") != SCHEMA_VERSION:
                raise AssertionError("unexpected oracle schemaVersion")
            if response.get("referenceCommit") != SEMANTIC_BASELINE:
                raise AssertionError("unexpected snake semantic baseline")
            if not isinstance(response.get("diagnostics"), list):
                raise AssertionError("oracle diagnostics must be an array")
            return response

    def capabilities(self) -> dict[str, Any]:
        response = self.request({"op": "capabilities"})
        result = response.get("result")
        if not response.get("ok") or not isinstance(result, dict):
            raise AssertionError("capabilities request failed")
        if result.get("implementation") != IMPLEMENTATION:
            raise AssertionError("wrong reference implementation")
        return response

    def load(self, game: Path, seed: int) -> dict[str, Any]:
        return self.request(
            {"op": "load", "gameDir": self.windows_path(game), "seed": seed}
        )

    def run(self, entry: str, watches: list[str]) -> dict[str, Any]:
        return self.request(
            {"op": "run", "entry": entry, "watch": watches}
        )

    def close(self) -> tuple[int, str]:
        if self.process.stdin is not None:
            self.process.stdin.close()
        try:
            self.process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            if os.name == "nt":
                subprocess.run(
                    ["taskkill", "/PID", str(self.process.pid), "/T", "/F"],
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                    timeout=3,
                    check=False,
                )
            else:
                try:
                    os.killpg(self.process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            self.process.wait(timeout=3)
        if self.process.stdout is not None:
            self.process.stdout.close()
        self.stderr.close()
        diagnostics = self.stderr_path.read_text(encoding="utf-8", errors="replace")
        return self.process.returncode, diagnostics


def copy_fixture(destination: Path) -> Path:
    game = destination / "game"
    shutil.copytree(FIXTURE_ROOT, game)
    return game


def without_response_id(response: dict[str, Any]) -> dict[str, Any]:
    return {key: value for key, value in response.items() if key != "id"}


def inspect_absent_paths(
    directory: Path, game: Path, relative_paths: list[str]
) -> dict[str, list[dict[str, Any]]]:
    result: dict[str, list[dict[str, Any]]] = {}
    for relative in relative_paths:
        candidates = [
            safe_relative_path(game, relative),
            safe_relative_path(directory, relative),
        ]
        result[relative] = [
            {
                "base": "game" if candidate.is_relative_to(game) else "process",
                **file_identity(candidate),
            }
            for candidate in candidates
        ]
    return result


def inspect_case_database(directory: Path, assertion: dict[str, Any]) -> dict[str, Any]:
    pattern = assertion["glob"]
    pattern_path = Path(pattern)
    if pattern_path.is_absolute() or ".." in pattern_path.parts:
        raise ValueError(f"unsafe database glob: {pattern!r}")
    matches = sorted(directory.glob(pattern))
    if len(matches) != 1:
        return {
            "glob": assertion["glob"],
            "matches": [str(path.relative_to(directory)) for path in matches],
            "error": "expected exactly one database",
        }
    path = matches[0]
    table = assertion["table"]
    connection = sqlite3.connect(
        f"{path.resolve(strict=True).as_uri()}?mode=ro", uri=True
    )
    try:
        row_count = connection.execute(
            f"SELECT COUNT(*) FROM {quote_identifier(table)}"
        ).fetchone()[0]
    finally:
        connection.close()
    return {
        "path": str(path.relative_to(directory)),
        "sha256": sha256(path),
        "table": table,
        "rowCount": row_count,
        "expectedRowCount": assertion["rowCount"],
    }


def run_case(
    args: argparse.Namespace,
    case: dict[str, Any],
    seed: int,
    deadline: float,
) -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix=f"snake-sql-{case['id']}-") as temporary:
        directory = Path(temporary)
        game = copy_fixture(directory)
        oracle = OracleProcess(args, directory, deadline, case["id"])
        capabilities: dict[str, Any] | None = None
        load: dict[str, Any] | None = None
        run: dict[str, Any] | None = None
        failure: str | None = None
        try:
            capabilities = oracle.capabilities()
            load = oracle.load(game, seed)
            if not load.get("ok") or load.get("result", {}).get("termination") != "waitingInput":
                raise AssertionError(f"fixture load failed for {case['id']}")
            run = oracle.run(case["entry"], case["watches"])
        except Exception as error:
            failure = f"{type(error).__name__}: {error}"
        finally:
            exit_code, stderr = oracle.close()

        record: dict[str, Any] = {
            "capabilities": without_response_id(capabilities) if capabilities else None,
            "load": without_response_id(load) if load else None,
            "run": without_response_id(run) if run else None,
            "processExitCode": exit_code,
            "stderr": stderr,
            "runnerFailure": failure,
        }
        absent_paths = case.get("absentPaths")
        if absent_paths:
            record["absentPaths"] = inspect_absent_paths(
                directory, game, absent_paths
            )
        sqlite_assertion = case.get("sqliteAfterClose")
        if sqlite_assertion:
            record["sqliteAfterClose"] = inspect_case_database(
                directory, sqlite_assertion
            )
        return record


def assemble_bbas_fixture(
    destination: Path, snake_tw_root: Path, plan: dict[str, Any]
) -> tuple[Path, dict[str, Any]]:
    game = copy_fixture(destination)
    source = safe_relative_path(snake_tw_root, plan["source"])
    if not source.is_file():
        raise FileNotFoundError(source)
    target_source = safe_relative_path(game, "erb/bbas_dataset.erb")
    shutil.copy2(source, target_source)
    identities: dict[str, Any] = {
        plan["source"]: file_identity(source),
    }
    for relative in plan["presentResources"]:
        source_resource = safe_relative_path(snake_tw_root, relative)
        if not source_resource.is_file():
            raise FileNotFoundError(source_resource)
        target = safe_relative_path(game, relative)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source_resource, target)
        identities[relative] = file_identity(source_resource)
    for relative in plan["missingResources"]:
        identities[relative] = file_identity(
            safe_relative_path(snake_tw_root, relative)
        )
        if identities[relative]["exists"]:
            raise AssertionError(f"expected missing BBAS resource exists: {relative}")
    return game, identities


def run_bbas_preflight(
    args: argparse.Namespace,
    plan: dict[str, Any],
    snake_tw_root: Path,
    seed: int,
    deadline: float,
) -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix="snake-sql-bbas-") as temporary:
        directory = Path(temporary)
        game, identities = assemble_bbas_fixture(directory, snake_tw_root, plan)
        oracle = OracleProcess(args, directory, deadline, "bbas-preflight")
        capabilities: dict[str, Any] | None = None
        load: dict[str, Any] | None = None
        run: dict[str, Any] | None = None
        failure: str | None = None
        try:
            capabilities = oracle.capabilities()
            load = oracle.load(game, seed)
            if not load.get("ok") or load.get("result", {}).get("termination") != "waitingInput":
                raise AssertionError("BBAS preflight fixture load failed")
            run = oracle.run(plan["entry"], plan["watches"])
        except Exception as error:
            failure = f"{type(error).__name__}: {error}"
        finally:
            exit_code, stderr = oracle.close()
        return {
            "inputs": identities,
            "capabilities": without_response_id(capabilities) if capabilities else None,
            "load": without_response_id(load) if load else None,
            "run": without_response_id(run) if run else None,
            "processExitCode": exit_code,
            "stderr": stderr,
            "runnerFailure": failure,
        }


def inspect_snake_tw_resources(root: Path, plan: dict[str, Any]) -> dict[str, Any]:
    resources: dict[str, Any] = {}
    for relative in plan["mustExist"]:
        path = safe_relative_path(root, relative)
        resources[relative] = file_identity(path)
        if not resources[relative]["exists"]:
            raise FileNotFoundError(path)
    for relative in plan["mustBeMissing"]:
        path = safe_relative_path(root, relative)
        resources[relative] = file_identity(path)
        if resources[relative]["exists"]:
            raise AssertionError(f"expected missing snake TW resource: {relative}")
    database = safe_relative_path(root, "plugins/qol_data.db")
    if not database.is_file():
        raise FileNotFoundError(database)
    resources["plugins/qol_data.db"]["sqlite"] = inspect_sqlite(
        database, immutable=True
    )
    return resources


def validate_case_record(case: dict[str, Any], record: dict[str, Any]) -> list[str]:
    case_id = case["id"]
    failures: list[str] = []
    if record.get("runnerFailure"):
        failures.append(f"{case_id}: {record['runnerFailure']}")
    if record.get("processExitCode") != 0:
        failures.append(f"{case_id}: oracle exit code {record.get('processExitCode')}")

    load = record.get("load")
    load_result = load.get("result") if isinstance(load, dict) else None
    if (
        not isinstance(load, dict)
        or load.get("ok") is not True
        or not isinstance(load_result, dict)
        or load_result.get("termination") != "waitingInput"
    ):
        failures.append(f"{case_id}: load did not reach waitingInput")

    run = record.get("run")
    run_result = run.get("result") if isinstance(run, dict) else None
    if not isinstance(run, dict) or run.get("ok") is not True:
        failures.append(f"{case_id}: run envelope is not a successful protocol response")
        return failures
    if not isinstance(run_result, dict):
        failures.append(f"{case_id}: run result is not an object")
        return failures
    if run_result.get("termination") != case["expectedTermination"]:
        failures.append(
            f"{case_id}: expected termination {case['expectedTermination']!r}, "
            f"got {run_result.get('termination')!r}"
        )
    watches = run_result.get("watches")
    if not isinstance(watches, dict):
        failures.append(f"{case_id}: watches is not an object")
    else:
        expected_watches = set(case["watches"])
        if set(watches) != expected_watches:
            failures.append(
                f"{case_id}: watch keys differ; expected {sorted(expected_watches)!r}, "
                f"got {sorted(watches)!r}"
            )
        for expression, value in watches.items():
            if isinstance(value, dict) and "error" in value:
                failures.append(f"{case_id}: watch {expression!r} failed to evaluate")

    for relative, candidates in record.get("absentPaths", {}).items():
        if any(candidate.get("exists") for candidate in candidates):
            failures.append(f"{case_id}: unexpected side effect exists at {relative}")

    sqlite_assertion = case.get("sqliteAfterClose")
    if sqlite_assertion:
        database = record.get("sqliteAfterClose")
        if not isinstance(database, dict) or "error" in database:
            failures.append(f"{case_id}: post-close database inspection failed")
        elif database.get("rowCount") != sqlite_assertion["rowCount"]:
            failures.append(
                f"{case_id}: expected {sqlite_assertion['rowCount']} rows after close, "
                f"got {database.get('rowCount')!r}"
            )
    return failures


def capture_failures(capture: dict[str, Any], plan: dict[str, Any]) -> list[str]:
    planned_cases = {case["id"]: case for case in plan["cases"]}
    failures: list[str] = []
    for case_id, record in capture["cases"].items():
        case = planned_cases.get(case_id)
        if case is None:
            failures.append(f"unknown captured case: {case_id}")
            continue
        failures.extend(validate_case_record(case, record))
    bbas = capture.get("bbasPreflight")
    if bbas:
        bbas_case = {
            "id": "bbas-preflight",
            "expectedTermination": plan["bbasPreflight"]["expectedTermination"],
            "watches": plan["bbasPreflight"]["watches"],
        }
        failures.extend(validate_case_record(bbas_case, bbas))
    return failures


def source_location(value: Any) -> dict[str, Any] | None:
    if not isinstance(value, dict):
        return None
    filename = value.get("Filename", value.get("file"))
    line = value.get("LineNo", value.get("line"))
    if isinstance(filename, str):
        filename = Path(filename).name
    if isinstance(line, str) and line.lstrip("-").isdigit():
        line = int(line)
    if filename is None and line is None:
        return None
    return {"file": filename, "line": line}


def diagnostic_locations(response: dict[str, Any]) -> list[dict[str, Any]]:
    projected = []
    for diagnostic in response.get("diagnostics", []):
        if isinstance(diagnostic, dict):
            projected.append(
                {
                    "level": diagnostic.get("level"),
                    "position": source_location(diagnostic.get("position")),
                }
            )
    return projected


def project_case(record: dict[str, Any]) -> dict[str, Any]:
    response = record["run"]
    result = response["result"]
    projected: dict[str, Any] = {
        "ok": response["ok"],
        "schemaVersion": response["schemaVersion"],
        "referenceCommit": response["referenceCommit"],
        "state": result.get("state"),
        "termination": result.get("termination"),
        "watches": result.get("watches"),
        "diagnostics": diagnostic_locations(response),
    }
    if result.get("termination") == "error":
        projected["position"] = source_location(result.get("position"))
    absent = record.get("absentPaths")
    if absent:
        projected["absentPaths"] = {
            relative: [candidate["exists"] for candidate in candidates]
            for relative, candidates in sorted(absent.items())
        }
    database = record.get("sqliteAfterClose")
    if database:
        projected["databaseSideEffect"] = {
            "table": database.get("table"),
            "rowCount": database.get("rowCount"),
        }
    return projected


def project_resources(resources: dict[str, Any]) -> dict[str, Any]:
    projected: dict[str, Any] = {}
    for relative, identity in sorted(resources.items()):
        item = {
            key: identity[key]
            for key in ("exists", "size", "sha256")
            if key in identity
        }
        sqlite = identity.get("sqlite")
        if sqlite:
            item["sqlite"] = {
                key: value
                for key, value in sqlite.items()
                if key != "inspectorSqliteVersion"
            }
        projected[relative] = item
    return projected


def stable_projection(capture: dict[str, Any], plan: dict[str, Any]) -> dict[str, Any]:
    expected_case_ids = {case["id"] for case in plan["cases"]}
    if set(capture["cases"]) != expected_case_ids:
        raise AssertionError("capture does not contain the complete planned case set")
    if capture.get("bbasPreflight") is None or capture.get("snakeTwResources") is None:
        raise AssertionError("capture is missing BBAS or snake TW resource evidence")
    bbas = capture["bbasPreflight"]
    return {
        "formatVersion": capture["formatVersion"],
        "semanticBaseline": capture["semanticBaseline"],
        "implementation": capture["implementation"],
        "referenceExecutable": capture["referenceExecutable"],
        "wrapper": capture["wrapper"],
        "fixture": capture["fixture"],
        "cases": {
            case_id: project_case(record)
            for case_id, record in sorted(capture["cases"].items())
        },
        "bbasPreflight": {
            "inputs": bbas["inputs"],
            "result": project_case(bbas),
        },
        "snakeTwResources": project_resources(capture["snakeTwResources"]),
    }


def verify_capture(capture: dict[str, Any], golden: dict[str, Any], plan: dict[str, Any]) -> None:
    if golden.get("status") != "fixed-snake-reference-golden":
        raise AssertionError("oracle.json is not a fixed golden")
    projection = stable_projection(capture, plan)
    if projection != golden.get("projection"):
        raise AssertionError("stable projection differs from the fixed golden")


def fixture_input_manifest() -> list[dict[str, Any]]:
    relative_paths = [
        "cases.json",
        "emuera.config",
        "setting.json",
    ]
    for directory in ("csv", "erb", "plugins"):
        root = safe_relative_path(FIXTURE_ROOT, directory)
        relative_paths.extend(
            str(path.relative_to(FIXTURE_ROOT))
            for path in root.rglob("*")
            if path.is_file()
        )
    manifest = []
    for relative in sorted(set(relative_paths)):
        path = safe_relative_path(FIXTURE_ROOT, relative)
        identity = file_identity(path)
        if not identity["exists"]:
            raise FileNotFoundError(path)
        manifest.append({"path": relative, **identity})
    return manifest


def wrapper_identity(executable: Path) -> dict[str, Any]:
    repository = Path(
        subprocess.check_output(
            ["git", "-C", str(executable.parent), "rev-parse", "--show-toplevel"],
            text=True,
        ).strip()
    )
    commit = subprocess.check_output(
        ["git", "-C", str(repository), "rev-parse", "HEAD"], text=True
    ).strip()
    dirty_output = subprocess.check_output(
        ["git", "-C", str(repository), "status", "--porcelain"], text=True
    )
    dirty = bool(dirty_output.strip())
    if dirty:
        raise RuntimeError("fixed capture requires a clean snake reference wrapper")
    return {"commit": commit, "dirty": dirty}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path)
    parser.add_argument("--wine")
    parser.add_argument("--winepath", default="winepath")
    parser.add_argument("--snake-tw-root", type=Path)
    parser.add_argument("--mode", choices=("capture", "verify"), required=True)
    parser.add_argument("--capture", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--case", action="append", dest="cases")
    parser.add_argument("--request-timeout", type=float, default=30.0)
    parser.add_argument("--budget-seconds", type=float, default=1800.0)
    args = parser.parse_args()
    if not math.isfinite(args.request_timeout) or args.request_timeout <= 0:
        parser.error("--request-timeout must be positive")
    if not math.isfinite(args.budget_seconds) or args.budget_seconds <= 0:
        parser.error("--budget-seconds must be positive")
    if args.mode == "capture":
        if args.exe is None or args.snake_tw_root is None or args.output is None:
            parser.error("capture requires --exe, --snake-tw-root, and --output")
        if args.capture is not None:
            parser.error("capture does not accept --capture")
        args.exe = args.exe.resolve(strict=True)
        args.snake_tw_root = args.snake_tw_root.resolve(strict=True)
        args.output = args.output.resolve()
    else:
        if args.capture is None:
            parser.error("verify requires --capture")
        if args.exe is not None or args.wine is not None or args.snake_tw_root is not None:
            parser.error("offline verify does not accept --exe, --wine, or --snake-tw-root")
        if args.cases:
            parser.error("offline verify requires the complete capture, not --case")
        args.capture = args.capture.resolve(strict=True)
        if args.output is not None:
            args.output = args.output.resolve()
    return args


def main() -> int:
    args = parse_args()
    plan = read_json(FIXTURE_ROOT / "cases.json")
    if plan.get("semanticBaseline") != SEMANTIC_BASELINE:
        raise AssertionError("cases.json semantic baseline mismatch")
    if args.mode == "verify":
        capture = read_json(args.capture)
        verify_capture(capture, read_json(FIXTURE_ROOT / "oracle.json"), plan)
        if args.output is not None:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(
                json.dumps(
                    stable_projection(capture, plan),
                    ensure_ascii=False,
                    indent=2,
                    sort_keys=True,
                )
                + "\n",
                encoding="utf-8",
            )
        print("PASS fixed snake SQL oracle (offline)", flush=True)
        return 0

    selected = set(args.cases or [])
    known = {case["id"] for case in plan["cases"]} | SHARED_CASES
    unknown = selected - known
    if unknown:
        raise ValueError(f"unknown cases: {sorted(unknown)}")

    wrapper = wrapper_identity(args.exe)
    executable_identity = file_identity(args.exe)
    deadline = time.monotonic() + args.budget_seconds
    cases: dict[str, Any] = {}
    for case in plan["cases"]:
        if selected and case["id"] not in selected:
            continue
        cases[case["id"]] = run_case(
            args, case, plan["seed"], deadline
        )
        print(f"CAPTURED {case['id']}", flush=True)

    include_bbas = not selected or "bbas-preflight" in selected
    include_resources = not selected or "snake-tw-resources" in selected
    capture: dict[str, Any] = {
        "formatVersion": 1,
        "semanticBaseline": SEMANTIC_BASELINE,
        "implementation": IMPLEMENTATION,
        "referenceExecutable": executable_identity,
        "wrapper": wrapper,
        "fixture": {
            "runnerSha256": sha256(Path(__file__).resolve()),
            "inputManifest": fixture_input_manifest(),
        },
        "cases": cases,
        "bbasPreflight": None,
        "snakeTwResources": None,
    }
    if include_bbas:
        capture["bbasPreflight"] = run_bbas_preflight(
            args,
            plan["bbasPreflight"],
            args.snake_tw_root,
            plan["seed"],
            deadline,
        )
        print("CAPTURED bbas-preflight", flush=True)
    if include_resources:
        capture["snakeTwResources"] = inspect_snake_tw_resources(
            args.snake_tw_root, plan["snakeTwResources"]
        )
        print("CAPTURED snake-tw-resources", flush=True)

    failures = capture_failures(capture, plan)
    complete = (
        set(cases) == {case["id"] for case in plan["cases"]}
        and capture["bbasPreflight"] is not None
        and capture["snakeTwResources"] is not None
    )
    if complete and not failures:
        capture["stableProjection"] = stable_projection(capture, plan)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(capture, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    if failures:
        for failure in failures:
            print(f"FAIL {failure}", file=sys.stderr)
        return 1
    print(f"WROTE {args.output}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
