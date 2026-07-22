"""Restore an exported eraTW VM snapshot through the real TUI worker path."""

from __future__ import annotations

import queue
import sys
import time
from pathlib import Path

from audit_paths import REPOSITORY_ROOT, project_path, runtime_library

ROOT = REPOSITORY_ROOT
sys.path.insert(0, str(ROOT / "frontends" / "era-tui" / "src"))

from rustyera_tui.runtime import RuntimeWorker  # noqa: E402


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: tui_snapshot_restore.py SNAPSHOT")
    snapshot = Path(sys.argv[1]).resolve(strict=True)
    project = project_path()
    library = runtime_library()
    worker = RuntimeWorker(library, project)
    restore_sent = False
    import_completed = False
    restored_phase = None
    started = time.monotonic()
    worker.start()
    try:
        while time.monotonic() - started < 300:
            try:
                event = worker.events.get(timeout=0.25)
            except queue.Empty:
                if not worker.is_alive():
                    print("worker stopped before snapshot restore completed")
                    return 1
                continue
            if event.kind in ("error", "runtime_fault"):
                print(f"ERROR: {event.value}")
                return 1
            if event.kind == "status":
                print(f"STATUS: {event.value}")
                if "快照传输完成" in str(event.value):
                    import_completed = True
            elif event.kind == "project_loaded" and not restore_sent:
                restore_sent = True
                print(f"RESTORE_REQUEST bytes={snapshot.stat().st_size}")
                worker.send("restore_snapshot", snapshot)
            elif event.kind == "phase" and import_completed:
                restored_phase = event.value
            elif event.kind == "wait" and import_completed and event.value is not None:
                wait = event.value
                print(
                    f"RESTORE_OK phase={restored_phase} wait={wait[0]} "
                    f"kind={wait[1]} stability={wait[2]} system={wait[5]}"
                )
                return 0
        print("timed out before restored wait")
        return 2
    finally:
        worker.stop()
        worker.join(timeout=5)


if __name__ == "__main__":
    raise SystemExit(main())
