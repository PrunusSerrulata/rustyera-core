#!/usr/bin/env python3
"""Run core-owned compatibility fixtures against one pinned, explicitly selected oracle.

This driver does not build, install fonts, change baselines, or claim Rust parity.
Run it only after the batch's shared review and static gates have passed.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import shutil
import signal
import subprocess
import threading
import time

from comparison import compare_case, validate_rust_evidence


FIXTURE = Path(__file__).resolve().parents[1] / "runtime-tester/fixture-snake-compatibility"


def subset(actual, expected, path="response"):
    if isinstance(expected, dict):
        if not isinstance(actual, dict):
            raise AssertionError(f"{path}: expected object, got {actual!r}")
        for key, value in expected.items():
            if key not in actual:
                raise AssertionError(f"{path}.{key}: missing")
            subset(actual[key], value, f"{path}.{key}")
    elif actual != expected:
        raise AssertionError(f"{path}: expected {expected!r}, got {actual!r}")


def validate_load(response, expected=None):
    subset(response, {"ok": True})
    termination = response.get("result", {}).get("termination")
    expected_termination = (expected or {}).get("result", {}).get("termination")
    if (termination in {"error", "timeout", "instructionLimit", "quit"}
            and expected_termination != termination):
        raise AssertionError("oracle failed during fixture loading")
    if expected is not None:
        subset(response, expected)


def step_expectations(step, response, oracle):
    if "expect" in step:
        subset(response, step["expect"][oracle])
    rejection = step.get("expectedRejection", {}).get(oracle)
    if rejection is None:
        return []
    if response.get("ok") is not False and response.get("result", {}).get("termination") != "error":
        raise AssertionError("an expected operation rejection succeeded")
    return [{"kind": "expected_rejection", "reason": rejection,
             "diagnosticComparison": "incomparable_schema"}]


def load_observation_options(logical_only, selected, font, font_path):
    if logical_only:
        if any(case.get("group") == "PRINTC" or "presentation" in case.get("assertions", [])
               or any(step["request"].get("observePresentation") for step in case["requests"])
               for case in selected):
            raise ValueError("logical-output-only cannot run presentation assertions")
        return {"observePresentation": False}
    return {
        "presentationFont": {**{key: font[key] for key in ["family", "sha256"]}, "file": font_path},
        "observePresentation": True,
    }


def identity(directory):
    files = []
    for path in sorted(directory.rglob("*")):
        if path.is_file():
            data = path.read_bytes()
            files.append(
                {
                    "path": path.relative_to(directory).as_posix(),
                    "bytes": len(data),
                    "sha256": hashlib.sha256(data).hexdigest(),
                }
            )
    encoded = json.dumps(files, sort_keys=True, separators=(",", ":")).encode()
    return {"sha256": hashlib.sha256(encoded).hexdigest(), "files": files}


def comparison_snapshot(snapshot):
    """Ignore only the NDJSON envelope ID, never a script field named id."""
    result = dict(snapshot)
    for field in ("request", "lastAvailableResponse"):
        value = result.get(field)
        if isinstance(value, dict):
            result[field] = {key: item for key, item in value.items() if key != "id"}
    return result


def prepare_case_game(template, output, ordinal, expected_identity):
    """Keep one case's saves and overlays out of every later case's initial state."""
    game = output / "case-games" / f"{ordinal:04d}"
    shutil.copytree(template, game)
    actual = identity(game)
    if actual != expected_identity:
        raise ValueError("effective fixture changed before case loading")
    return game


class Oracle:
    def __init__(self, args, baseline, deadline, game_directory):
        self.args, self.baseline, self.deadline = args, baseline, deadline
        self.responses = queue.Queue()
        self.sequence = 0
        self.last_response = None
        self.records = []
        self.case = "setup"
        self.pending_request = None
        self.closed = threading.Event()
        self.watchdog_failure = None
        env = os.environ.copy()
        if args.wine:
            env.update(WINEPREFIX=str(args.wine_prefix), WINEDEBUG="-all")
        self.env = env
        self.stderr = (args.output / f"stderr-{game_directory.name}.log").open("w", encoding="utf-8")
        command = ([args.wine] if args.wine else []) + [str(args.exe)]
        self.process = subprocess.Popen(
            command,
            cwd=game_directory,
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=self.stderr,
            text=True,
            encoding="utf-8",
            start_new_session=os.name != "nt",
        )
        threading.Thread(target=self._read, daemon=True).start()
        self.watchdog_thread = threading.Thread(target=self._watch, daemon=True)
        self.watchdog_thread.start()

    def _watch(self):
        previous = None
        next_sample = time.monotonic() + 5
        while not self.closed.wait(0.05):
            now = time.monotonic()
            if now < next_sample and now < self.deadline:
                continue
            current = self.snapshot(self.pending_request)
            print(json.dumps({"watchdog": current}, ensure_ascii=False), flush=True)
            compared = comparison_snapshot(current)
            failure = "oracle budget exhausted" if now >= self.deadline else (
                "unchanged complete observations at consecutive 5s samples" if compared == previous else None
            )
            if failure:
                self.watchdog_failure = failure
                try:
                    self.kill()
                except OSError as error:
                    self.watchdog_failure += f"; process cleanup failed: {error}"
                self.responses.put(TimeoutError(self.watchdog_failure))
                return
            previous = compared
            next_sample += 5

    def kill(self):
        if os.name == "nt":
            if self.process.poll() is None:
                self.process.kill()
        else:
            try:
                os.killpg(self.process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass

    def _read(self):
        try:
            for line in self.process.stdout:
                self.responses.put(line)
        except Exception as error:
            self.responses.put(error)
        finally:
            self.responses.put(None)

    def remaining(self):
        if self.watchdog_failure:
            raise TimeoutError(self.watchdog_failure)
        remaining = self.deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("oracle command budget exhausted")
        return remaining

    def windows_path(self, path):
        if not self.args.wine:
            return str(path)
        return subprocess.check_output(
            [self.args.winepath, "-w", str(path)],
            env=self.env,
            text=True,
            timeout=min(10, self.remaining()),
        ).strip()

    def snapshot(self, request):
        return {
            "case": self.case,
            "request": request,
            "process": {"pid": self.process.pid, "exitCode": self.process.poll()},
            "lastAvailableResponse": self.last_response,
        }

    def request(self, request):
        self.remaining()
        self.sequence += 1
        sent = {"id": self.sequence, **request}
        self.pending_request = request
        self.process.stdin.write(json.dumps(sent, ensure_ascii=False) + "\n")
        self.process.stdin.flush()
        request_deadline = min(self.deadline, time.monotonic() + self.args.request_timeout)
        try:
            line = self.responses.get(timeout=max(0, request_deadline - time.monotonic()))
        except queue.Empty as error:
            raise TimeoutError(f"request timed out: {request}") from error
        self.remaining()
        if line is None or isinstance(line, Exception):
            raise RuntimeError(f"oracle protocol ended: {line}")
        response = json.loads(line)
        subset(
            response, {"id": self.sequence, "schemaVersion": 2, "referenceCommit": self.baseline}
        )
        if not isinstance(response.get("diagnostics"), list):
            raise AssertionError("missing diagnostic array")
        self.last_response = response
        self.pending_request = None
        self.records.append({"case": self.case, "request": request, "response": response})
        return response

    def close(self):
        self.closed.set()
        self.kill()
        remaining = self.deadline - time.monotonic()
        if remaining > 0:
            try:
                self.process.wait(timeout=min(0.2, remaining))
            except subprocess.TimeoutExpired:
                pass
        self.watchdog_thread.join(timeout=max(0, min(0.1, self.deadline - time.monotonic())))
        self.stderr.close()


def observe(oracle, presentation=False):
    response = oracle.request({"op": "observe", "observePresentation": presentation})
    subset(response, {"ok": True})
    return response["result"]


def assertions(oracle, case, last, load_request):
    findings = []
    for assertion in case.get("assertions", []):
        if assertion == "observe_stable":
            first = observe(oracle, case["group"] == "PRINTC")
            second = observe(oracle, case["group"] == "PRINTC")
            if second != first:
                raise AssertionError("observation changed state")
        elif assertion == "rng_roundtrip":
            watches = last["result"]["watches"]
            if oracle.args.oracle == "original":
                subset(watches["RESULT:2"], watches["RESULT:0"])
                subset(watches["RESULT:3"], watches["RESULT:1"])
            else:
                # Pinned snake DumpRanddata writes GetRand into ToArray's copy;
                # the zero RANDDATA is then restored by InitRanddata. Record this
                # oracle defect, never modify normal engine behavior to hide it.
                subset(last["result"], {"randomSeed": 123456, "randomAlgorithm": "sfmt19937"})
                subset(watches, {"RESULT:0": 192905, "RESULT:1": 520548,
                                 "RESULT:2": 0, "RESULT:3": 0})
                findings.append({"kind": "pinned_oracle_rng_state_loss", "roundtrip": False,
                                 "source": "Emuera/Runtime/Script/Statements/Variable/VariableEvaluator.cs:DumpRanddata",
                                 "reason": "GetRand writes a temporary RANDDATA.ToArray copy; INITRAND restores zeros"})
        elif assertion == "presentation":
            layout = last["result"]["presentation"]
            subset(layout, {"version": 1, "pending": False})
            nodes = [
                node
                for line in layout["lines"]
                for button in line["buttons"]
                for node in button["nodes"]
            ]
            if not nodes or any(node["width"] < 0 or node["error"] for node in nodes):
                raise AssertionError("PRINTC did not produce valid measured nodes")
            subset(layout, {"fontByteSource": "unverified-installed-source"})
            if oracle.args.oracle == "snake":
                fonts = [node["font"] for node in nodes if "font" in node]
                providers = {font["provider"] for font in fonts}
                if not providers or providers != set(layout["providerVersions"]):
                    raise AssertionError("provider summary differs from measured nodes")
                if (oracle.args.drawing_mode or "SKIASHARP") == "SKIASHARP" and providers != {"skia-measure-text"}:
                    raise AssertionError("SKIASHARP nodes used a different provider")
                # TEXTRENDERER uses GDI only for raster fonts; the pinned vector
                # font can legitimately retain the Skia cached measurement path.
                if not providers <= {"skia-measure-text", "system-windows-forms-text-renderer"}:
                    raise AssertionError("unknown actual measurement provider")
            if oracle.args.oracle == "snake" and not any(
                node["kind"] == "ConsoleSpacePart" and node["width"] > 0 for node in nodes
            ):
                raise AssertionError("snake PRINTC lost real pixel padding")
        elif assertion == "input_atomicity":
            subset(
                oracle.request({"op": "injectInput", "inputTrace": {"active": True}}), {"ok": True}
            )
            before = observe(oracle)
            bad = {
                "active": False,
                "beforeRun": [{"keyCode": 65, "down": True}, {"keyCode": 256, "down": False}],
            }
            subset(oracle.request({"op": "injectInput", "inputTrace": bad}), {"ok": False})
            if observe(oracle) != before:
                raise AssertionError("invalid trace mutated input state")
            invalid_requests = [
                {"op": "execute", "statement": 7},
                {"op": "execute", "statement": "PRINTL A", "watch": "RESULT"},
                {"op": "run", "entry": 7},
                {"op": "run", "entry": "SYSTEM_TITLE", "arguments": []},
                {"op": "run", "uiInputs": [{"text": "1", "changedByMouse": "yes"}]},
                {"op": "run", "inputs": {}},
                {"op": "run", "instructionLimit": "invalid"},
                {"op": "injectInput", "observePresentation": "yes"},
            ]
            for invalid in invalid_requests:
                invalid["inputTrace"] = {
                    "active": False,
                    "beforeRun": [{"keyCode": 65, "down": True}],
                    "awaitPumps": [[{"keyCode": 66, "down": True}]],
                }
                subset(oracle.request(invalid), {"ok": False})
                if observe(oracle) != before:
                    raise AssertionError(f"malformed request mutated input: {invalid}")
        elif assertion == "input_reset":
            subset(
                oracle.request(
                    {
                        "op": "injectInput",
                        "inputTrace": {
                            "active": True,
                            "beforeRun": [{"keyCode": 65, "down": True}],
                            "awaitPumps": [[{"keyCode": 66, "down": True}]],
                        },
                    }
                ),
                {"ok": True},
            )
            subset(oracle.request({"op": "reset"}), {"ok": True})
            subset(oracle.request(load_request), {"ok": True})
            result = oracle.request({"op": "injectInput", "inputTrace": {"active": True}})
            subset(
                result,
                {
                    "ok": True,
                    "result": {
                        "primitiveInput": {"pendingPumps": 0, "eventCount": 0, "pumpCount": 0}
                    },
                },
            )
            for key in result["result"]["primitiveInput"]["keys"]:
                subset(key, {"rawState": 0, "evaluatorToggle": 0})
                if oracle.args.oracle == "snake":
                    subset(key, {"latch": 0})
        else:
            raise ValueError(f"unknown assertion {assertion}")
    return findings


def close_oracle(oracle, evidence, failure):
    """Keep primary failure and raw requests even when OS cleanup is denied."""
    if oracle is None:
        return failure
    evidence.setdefault("requests", []).extend(oracle.records)
    try:
        oracle.close()
    except Exception as error:
        evidence["cleanupFailure"] = f"{type(error).__name__}: {error}"
        failure = failure or f"oracle cleanup failed: {error}"
        evidence.setdefault("failure", {"case": oracle.case, "error": failure})
    return failure


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--oracle", choices=["original", "snake"], required=True)
    parser.add_argument("--fixture", type=Path, default=FIXTURE)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--wrapper-sha", required=True)
    parser.add_argument("--rust-evidence", type=Path, required=True)
    parser.add_argument("--font-file", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--wine")
    parser.add_argument("--winepath", default="winepath")
    parser.add_argument("--wine-prefix", type=Path)
    parser.add_argument("--case", action="append", default=[])
    parser.add_argument("--drawing-mode", choices=["SKIASHARP", "TEXTRENDERER"])
    parser.add_argument("--logical-output-only", action="store_true",
                        help="capture values and logical output only; reject presentation cases")
    parser.add_argument("--budget-seconds", type=float, default=300)
    parser.add_argument("--request-timeout", type=float, default=20)
    args = parser.parse_args()
    if args.budget_seconds <= 0 or args.request_timeout <= 0:
        parser.error("budgets must be positive")
    if args.wine and args.wine_prefix is None:
        parser.error("Wine requires an isolated --wine-prefix")
    if len(args.wrapper_sha) != 40 or any(c not in "0123456789abcdef" for c in args.wrapper_sha):
        parser.error("--wrapper-sha must be the full wrapper commit")
    for name in ["fixture", "exe", "font_file", "output", "wine_prefix"]:
        value = getattr(args, name)
        if value is not None:
            setattr(args, name, value.resolve())
    if args.output.exists():
        parser.error("--output must be a new directory; existing evidence is never overwritten")
    manifest = json.loads((args.fixture / "cases.json").read_text())
    rust = json.loads(args.rust_evidence.read_text())
    source_fixture = identity(args.fixture)
    validate_rust_evidence(rust, args.oracle, source_fixture, manifest["seed"],
                           manifest.get("requiredRustPolicy", {}).get(args.oracle))
    rust_cases = {case["id"]: case for case in rust["cases"]}
    if hashlib.sha256(args.font_file.read_bytes()).hexdigest() != manifest["font"]["sha256"]:
        parser.error("the supplied font does not match the pinned fixture font")
    selected = [
        case
        for case in manifest["cases"]
        if not args.case or case["id"] in args.case or case["group"] in args.case
    ]
    if not selected:
        parser.error("no matching cases")
    args.output.mkdir(parents=True)
    game = args.output / "game"
    shutil.copytree(args.fixture, game)
    if args.drawing_mode == "SKIASHARP" and args.oracle != "snake":
        parser.error("SKIASHARP is a snake-only drawing mode")
    if args.oracle == "snake":
        config = game / "emuera.config"
        config.write_text(
            config.read_text().replace(
                "Drawing interface:TEXTRENDERER", "Drawing interface:" + (args.drawing_mode or "SKIASHARP")
            )
        )
    deadline = time.monotonic() + args.budget_seconds
    evidence = {
        "version": 1,
        "oracle": args.oracle,
        "wrapperSha": args.wrapper_sha,
        "semanticBaseline": manifest["semanticBaselines"][args.oracle],
        "seed": manifest["seed"],
        "sourceFixture": source_fixture,
        "effectiveFixture": identity(game),
        "caseFixtures": [],
        "font": {**manifest["font"], "byteSourceStatus": "unverified-installed-source"},
        "drawingMode": args.drawing_mode or ("SKIASHARP" if args.oracle == "snake" else "TEXTRENDERER"),
        "presentationObservation": "not_requested" if args.logical_output_only else "font_pinned_snapshot",
        "rust": {key: rust[key] for key in ["coreSha", "dirty", "profile"]},
        "rustComparison": {"status": "pending", "cases": []},
        "cases": [],
    }
    oracle = None
    failure = None
    active_case = "startup"
    try:
        for ordinal, case in enumerate(selected):
            active_case = case["id"]
            case_game = prepare_case_game(game, args.output, ordinal, evidence["effectiveFixture"])
            # Snake GetValidPath returns a relative path. The CLI's load operation
            # does not change the process CWD; use the actual isolated game root.
            oracle = Oracle(args, evidence["semanticBaseline"], deadline, case_game)
            oracle.case = active_case
            capabilities = oracle.request({"op": "capabilities"})
            subset(capabilities, {"ok": True, "result": {
                "observationVersions": {"presentationSnapshot": 1, "headlessInputTrace": 1}}})
            if args.oracle == "snake":
                subset(capabilities, {"result": {
                    "implementation": "emuera_lazyloading_selfmodified_version"}})
            evidence.setdefault("capabilities", capabilities)
            load = {
                "op": "load", "gameDir": oracle.windows_path(case_game),
                "seed": manifest["seed"], "instructionLimit": 100000, "timeoutMs": 3000,
                **load_observation_options(args.logical_output_only, selected, manifest["font"],
                                           oracle.windows_path(args.font_file)),
            }
            evidence["caseFixtures"].append({
                "case": active_case, "gameDir": load["gameDir"],
                "initialSha256": evidence["effectiveFixture"]["sha256"],
                "isolation": "fresh process and working directory per case; requests within a case share both",
                "capabilities": capabilities,
                "stderr": f"stderr-{case_game.name}.log",
            })
            load_response = oracle.request(load)
            validate_load(load_response, manifest.get("loadExpect"))
            last = None
            observed_steps = []
            findings = []
            for step in case["requests"]:
                last = oracle.request(step["request"])
                observed_steps.append({"request": step["request"], "response": last})
                findings.extend(step_expectations(step, last, args.oracle))
            findings.extend(assertions(oracle, case, last, load))
            evidence["rustComparison"]["cases"].append(
                compare_case(case, observed_steps, rust_cases.get(case["id"]), load_response, rust["profile"])
            )
            evidence["cases"].append(
                {
                    "id": case["id"],
                    "group": case["group"],
                    "status": "observed" if case.get("observation") or findings else "passed",
                    "findings": findings,
                    "targetBatch": case["targetBatch"],
                    "rustCurrentPolicy": case["rustCurrentPolicy"],
                    "snakeTargetStatus": case["snakeTargetStatus"],
                }
            )
            # Report completed observable state even when every individual case
            # finishes before its first periodic sample. The outer Wine monitor
            # must see real case progress across multiple fresh CLI processes.
            print(json.dumps({"oracleCaseCompleted": oracle.snapshot(None)},
                             ensure_ascii=False), flush=True)
            failure = close_oracle(oracle, evidence, oracle.watchdog_failure)
            oracle = None
            if failure:
                raise RuntimeError(failure)
    except Exception as error:
        failure = str(error)
        evidence["failure"] = {"case": active_case, "error": failure}
    finally:
        if oracle is not None and oracle.watchdog_failure and failure is None:
            failure = oracle.watchdog_failure
            evidence["failure"] = {"case": oracle.case, "error": failure}
        failure = close_oracle(oracle, evidence, failure)
        evidence["status"] = "failed" if failure else "completed_observations"
        evidence["rustComparison"]["status"] = "incomplete" if failure else "compared"
        (args.output / "evidence.json").write_text(
            json.dumps(evidence, ensure_ascii=False, indent=2) + "\n"
        )
    print(
        json.dumps(
            {
                "status": evidence["status"],
                "evidence": str(args.output / "evidence.json"),
                "failure": failure,
            },
            ensure_ascii=False,
        ),
        flush=True,
    )
    return 1 if failure else 0


if __name__ == "__main__":
    raise SystemExit(main())
