"""Drive the real Textual frontend worker to the audited eraTW day-one milestone."""

from __future__ import annotations

import os
import queue
import sys
import time
import traceback
from pathlib import Path

from audit_paths import REPOSITORY_ROOT, project_path, runtime_library

ROOT = REPOSITORY_ROOT
sys.path.insert(0, str(ROOT / "frontends" / "era-tui" / "src"))

from rustyera_tui.presentation import PresentationModel  # noqa: E402
from rustyera_tui.runtime import RuntimeClient, RuntimeWorker  # noqa: E402

_original_pump = RuntimeClient.pump
_original_handle_runtime = RuntimeClient._handle_runtime


def _traced_pump(client: RuntimeClient) -> bool:
    try:
        return _original_pump(client)
    except Exception:  # noqa: BLE001 - local audit instrumentation
        traceback.print_exc()
        raise


RuntimeClient.pump = _traced_pump


def _traced_handle_runtime(
    client: RuntimeClient, tag: int, value: object, correlation_id: int | None
) -> None:
    if tag == 92:
        print(f"RAW_FAULT={value!r}")
    _original_handle_runtime(client, tag, value, correlation_id)


RuntimeClient._handle_runtime = _traced_handle_runtime


def plain_tail(model: PresentationModel, count: int = 20) -> str:
    return "\n".join(
        "".join(segment.text for segment in line.segments) for line in model.lines[-count:]
    )


def display_rows(model: PresentationModel, count: int = 300) -> list[str]:
    rows: list[str] = []
    for line in model.lines[-count:]:
        rows.extend("".join(segment.text for segment in line.segments).split("\n"))
    return rows


def main() -> int:
    project = project_path()
    library = runtime_library()
    default_answers = "0,1,1,1,1,1,1,1,1,1,1,1,1,0,9999,0,2,1999,0,100,1"
    answers = [
        int(value)
        for value in os.environ.get("ERA_AUDIT_ANSWERS", default_answers).split(",")
    ]
    fallback_answer = os.environ.get("ERA_AUDIT_FALLBACK_ANSWER")
    snapshot_path = os.environ.get("ERA_AUDIT_SNAPSHOT_PATH")
    snapshot_every_wait = os.environ.get("ERA_AUDIT_SNAPSHOT_EVERY_WAIT") == "1"
    layout_check = os.environ.get("ERA_AUDIT_LAYOUT_CHECK") == "1"
    layout_stage: str | None = None
    snapshot_requested = False
    snapshot_attempts = 0
    snapshot_attempt_wait: tuple[dict[int, object], str] | None = None
    interactive_stdin = os.environ.get("ERA_AUDIT_STDIN") == "1" or sys.stdin.isatty()
    answer_index = 0
    handled_waits: set[tuple[int, int, int]] = set()
    model = PresentationModel()
    worker = RuntimeWorker(library, project)
    started = time.monotonic()
    presentation_events = 0
    last_progress = started
    worker.start()

    def advance_wait(
        wait: dict[int, object], tail: str, *, snapshot_attempted: bool = False
    ) -> int | None:
        nonlocal answer_index, snapshot_requested, snapshot_attempt_wait, layout_stage
        if wait[1] == 0:
            worker.send("submit_text", "")
        elif layout_check and layout_stage == "day_one":
            look_rows = [row for row in display_rows(model) if "[Look]" in row]
            if not look_rows:
                print("DAY1_LAYOUT_ERROR no Look row after entering day one")
                return 1
            print("DAY1_LOOK_ROW_OK")
            layout_stage = "map"
            worker.send("submit_text", "400")
        elif layout_check and layout_stage in ("map", "map_toggled"):
            c_rows = [
                row for row in display_rows(model) if "[C] - 移動先表示切替" in row
            ]
            if not c_rows or not c_rows[-1].rstrip().endswith("[C] - 移動先表示切替"):
                print(f"MAP_LAYOUT_ERROR rows={c_rows[-3:]!r}")
                return 1
            if layout_stage == "map":
                print("MAP_C_ROW_OK before_toggle=1")
                layout_stage = "map_toggled"
                worker.send("submit_text", "C")
            else:
                print("MAP_C_ROW_OK after_toggle=1")
                print("TUI_LAYOUT_OK")
                return 0
        elif answer_index < len(answers):
            answer = answers[answer_index]
            answer_index += 1
            print(f"answer[{answer_index}]={answer}")
            worker.send("submit_text", str(answer))
        elif wait[5] and all(marker in tail for marker in ("SAVE", "LOAD", "UPDATE")):
            print(
                f"DAY1_MILESTONE wait={wait[0]} answers={answer_index} "
                f"elapsed={time.monotonic() - started:.2f}s lines={len(model.lines)}"
            )
            if layout_check:
                layout_stage = "day_one"
                worker.send("submit_text", "100")
            elif snapshot_path and not snapshot_attempted:
                worker.send("export_snapshot", snapshot_path)
                snapshot_requested = True
                snapshot_attempt_wait = (wait, tail)
            elif snapshot_every_wait:
                print(f"NO_ELIGIBLE_SNAPSHOT_POINT attempts={snapshot_attempts}")
                return 4
            else:
                return 0
        elif fallback_answer is not None and "[完成決定]" in tail:
            print("fallback answer=999")
            worker.send("submit_text", "999")
        elif fallback_answer is not None:
            print(f"fallback answer={fallback_answer}")
            worker.send("submit_text", fallback_answer)
        elif interactive_stdin:
            try:
                answer = input("answer> ").strip()
            except EOFError:
                print("stdin closed before the day-one menu")
                return 2
            if answer == ":quit":
                print("audit stopped by operator")
                return 2
            print(f"stdin answer={answer!r}")
            worker.send("submit_text", answer)
        else:
            print("unplanned stable wait before the day-one menu")
            return 2
        return None

    try:
        while time.monotonic() - started < 900:
            try:
                event = worker.events.get(timeout=0.25)
            except queue.Empty:
                if not worker.is_alive():
                    print("worker stopped without reaching the milestone")
                    return 1
                continue
            if event.kind == "presentation_snapshot":
                model.apply_snapshot(event.value)
                presentation_events += 1
            elif event.kind == "presentation_delta":
                model.apply_delta(event.value)
                presentation_events += 1
            if event.kind in ("presentation_snapshot", "presentation_delta"):
                if time.monotonic() - last_progress >= 30:
                    last_progress = time.monotonic()
                    print(
                        f"progress elapsed={last_progress - started:.2f}s "
                        f"revision={model.revision} lines={len(model.lines)} "
                        f"presentation_events={presentation_events}"
                    )
                continue
            if (
                event.kind == "error"
                and snapshot_every_wait
                and snapshot_requested
                and "当前状态不能生成快照" in str(event.value)
            ):
                print(
                    f"SNAPSHOT_INELIGIBLE attempt={snapshot_attempts} "
                    f"wait={snapshot_attempt_wait[0][0] if snapshot_attempt_wait else 'unknown'} "
                    f"error={event.value}"
                )
                snapshot_requested = False
                if snapshot_attempt_wait is not None:
                    wait, tail = snapshot_attempt_wait
                    snapshot_attempt_wait = None
                    result = advance_wait(wait, tail, snapshot_attempted=True)
                    if result is not None:
                        return result
                continue
            if event.kind in ("error", "runtime_fault"):
                print(f"ERROR: {event.value}")
                print(plain_tail(model))
                return 1
            if event.kind == "status" and snapshot_requested and snapshot_path:
                if "VM 快照已导出" in str(event.value):
                    size = Path(snapshot_path).stat().st_size
                    print(f"VM_SNAPSHOT_BYTES={size}")
                    return 0
            if event.kind == "log" and (
                "Fault" in str(event.value) or "unsupported" in str(event.value).lower()
            ):
                print(f"LOG: {event.value}")
            if event.kind != "wait" or event.value is None:
                continue
            wait = event.value
            token = wait[11]
            key = (wait[0], token[0], token[1])
            if key in handled_waits:
                continue
            handled_waits.add(key)
            tail = plain_tail(model)
            print(
                f"wait={wait[0]} kind={wait[1]} system={wait[5]} "
                f"answer_index={answer_index} elapsed={time.monotonic() - started:.2f}s"
            )
            print(tail[-2000:])
            if snapshot_every_wait and snapshot_path and wait[2] == 0 and wait.get(8) is None:
                snapshot_attempts += 1
                snapshot_requested = True
                snapshot_attempt_wait = (wait, tail)
                worker.send("export_snapshot", snapshot_path)
                continue
            result = advance_wait(wait, tail)
            if result is not None:
                return result
        print("timed out before the day-one menu")
        print(plain_tail(model))
        return 3
    finally:
        worker.stop()
        worker.join(timeout=5)


if __name__ == "__main__":
    raise SystemExit(main())
