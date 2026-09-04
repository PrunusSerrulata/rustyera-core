#!/usr/bin/env python3
"""Capture, finalize, and verify the fixed snake Batch 5.0 oracle."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import queue
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import threading
import time
from typing import Any
import wave

SNAKE_BASELINE = "fc4fb21416768c17256d0e82f997e5f99c9bba91"
ORIGINAL_BASELINE = "26a35dc9334bb67590b96f7b8efbefbf199e391e"
TOOL_ROOT = Path(__file__).resolve().parent
CORE_ROOT = TOOL_ROOT.parent.parent
FIXTURE_ROOT = TOOL_ROOT / "fixture-snake-batch5-save-audio-oracle"
PLAN_PATH = FIXTURE_ROOT / "cases.json"
GOLDEN_PATH = FIXTURE_ROOT / "oracle.json"
FORMATS = {"binary": (True, False), "gzip": (True, True), "text": (False, False)}
EXPECTED_INPUTS = {
    "sav/global.sav": "56f80b52a8a6c8fc7dd080f9a69967758fb83df966a45330123bbc3d8a1e37cf",
    "sav/save1000.sav": "442b1d41d3d17f2dbfdb6587ae521361bf07174f653affdc0bb82a9693dae0a2",
}
BINARY_MAGIC = b"\x89ERA\r\n\x1a\n"
GZIP_MAGIC = b"\x89ERAZIP\n"
TEXT_MAGIC = "__EMUERA_1808_STRAT__"


def read_object(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def write_object(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def file_identity(path: Path) -> dict[str, Any]:
    if not path.is_file():
        return {"exists": False}
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return {"exists": True, "size": path.stat().st_size, "sha256": digest.hexdigest()}


def git_text(repository: Path, *args: str) -> str:
    return subprocess.check_output(["git", "-C", str(repository), *args], text=True).strip()


def repository_identity(repository: Path, status_digest: bool = False) -> dict[str, Any]:
    root = Path(git_text(repository, "rev-parse", "--show-toplevel"))
    status = subprocess.check_output(["git", "-C", str(root), "status", "--porcelain=v1", "-z"])
    value: dict[str, Any] = {
        "commit": git_text(root, "rev-parse", "HEAD"),
        "branch": git_text(root, "branch", "--show-current"),
        "dirty": bool(status),
    }
    if status_digest:
        value["statusPorcelainV1ZSha256"] = hashlib.sha256(status).hexdigest()
    return value


def wrapper_identity(executable: Path, repository: Path | None = None) -> dict[str, Any]:
    if repository is None:
        repository = Path(git_text(executable.parent, "rev-parse", "--show-toplevel"))
    value = repository_identity(repository)
    if value["dirty"]:
        raise RuntimeError(f"reference wrapper repository is dirty: {repository}")
    return value


def manifest(root: Path, excluded: Path | None = None) -> list[dict[str, Any]]:
    return [
        {"path": str(path.relative_to(root)), **file_identity(path)}
        for path in sorted(root.rglob("*"))
        if path.is_file() and path != excluded
    ]


def source_location(value: Any) -> dict[str, Any] | None:
    if not isinstance(value, dict):
        return None
    filename = value.get("Filename", value.get("file"))
    line = value.get("LineNo", value.get("line"))
    if filename is not None:
        filename = Path(str(filename)).name
    return None if filename is None and line is None else {"file": filename, "line": line}


def project_response(response: dict[str, Any]) -> dict[str, Any]:
    result = response.get("result") if isinstance(response.get("result"), dict) else {}
    error = response.get("error") if isinstance(response.get("error"), dict) else {}
    diagnostics = []
    for item in response.get("diagnostics", []):
        if isinstance(item, dict):
            diagnostics.append({
                "level": item.get("level"), "message": item.get("message"),
                "position": source_location(item.get("position")),
            })
    value: dict[str, Any] = {
        "ok": response.get("ok"),
        "error": {"type": error.get("type"), "message": error.get("message")} if error else None,
        "termination": result.get("termination"), "state": result.get("state"),
        "watches": result.get("watches"), "output": result.get("output"),
        "diagnostics": diagnostics,
    }
    position = source_location(result.get("position"))
    if position:
        value["position"] = position
    return value


class OracleSession:
    """A validated persistent CLI session with a five-second state watchdog."""

    def __init__(self, args: argparse.Namespace, engine: str, directory: Path, case_id: str) -> None:
        self.args = args
        self.engine = engine
        self.directory = directory
        self.case_id = case_id
        self.executable = args.snake_exe if engine == "snake" else args.original_exe
        self.prefix = args.snake_wineprefix if engine == "snake" else args.original_wineprefix
        self.baseline = SNAKE_BASELINE if engine == "snake" else ORIGINAL_BASELINE
        self.environment = os.environ.copy()
        if self.prefix:
            self.environment["WINEPREFIX"] = str(self.prefix)
        self.responses: queue.Queue[str | Exception | None] = queue.Queue()
        self.sequence = 0
        self.last_response: dict[str, Any] | None = None
        self.stderr_path = directory / f"{case_id}-stderr.log"
        self.stderr = self.stderr_path.open("w", encoding="utf-8")
        command = ([args.wine] if args.wine else []) + [str(self.executable)]
        self.process = subprocess.Popen(
            command, cwd=directory, env=self.environment, stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=self.stderr, text=True, encoding="utf-8",
            errors="strict", start_new_session=os.name != "nt",
        )
        threading.Thread(target=self._read_stdout, daemon=True).start()

    def _read_stdout(self) -> None:
        try:
            assert self.process.stdout is not None
            for line in self.process.stdout:
                self.responses.put(line)
        except Exception as error:  # pragma: no cover
            self.responses.put(error)
        finally:
            self.responses.put(None)

    def windows_path(self, path: Path) -> str:
        if not self.args.wine:
            return str(path)
        return subprocess.check_output(
            [self.args.winepath, "-w", str(path)], env=self.environment,
            text=True, timeout=10,
        ).strip()

    def request(self, request: dict[str, Any]) -> dict[str, Any]:
        self.sequence += 1
        wire = {"id": self.sequence, **request}
        assert self.process.stdin is not None
        self.process.stdin.write(json.dumps(wire, ensure_ascii=False) + "\n")
        self.process.stdin.flush()
        deadline = time.monotonic() + self.args.request_timeout
        previous = None
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(f"{self.case_id}: request timeout")
            try:
                line = self.responses.get(timeout=min(5.0, remaining))
            except queue.Empty:
                snapshot = {
                    "case": self.case_id, "alive": self.process.poll() is None,
                    "pending": wire, "lastFullResponse": self.last_response,
                }
                print("WATCHDOG " + json.dumps(snapshot, sort_keys=True), file=sys.stderr, flush=True)
                if snapshot == previous:
                    raise TimeoutError(f"{self.case_id}: identical watchdog snapshots")
                previous = snapshot
                continue
            if line is None:
                raise RuntimeError(f"{self.case_id}: oracle exited before response")
            if isinstance(line, Exception):
                raise line
            response = json.loads(line)
            if not isinstance(response, dict) or response.get("id") != self.sequence:
                raise AssertionError(f"{self.case_id}: invalid response envelope")
            if response.get("schemaVersion") != 2 or response.get("referenceCommit") != self.baseline:
                raise AssertionError(f"{self.case_id}: wrong oracle identity")
            if not isinstance(response.get("diagnostics"), list):
                raise AssertionError(f"{self.case_id}: invalid diagnostics")
            self.last_response = response
            return response

    def capabilities(self) -> dict[str, Any]:
        response = self.request({"op": "capabilities"})
        if not response.get("ok") or not isinstance(response.get("result"), dict):
            raise AssertionError(f"{self.case_id}: capabilities failed")
        return response

    def load(self, game: Path) -> dict[str, Any]:
        return self.request({"op": "load", "gameDir": self.windows_path(game), "seed": 5050})

    def run(self, entry: str, watches: list[str]) -> dict[str, Any]:
        return self.request({"op": "run", "entry": entry, "watch": watches})

    def close(self) -> tuple[int, str]:
        if self.process.stdin:
            self.process.stdin.close()
        try:
            self.process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            if os.name == "nt":
                subprocess.run(["taskkill", "/PID", str(self.process.pid), "/T", "/F"], check=False)
            else:
                try:
                    os.killpg(self.process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            self.process.wait(timeout=3)
        if self.process.stdout:
            self.process.stdout.close()
        self.stderr.close()
        return self.process.returncode, self.stderr_path.read_text(encoding="utf-8", errors="replace")


def close_cleanly(session: OracleSession) -> None:
    code, stderr = session.close()
    if code != 0:
        raise AssertionError(f"{session.case_id}: oracle exited {code}: {stderr}")


def assert_success(response: dict[str, Any], case_id: str) -> None:
    result = response.get("result")
    if not response.get("ok") or not isinstance(result, dict) or result.get("termination") == "error":
        raise AssertionError(f"{case_id}: {project_response(response)}")


def configure_format(game: Path, binary: bool, compressed: bool) -> None:
    path = game / "emuera.config"
    value = path.read_text(encoding="utf-8")
    replacements = {
        "Use the binary format for saving data:YES": f"Use the binary format for saving data:{'YES' if binary else 'NO'}",
        "Compress save data:NO": f"Compress save data:{'YES' if compressed else 'NO'}",
    }
    for before, after in replacements.items():
        if value.count(before) != 1:
            raise AssertionError(f"config token must occur once: {before}")
        value = value.replace(before, after)
    path.write_text(value, encoding="utf-8")


def original_config(format_name: str) -> str:
    binary, compressed = FORMATS[format_name]
    return "\n".join([
        "描画インターフェース:TEXTRENDERER",
        "ロード時にレポートを表示する:NO",
        "ロード時に引数を解析する:NO",
        "呼び出されなかった関数を無視する:NO",
        "読み込み順をファイル名順にソートする:YES",
        f"セーブデータをバイナリ形式で保存する:{'YES' if binary else 'NO'}",
        f"セーブデータを圧縮して保存する:{'YES' if compressed else 'NO'}",
        "セーブデータをsavフォルダ内に作成する:YES",
        "LOADTEXTとSAVETEXTで使える拡張子:txt,xml",
        "EMUERAの表示言語:English",
        "",
    ])


def assemble_game(directory: Path, format_name: str, shared: bool,
                  engine: str = "snake") -> Path:
    game = directory / "game"
    if format_name == "text":
        (game / "csv").mkdir(parents=True)
        (game / "erb").mkdir()
        for name in ("emuera.config", "setting.json"):
            shutil.copy2(FIXTURE_ROOT / name, game / name)
        for name in ("GAMEBASE.CSV", "CHARA0.CSV"):
            shutil.copy2(FIXTURE_ROOT / "csv" / name, game / "csv" / name)
        overlay = FIXTURE_ROOT / "overlays/text"
        shutil.copy2(overlay / "batch5.erh", game / "erb/batch5.erh")
        shutil.copy2(overlay / "batch5.erb", game / "erb/batch5.erb")
    elif not shared:
        shutil.copytree(FIXTURE_ROOT, game, ignore=shutil.ignore_patterns("oracle.json", "overlays"))
    else:
        (game / "csv").mkdir(parents=True)
        (game / "erb").mkdir()
        for name in ("emuera.config", "setting.json"):
            shutil.copy2(FIXTURE_ROOT / name, game / name)
        for name in ("GAMEBASE.CSV", "CHARA0.CSV"):
            shutil.copy2(FIXTURE_ROOT / "csv" / name, game / "csv" / name)
        overlay = FIXTURE_ROOT / "overlays/original"
        shutil.copy2(overlay / "batch5.erh", game / "erb/batch5.erh")
        shutil.copy2(overlay / "batch5.erb", game / "erb/batch5.erb")
    if engine == "original":
        (game / "emuera.config").write_text(original_config(format_name), encoding="utf-8")
    else:
        configure_format(game, *FORMATS[format_name])
    return game


def inspect_save(path: Path) -> dict[str, Any]:
    value = file_identity(path)
    data = path.read_bytes()
    if data.startswith(BINARY_MAGIC) or data.startswith(GZIP_MAGIC):
        value.update({
            "codec": "gzip" if data.startswith(GZIP_MAGIC) else "binary",
            "magicHex": data[:8].hex(), "version": struct.unpack("<I", data[8:12])[0],
            "dataCount": struct.unpack("<I", data[12:16])[0],
        })
    else:
        text = data.decode("utf-8-sig")
        value.update({
            "codec": "text", "marker": TEXT_MAGIC if TEXT_MAGIC in text else None,
            "version": 1808 if TEXT_MAGIC in text else None,
        })
    return value


def assert_codec(value: dict[str, Any], format_name: str) -> None:
    if value.get("codec") != format_name or value.get("version") != 1808:
        raise AssertionError(f"expected {format_name}/1808, got {value}")


def run_once(args: argparse.Namespace, engine: str, directory: Path, game: Path,
             entry: str, watches: list[str], case_id: str) -> dict[str, Any]:
    session = OracleSession(args, engine, directory, case_id)
    try:
        session.capabilities()
        assert_success(session.load(game), f"{case_id}-load")
        response = session.run(entry, watches)
        assert_success(response, case_id)
        return project_response(response)
    finally:
        close_cleanly(session)


def load_save_once(args: argparse.Namespace, engine: str, directory: Path, game: Path,
                   save: Path, watches: list[str], case_id: str,
                   expected_failure: bool = False) -> dict[str, Any]:
    session = OracleSession(args, engine, directory, case_id)
    try:
        session.capabilities()
        assert_success(session.load(game), f"{case_id}-load")
        response = session.request({"op": "loadSave", "savePath": session.windows_path(save), "watch": watches})
        if expected_failure:
            if response.get("ok") and response.get("result", {}).get("termination") != "error":
                raise AssertionError(f"{case_id}: expected reference refusal was accepted")
        else:
            assert_success(response, case_id)
        return project_response(response)
    finally:
        close_cleanly(session)


def capture_save_pair(args: argparse.Namespace, engine: str, format_name: str,
                      shared: bool) -> tuple[dict[str, Any], bytes, bytes]:
    plan = read_object(PLAN_PATH)
    with tempfile.TemporaryDirectory(prefix=f"b5-{engine}-{format_name}-") as temporary:
        directory = Path(temporary)
        game = assemble_game(directory, format_name, shared, engine)
        entries = (
            ("B5_TEXT_WRITE_NORMAL", "B5_TEXT_WRITE_GLOBAL", "B5_TEXT_READ_GLOBAL")
            if format_name == "text" else
            (("B5_SHARED_WRITE_NORMAL", "B5_SHARED_WRITE_GLOBAL", "B5_SHARED_READ_GLOBAL")
             if shared else ("B5_WRITE_NORMAL", "B5_WRITE_GLOBAL", "B5_READ_GLOBAL"))
        )
        normal_write = run_once(args, engine, directory, game, entries[0], ["RESULT"], "normal-write")
        global_write = run_once(args, engine, directory, game, entries[1], ["RESULT"], "global-write")
        normal_result = normal_write.get("watches", {}).get("RESULT")
        if engine == "snake" and format_name == "text":
            if normal_result != 0 or "unexpected error occurred while saving" not in str(normal_write.get("output", "")).lower():
                raise AssertionError("snake Text normal write no longer matches its fixed failure")
        elif not isinstance(normal_result, int):
            raise AssertionError(f"{engine} {format_name} normal save did not expose RESULT")
        if not isinstance(global_write.get("watches", {}).get("RESULT"), int):
            raise AssertionError(f"{engine} {format_name} GLOBAL save did not expose RESULT")
        normal_path, global_path = game / "sav/save00.sav", game / "sav/global.sav"
        if args.only_format == format_name:
            shutil.copy2(normal_path, args.output.parent / f"debug-{engine}-{format_name}-save00.sav")
            shutil.copy2(global_path, args.output.parent / f"debug-{engine}-{format_name}-global.sav")
        normal_artifact, global_artifact = inspect_save(normal_path), inspect_save(global_path)
        assert_codec(normal_artifact, format_name)
        assert_codec(global_artifact, format_name)
        ordinary_watches = (
            plan["textOrdinaryWatches"] if format_name == "text"
            else (plan["ordinaryWatches"][:10] if shared else plan["ordinaryWatches"])
        )
        global_watches = (
            plan["textGlobalWatches"] if format_name == "text"
            else (plan["globalWatches"][:10] if shared else plan["globalWatches"])
        )
        normal_load = load_save_once(
            args, engine, directory, game, normal_path, ordinary_watches, "normal-load",
            expected_failure=(engine == "snake" and format_name == "text"),
        )
        global_load = run_once(args, engine, directory, game, entries[2], global_watches, "global-load")
        value = {
            "normalWrite": normal_write, "normalArtifact": normal_artifact, "normalLoad": normal_load,
            "globalWrite": global_write, "globalArtifact": global_artifact, "globalLoad": global_load,
        }
        return value, normal_path.read_bytes(), global_path.read_bytes()


def locate_tag(data: bytes, key: str) -> int:
    encoded = key.encode("utf-16-le")
    offset = data.find(encoded)
    if offset < 2 or data[offset - 1] != len(encoded):
        raise ValueError(f"unexpected key encoding for {key}")
    return offset - 2


def damaged_payloads(binary: bytes, compressed: bytes) -> dict[str, tuple[bytes, str]]:
    unknown = bytearray(binary)
    unknown[locate_tag(binary, "B5_INT")] = 0x08
    bomb = GZIP_MAGIC + struct.pack("<II", 1808, 0) + gzip.compress(b"\0" * (16 * 1024 * 1024 + 1), 9)
    return {
        "bad-header": (b"BAD-SAVE" + binary[8:], "invalid-header"),
        "truncated-header": (binary[:12], "truncated-header"),
        "truncated-binary-body": (binary[:-7], "truncated-body"),
        "truncated-gzip-body": (compressed[: len(compressed) // 2], "truncated-compressed-body"),
        "unknown-tag-0x08": (bytes(unknown), "unknown-type-tag"),
        "zip-bomb-16mib-plus-one": (bomb, "decompressed-bytes-limit"),
    }


def capture_damaged(args: argparse.Namespace, binary: bytes, compressed: bytes) -> dict[str, Any]:
    plan = read_object(PLAN_PATH)
    records = {}
    for case_id, (payload, rust_target) in damaged_payloads(binary, compressed).items():
        with tempfile.TemporaryDirectory(prefix=f"b5-{case_id}-") as temporary:
            directory = Path(temporary)
            game = assemble_game(directory, "binary", False, "snake")
            path = directory / f"{case_id}.sav"
            path.write_bytes(payload)
            session = OracleSession(args, "snake", directory, case_id)
            try:
                session.capabilities()
                assert_success(session.load(game), f"{case_id}-load")
                before = session.run("B5_SENTINEL", plan["sentinelWatches"])
                assert_success(before, f"{case_id}-sentinel")
                refused = session.request({"op": "loadSave", "savePath": session.windows_path(path), "watch": plan["sentinelWatches"]})
                if refused.get("ok") and refused.get("result", {}).get("termination") != "error":
                    raise AssertionError(f"{case_id}: damaged input accepted")
                after = session.request({"op": "observe", "watch": plan["sentinelWatches"]})
                if not after.get("ok"):
                    raise AssertionError(f"{case_id}: wrapper failed to recover")
            finally:
                code, stderr = session.close()
            if code != 0:
                raise AssertionError(f"{case_id}: wrapper crashed: {stderr}")
            records[case_id] = {
                "artifact": file_identity(path), "referenceActual": project_response(refused),
                "stateBefore": project_response(before), "stateAfter": project_response(after),
                "wrapperRecovered": True, "processExitCode": code, "rustBatch5Target": rust_target,
            }
    return records


def capture_float(args: argparse.Namespace) -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix="b5-float-") as temporary:
        directory = Path(temporary)
        game = assemble_game(directory, "binary", False, "snake")
        shutil.copy2(FIXTURE_ROOT / "overlays/float.erh", game / "erb/float.erh")
        shutil.copy2(FIXTURE_ROOT / "overlays/float.erb", game / "erb/float.erb")
        written = run_once(args, "snake", directory, game, "B5_WRITE_FLOAT", ["RESULT"], "float-write")
        save = game / "sav/save00.sav"
        loaded = load_save_once(args, "snake", directory, game, save, ["B5_FLOAT"], "float-load")
        return {
            "referenceActual": "accepted", "rustBatch5Target": "reject-unsupported-float-tag",
            "write": written, "artifact": inspect_save(save), "load": loaded,
        }


def capture_language(args: argparse.Namespace, engine: str) -> list[dict[str, Any]]:
    plan = read_object(PLAN_PATH)
    with tempfile.TemporaryDirectory(prefix=f"b5-language-{engine}-") as temporary:
        directory = Path(temporary)
        game = assemble_game(directory, "binary", engine == "original", engine)
        session = OracleSession(args, engine, directory, f"language-{engine}")
        records = []
        try:
            session.capabilities()
            assert_success(session.load(game), f"language-{engine}-load")
            for source in plan["languageCases"]:
                response = session.request({"op": "analyzeLine", "source": source})
                records.append({"source": source, "result": project_response(response)})
        finally:
            close_cleanly(session)
        return records


def command_version(command: list[str]) -> str:
    result = subprocess.run(command, text=True, capture_output=True, check=True)
    return (result.stdout or result.stderr).strip()


def workspace_identities() -> dict[str, Any]:
    return {
        name: repository_identity(CORE_ROOT.parent / directory)
        for name, directory in {"core": "rustyera-core", "tui": "rustyera-tui", "web": "rustyera-web"}.items()
    }


def snake_tw_inputs(root: Path) -> dict[str, Any]:
    saves = {}
    for relative, digest in EXPECTED_INPUTS.items():
        value = inspect_save(root / relative)
        if value.get("sha256") != digest:
            raise AssertionError(f"snake TW digest mismatch: {relative}")
        assert_codec(value, "gzip")
        saves[relative] = value
    setting = read_object(root / "setting.json")
    if setting.get("UseNewRandom") is not True:
        raise AssertionError("UseNewRandom must be true")
    return {
        "repository": repository_identity(root, True), "saves": saves,
        "setting": {"identity": file_identity(root / "setting.json"), "UseNewRandom": True},
    }


def source_semantics(args: argparse.Namespace) -> dict[str, Any]:
    root = Path(git_text(args.snake_exe.parent, "rev-parse", "--show-toplevel"))
    paths = [root / "Emuera/Runtime/Script/Statements/Function/Creator.Method.cs", root / "Emuera/Runtime/Utils/Sound.cs"]
    return {
        "files": [{"path": str(path.relative_to(root)), **file_identity(path)} for path in paths],
        "pitchFlagZeroOrOmitted": "preserve-pitch", "pitchFlagNonzero": "do-not-preserve-pitch",
        "seekAction": "absent-from-ERB-API",
        "evidenceBoundary": "GUI proves calls succeed; fixed source proves pitch-flag mapping",
    }


def capture(args: argparse.Namespace) -> None:
    plan = read_object(PLAN_PATH)
    snake_formats, payloads = {}, {}
    format_names = [args.only_format] if args.only_format else plan["formats"]
    for name in format_names:
        checkpoint = args.output.parent / f"checkpoint-snake-{name}.json"
        if name in args.reuse_format:
            stored = read_object(checkpoint)
            if stored.get("format") != name or stored.get("engine") != "snake":
                raise AssertionError(f"invalid checkpoint for {name}")
            snake_formats[name] = stored["result"]
        else:
            snake_formats[name], payloads[name], _ = capture_save_pair(args, "snake", name, False)
        write_object(
            checkpoint,
            {"formatVersion": 2, "engine": "snake", "format": name, "result": snake_formats[name]},
        )
        print(f"CAPTURED snake {name}", flush=True)
    if args.only_format:
        write_object(args.output, {"formatVersion": 2, "snakeFormats": snake_formats})
        print(f"PARTIAL CAPTURE WRITTEN {args.output}")
        return
    shared = {}
    for engine in ("snake", "original"):
        shared[engine] = {}
        for name in ("binary", "text"):
            if engine == "snake" and name == "text":
                shared[engine][name] = snake_formats["text"]
            else:
                shared[engine][name], _, _ = capture_save_pair(args, engine, name, True)
    write_object(args.output.parent / "checkpoint-shared-original.json", shared)
    with tempfile.TemporaryDirectory(prefix="b5-audio-headless-") as temporary:
        directory = Path(temporary)
        game = assemble_game(directory, "binary", False, "snake")
        audio = run_once(args, "snake", directory, game, "B5_AUDIO_HEADLESS", plan["audioWatches"], "audio-headless")
    for name in ("binary", "gzip"):
        if name in payloads:
            continue
        with tempfile.TemporaryDirectory(prefix=f"b5-damage-source-{name}-") as temporary:
            directory = Path(temporary)
            game = assemble_game(directory, name, False, "snake")
            run_once(args, "snake", directory, game, "B5_WRITE_NORMAL", ["RESULT"], f"damage-source-{name}")
            payloads[name] = (game / "sav/save00.sav").read_bytes()
    result = {
        "formatVersion": 2,
        "semanticBaselines": {"snake": SNAKE_BASELINE, "original": ORIGINAL_BASELINE},
        "referenceExecutables": {
            "snake": {"file": file_identity(args.snake_exe), "wrapper": wrapper_identity(args.snake_exe)},
            "original": {"file": file_identity(args.original_exe), "wrapper": wrapper_identity(args.original_exe, args.original_root)},
        },
        "fixtureManifest": manifest(FIXTURE_ROOT, GOLDEN_PATH),
        "driverManifest": [{"path": Path(__file__).name, **file_identity(Path(__file__))}],
        "workspace": workspace_identities(), "snakeTw": snake_tw_inputs(args.snake_tw_root),
        "toolchain": {
            "platform": platform.platform(), "python": platform.python_version(),
            "rustc": command_version(["rustc", "-Vv"]), "cargo": command_version(["cargo", "-V"]),
            "dotnet": command_version(["dotnet", "--version"]), "node": command_version(["node", "--version"]),
            "npm": command_version(["npm", "--version"]),
            "wine": command_version([args.wine, "--version"]) if args.wine else None,
        },
        "snakeFormats": snake_formats, "sharedOriginalComparison": shared,
        "languageSignatures": {engine: capture_language(args, engine) for engine in ("snake", "original")},
        "float": capture_float(args), "damaged": capture_damaged(args, payloads["binary"], payloads["gzip"]),
        "audioHeadless": audio, "audioSourceSemantics": source_semantics(args),
    }
    write_object(args.output, result)
    print(f"CAPTURE WRITTEN {args.output}")


def write_tone(path: Path, duration_ms: int, frequency: float) -> None:
    sample_rate = 44_100
    with wave.open(str(path), "wb") as output:
        output.setnchannels(1)
        output.setsampwidth(2)
        output.setframerate(sample_rate)
        for index in range(sample_rate * duration_ms // 1000):
            output.writeframesraw(struct.pack("<h", int(math.sin(2 * math.pi * frequency * index / sample_rate) * 8000)))


def prepare_gui(args: argparse.Namespace) -> None:
    if args.output.exists():
        raise FileExistsError(args.output)
    shutil.copytree(FIXTURE_ROOT, args.output, ignore=shutil.ignore_patterns("oracle.json", "overlays"))
    sound = args.output / "sound"
    sound.mkdir()
    write_tone(sound / "batch5-long.wav", 4000, 440)
    write_tone(sound / "batch5-short.wav", 900, 660)
    path = args.output / "erb/batch5.erb"
    value = path.read_text(encoding="utf-8")
    before = "@SYSTEM_TITLE\nPRINTL BATCH5_READY\nINPUT\nRETURN"
    if value.count(before) != 1:
        raise AssertionError("GUI SYSTEM_TITLE overlay must match once")
    path.write_text(value.replace(before, "@SYSTEM_TITLE\nCALL B5_GUI_AUDIO\nRETURN"), encoding="utf-8")
    print(f"GUI PROJECT READY {args.output}")


def audit_prefix(args: argparse.Namespace) -> None:
    prefix, source, forbidden = args.prefix.resolve(), args.source_prefix.resolve(), args.forbidden_root.resolve()
    if prefix == source:
        raise AssertionError("source Wine prefix cannot be reused directly")
    links, forbidden_links = [], []
    for root, directories, files in os.walk(prefix, followlinks=False):
        root_path = Path(root)
        for name in list(directories) + files:
            path = root_path / name
            if path.is_symlink():
                resolved = path.resolve(strict=False)
                item = {"path": str(path.relative_to(prefix)), "target": os.readlink(path), "resolved": str(resolved)}
                links.append(item)
                if resolved == forbidden or resolved.is_relative_to(forbidden):
                    forbidden_links.append(item)
        directories[:] = [name for name in directories if not (root_path / name).is_symlink()]
    copied = prefix / "drive_c/eratw-sub-modding"
    if copied.exists() or copied.is_symlink() or forbidden_links:
        raise AssertionError("isolated prefix contains snake TW link")
    write_object(args.output, {
        "schemaVersion": 1, "sourcePrefixReused": False, "forbiddenCopiedPathAbsent": True,
        "forbiddenResolvedLinks": forbidden_links, "symlinks": sorted(links, key=lambda item: item["path"]), "passed": True,
    })
    print(f"WINE PREFIX AUDIT PASS {args.output}")


def parse_gui(path: Path) -> dict[str, int | list[int]]:
    values: dict[str, int | list[int]] = {}
    for line in path.read_text(encoding="utf-8-sig").splitlines():
        line = line.strip()
        if not line:
            continue
        if line.startswith("channels="):
            values["channels"] = [int(item) for item in line.partition("=")[2].split(",")]
        else:
            for item in line.split(","):
                key, separator, raw = item.partition("=")
                if not separator:
                    raise ValueError(f"invalid GUI field: {item}")
                values[key] = int(raw)
    return values


def validate_gui(values: dict[str, int | list[int]]) -> dict[str, Any]:
    exact = {
        "invalid_get": 0, "invalid_is": -1, "invalid_control": -1, "invalid_action": -2,
        "playing": 0, "volume": 37, "pause_ret": 1, "paused_is": -1,
        "resume_ret": 1, "resumed_is": 0, "rate_omitted": 1, "speed": 250,
        "pitch_zero": 1, "pitch_nonzero": 1, "speed_low": 10, "speed_high": 1000,
        "stop_ret": 1, "stopped": -1, "bgm_playing": 1, "bgm_pause_ret": 1,
        "bgm_paused": 0, "bgm_resume_ret": 1, "bgm_rate_omitted": 1,
        "bgm_pitch_zero": 1, "bgm_pitch_nonzero": 1, "bgm_resumed": 1,
        "bgm_stop_ret": 1, "bgm_stopped": 0,
    }
    failures = {key: {"expected": expected, "actual": values.get(key)} for key, expected in exact.items() if values.get(key) != expected}
    if values.get("channels") != list(range(10)):
        failures["channels"] = {"expected": list(range(10)), "actual": values.get("channels")}
    integer = lambda key: int(values.get(key, -1))
    relations = {
        "playingPositionPositive": integer("sound_play_position") > 0,
        "soundDuration": 1900 <= integer("sound_play_duration") <= 2100,
        "omittedReturnMatchesDuration": integer("omitted") == integer("r0") and integer("r0") >= 1900,
        "omittedFields": integer("r1") > 0 and integer("r2") == 1 and integer("r3") == 37 and integer("r4") == 100,
        "pauseStable": abs(integer("pause_pos_b") - integer("pause_pos_a")) <= 50,
        "resumeContinues": integer("resume_pos") > integer("pause_pos_b"),
        "stopReset": integer("stopped_duration") == 0 and integer("stopped_position") == 0,
        "allBusyOverwritesZero": 400 <= integer("all_busy_overwrite_0_duration") <= 500,
        "pausedChannelReused": 400 <= integer("paused_reused_3_duration") <= 500,
        "bgmPauseStable": abs(integer("pos_b") - integer("pos_a")) <= 50,
        "bgmDuration": 1900 <= integer("bgm_duration") <= 2100,
        "bgmSpeed": integer("bgm_speed") == 250,
    }
    failed_relations = [key for key, passed in relations.items() if not passed]
    if failures or failed_relations:
        raise AssertionError(f"GUI validation failed: exact={failures}, relations={failed_relations}")
    return {"status": "passed", "exact": exact, "relations": relations}


def finalize(args: argparse.Namespace) -> None:
    capture_value = read_object(args.capture)
    process, audit, observation = read_object(args.gui_process), read_object(args.prefix_audit), read_object(args.gui_observation)
    if process.get("exitCode") != 0 or process.get("markerObserved") is not True:
        raise AssertionError("GUI process did not complete normally")
    if audit.get("passed") is not True or audit.get("forbiddenResolvedLinks"):
        raise AssertionError("prefix audit failed")
    if observation.get("completeObservableState") is not True:
        raise AssertionError("GUI observation is incomplete")
    capture_value["audioGui"] = {
        "resultFile": file_identity(args.gui_result), "process": process,
        "prefixAudit": {"file": file_identity(args.prefix_audit), "passed": True},
        "observation": {"file": file_identity(args.gui_observation), "completeObservableState": True},
        "semanticVerdict": validate_gui(parse_gui(args.gui_result)),
        "pitchEvidenceBoundary": capture_value["audioSourceSemantics"]["evidenceBoundary"],
    }
    write_object(args.output, capture_value)
    print(f"FINALIZED CAPTURE {args.output}")


def validate_shape(value: dict[str, Any]) -> None:
    plan = read_object(PLAN_PATH)
    if value.get("formatVersion") != 2 or set(value.get("snakeFormats", {})) != set(plan["formats"]):
        raise AssertionError("capture schema or format cases are incomplete")
    for engine in ("snake", "original"):
        if [item.get("source") for item in value.get("languageSignatures", {}).get(engine, [])] != plan["languageCases"]:
            raise AssertionError(f"{engine} language cases are incomplete")
    if "audioGui" not in value:
        raise AssertionError("GUI evidence missing")


def verify(args: argparse.Namespace) -> None:
    capture_value, golden = read_object(args.capture), read_object(GOLDEN_PATH)
    validate_shape(capture_value)
    if capture_value.get("fixtureManifest") != manifest(FIXTURE_ROOT, GOLDEN_PATH):
        raise AssertionError("current fixture differs from capture")
    if capture_value.get("driverManifest") != [{"path": Path(__file__).name, **file_identity(Path(__file__))}]:
        raise AssertionError("current driver differs from capture")
    if golden.get("schemaVersion") != 1 or golden.get("status") != "fixed-snake-reference-golden":
        raise AssertionError("invalid fixed golden")
    if capture_value != golden.get("projection"):
        raise AssertionError("capture differs from fixed golden")
    print("PASS snake Batch 5.0 oracle (offline)")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("capture", "prepare-gui", "audit-wine-prefix", "finalize", "verify"), required=True)
    for name in ("snake-exe", "original-exe", "original-root", "snake-wineprefix", "original-wineprefix", "snake-tw-root", "capture", "output", "prefix", "source-prefix", "forbidden-root", "gui-result", "gui-process", "gui-observation", "prefix-audit"):
        parser.add_argument(f"--{name}", type=Path)
    parser.add_argument("--wine")
    parser.add_argument("--winepath", default="winepath")
    parser.add_argument("--request-timeout", type=float, default=30)
    parser.add_argument("--only-format", choices=tuple(FORMATS))
    parser.add_argument("--reuse-format", action="append", choices=tuple(FORMATS), default=[])
    args = parser.parse_args()
    if not math.isfinite(args.request_timeout) or args.request_timeout <= 0:
        parser.error("--request-timeout must be positive")
    required = {
        "capture": ("snake_exe", "original_exe", "original_root", "snake_wineprefix", "original_wineprefix", "snake_tw_root", "output"),
        "prepare-gui": ("output",),
        "audit-wine-prefix": ("prefix", "source_prefix", "forbidden_root", "output"),
        "finalize": ("capture", "gui_result", "gui_process", "gui_observation", "prefix_audit", "output"),
        "verify": ("capture",),
    }[args.mode]
    missing = [name for name in required if getattr(args, name) is None]
    if missing:
        parser.error("missing " + ", ".join("--" + name.replace("_", "-") for name in missing))
    for name in required:
        path = getattr(args, name)
        setattr(args, name, path.resolve() if name == "output" else path.resolve(strict=True))
    return args


def main() -> int:
    args = parse_args()
    actions = {"capture": capture, "prepare-gui": prepare_gui, "audit-wine-prefix": audit_prefix, "finalize": finalize, "verify": verify}
    actions[args.mode](args)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
