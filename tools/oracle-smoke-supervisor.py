#!/usr/bin/env python3
"""Supervise the existing NDJSON smoke request file without changing its assertions."""

import argparse
import json
import math
import os
from pathlib import Path
import queue
import signal
import subprocess
import sys
import threading
import time


def unchanged(previous, current):
    return previous is not None and comparison_state(previous) == comparison_state(current)


def comparison_state(snapshot):
    """Normalize transport/report metadata only; script dictionaries remain intact."""
    state = dict(snapshot)
    state.pop("reportMetadata", None)
    state.pop("responsesCompleted", None)
    # This helper's case label is exactly the NDJSON transport id, unlike fixture-group names.
    state.pop("case", None)
    for key in ("pending", "lastFullResponse"):
        envelope = state.get(key)
        if isinstance(envelope, dict):
            state[key] = {name: value for name, value in envelope.items() if name != "id"}
    return state


def stop(process):
    if os.name == "nt":
        subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=3, check=False)
    else:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def supervise(args):
    deadline = time.monotonic() + args.timeout
    lock, closed = threading.Lock(), threading.Event()
    state = {"phase": "starting", "pending": None, "lastFullResponse": None,
             "case": None, "responsesCompleted": 0}
    responses = queue.Queue()
    with args.stderr.open("w", encoding="utf-8") as errors, args.output.open("w", encoding="utf-8") as output:
        process = subprocess.Popen(args.command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=errors, text=True, encoding="utf-8", errors="strict",
                                   start_new_session=os.name != "nt")

        def read():
            try:
                for line in process.stdout:
                    responses.put(line)
            except Exception as error:
                responses.put(error)
            finally:
                responses.put(None)

        def watchdog():
            previous, next_sample = None, time.monotonic() + 5
            while not closed.wait(0.05):
                now = time.monotonic()
                if now < next_sample and now < deadline:
                    continue
                with lock:
                    snapshot = {**state, "process": {"pid": process.pid, "returncode": process.poll()}}
                print(json.dumps({"oracleSmokeWatchdog": snapshot}, ensure_ascii=False), file=sys.stderr, flush=True)
                failure = "wall-clock budget exhausted" if now >= deadline else (
                    "unchanged full snapshots at consecutive 5s samples" if unchanged(previous, snapshot) else None)
                if failure:
                    print(f"oracle smoke failed: {failure}", file=sys.stderr, flush=True)
                    try:
                        stop(process)
                    finally:
                        # Also terminates a caller blocked in pipe/file operations.
                        os._exit(2)
                previous = snapshot
                next_sample += 5

        threading.Thread(target=read, daemon=True).start()
        observer = threading.Thread(target=watchdog, daemon=True)
        observer.start()
        try:
            with args.requests.open(encoding="utf-8") as requests:
                for wire in requests:
                    request = json.loads(wire)
                    with lock:
                        state.update(phase="request", case=request.get("id"), pending=request)
                    process.stdin.write(wire.rstrip("\r\n") + "\n")
                    process.stdin.flush()
                    line = responses.get(timeout=max(0.001, deadline - time.monotonic()))
                    if line is None:
                        raise RuntimeError("oracle exited before completing the pending request")
                    if isinstance(line, Exception):
                        raise line
                    response = json.loads(line)
                    with lock:
                        state.update(phase="response", pending=None, lastFullResponse=response,
                                     responsesCompleted=state["responsesCompleted"] + 1)
                    # Keep the original envelope/schema/values for the shell's existing jq assertions.
                    output.write(line.rstrip("\r\n") + "\n")
                    output.flush()
            with lock:
                state.update(phase="closing", pending="process_exit")
            process.stdin.close()
            code = process.wait(timeout=max(0.001, deadline - time.monotonic()))
            if code:
                raise RuntimeError(f"oracle exited with status {code}")
        finally:
            stop(process)
            closed.set()
            observer.join(timeout=1)
            process.wait(timeout=3)
            process.stdout.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("requests", "output", "stderr"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--timeout", type=float, required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.command[:1] == ["--"]:
        args.command = args.command[1:]
    if not args.command or not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("a command and finite positive timeout are required")
    supervise(args)


if __name__ == "__main__":
    main()
