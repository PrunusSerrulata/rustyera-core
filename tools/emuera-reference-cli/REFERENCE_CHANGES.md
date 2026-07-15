# Reference CLI changes to the pinned Emuera tree

`reference/emuera.em` is the compatibility oracle and is read-only by default.
This file is the mandatory audit log for the small exceptions needed by
`tools/emuera-reference-cli`. It records reference-tree changes separately
from Rust and wrapper changes.

The pinned compatibility baseline remains
`26a35dc9334bb67590b96f7b8efbefbf199e391e`. These hooks do not constitute a
reference-version update.

## Invariants

Every reference-tree hook must satisfy all of the following:

- it is needed to expose or stabilize the reference oracle;
- it is gated by `Program.HeadlessMode`, callable only by the friend assembly,
  or observational only;
- the ordinary Emuera entry point continues to use the original backend logic;
- it does not make the C# result agree with Rust by changing language or runtime
  semantics;
- its platform smoke test and matching Rust differential test are recorded.

If the CLI fails or hangs, fix the wrapper first. Modify the reference tree only
when the wrapper cannot avoid the UI dependency or cannot observe the required
backend state.

## Current reference-tree hook inventory

The following is the complete intentional oracle-related inventory in the
current tree.

| Reference file | Oracle purpose | Isolation and normal-game effect |
| --- | --- | --- |
| `Emuera/Emuera.csproj` | Grants `Emuera.ReferenceCli` access to internal reference types with `InternalsVisibleTo`. | Assembly visibility only; no runtime behavior changes. |
| `Emuera/Program.Headless.cs` | Defines `HeadlessMode` and configures the game directory/debug flag without entering `Program.Main`. | Called only by the friend CLI. The normal entry point and directory setup remain unchanged. |
| `Emuera/UI/Dialog.cs` | Prevents invisible modal dialogs from blocking NDJSON requests; prompts conservatively resolve to “No”. | Branches only when `HeadlessMode` is true. Normal mode still calls the same WinForms message boxes. |
| `Emuera/Runtime/Script/Data/ParserMediator.Headless.cs` | Atomically projects and clears parser warnings per response. | Observational helper called only by the CLI; parser warning production is unchanged. |
| `Emuera/Runtime/Script/Process.Headless.cs` | Adds bounded execution, standalone instruction dispatch, isolated function entry, counters, and termination state. | Entry points are internal and used only by the CLI. Limit checks return immediately outside headless mode. |
| `Emuera/Runtime/Script/Process.ScriptProc.cs` | Invokes the headless instruction/timeout check at the real VM dispatch boundary. | `HeadlessCheckLimit` is a no-op when `HeadlessMode` is false; instruction dispatch is otherwise unchanged. |
| `Emuera/Runtime/Script/Process.cs` | Stops an isolated headless root function before the title/system state machine resumes. | `HeadlessFinishFunctionRun` returns false outside headless mode, preserving the original system-process loop. |
| `Emuera/UI/Game/EmueraConsole.Headless.cs` | Constructs a console without a `MainWindow`, exposes state/process/input, resumes execution, and adapts textbox changes. | Construction is rejected unless `HeadlessMode` is true. In normal mode the textbox adapter calls the original `MainWindow.ApplyTextBoxChanges`. |
| `Emuera/UI/Game/EmueraConsole.cs` | Allows the real backend console/process to run without a native window and skips debug-window, repaint, scrollbar, timer, and title-widget operations in headless mode. | Every changed behavior is conditional on `HeadlessMode`; the public `EmueraConsole(MainWindow)` game path is retained. Script execution, input state, and output buffering still use the original backend. |
| `Emuera/UI/Game/EmueraConsole.Print.cs` | Keeps the display buffer used by snapshots while suppressing native repaint/textbox work. | Only UI side effects are skipped in headless mode; normal rendering calls are unchanged. |
| `Emuera/Runtime/Script/Statements/Instraction.Child.cs` | Routes INPUT-family textbox layout updates through the console adapter so waiting for input does not dereference a missing window. | All six INPUT-family instructions still build the same request and call the same backend wait logic. In normal mode the adapter performs the original textbox call. |
| `Emuera/UI/Framework/Forms/MainWindow.Headless.cs` | Retains the earlier internal console accessor used by the first hidden-window host. | Observational property only. The current CLI no longer constructs `MainWindow`; the normal game is unaffected. |

## Change records

### Headless oracle integration

The initial CLI integration added the friend-assembly declaration, headless mode
configuration, diagnostics projection, bounded/process execution helpers,
console/process state access, modal-dialog suppression, and the original
`MainWindow.Headless.cs` accessor.

Verification is provided by the Windows protocol smoke test and the Rust
lexer/parser tests that consume equivalent inputs.

### 2026-07-14: Wine no-window project loading

Symptom: lexer/parser-only requests worked under Wine, but loading a complete
fixture blocked while the CLI initialized WinForms and forced creation of a
hidden native window handle.

Reference-tree changes:

- `Emuera/UI/Game/EmueraConsole.Headless.cs`: added a headless-only no-window
  constructor and textbox adapter.
- `Emuera/UI/Game/EmueraConsole.cs`: added headless UI guards while retaining
  the normal constructor and backend initialization.
- `Emuera/UI/Game/EmueraConsole.Print.cs`: suppressed native paint/textbox
  side effects only in headless mode.
- `Emuera/Runtime/Script/Statements/Instraction.Child.cs`: routed six
  INPUT-family UI calls through the gated adapter.

Wrapper/test changes outside the reference tree removed
`ApplicationConfiguration.Initialize`, `MainWindow`, and native handle
creation from `ReferenceHost`; extended the macOS Wine script through project
load, CSV values, semantic line reduction, execution, function calls, input,
and reset; and added a watchdog.

Verification:

- `dotnet build tools/emuera-reference-cli/Emuera.ReferenceCli.csproj -c Debug-NAudio -p:Platform=x64 -r win-x64 --no-restore`
- `tools/emuera-reference-cli/test-macos-wine.sh`
- `cargo test -p erabasic-csv reference_cli_fixture_has_the_same_rust_projection -- --exact`
- `cargo test --workspace`

The Wine test completed all requests with empty stderr. The matching Rust/C#
CSV projections agreed on ABL size/name lookup, item price, initial STR data,
character ABL data, and GAMEBASE code.

### 2026-07-15: Timed one-input wait in the no-window oracle

Symptom: loading a minimal project whose `SYSTEM_TITLE` executed a positive-time
`TONEINPUTS` constructed the correct `InputRequest`, then threw
`NullReferenceException` while updating `MainWindow`'s last-input marker.

Reference-tree change:

- `Emuera/UI/Game/EmueraConsole.cs`: skips `window.update_lastinput()` only when
  `Program.HeadlessMode` is active. The input request, timer setup and backend
  state transition are unchanged. Normal games still execute the original UI
  call because `HeadlessMode` is false on the ordinary startup path.

The wrapper cannot supply this UI object without reintroducing the hidden-window
dependency that headless mode exists to avoid. Verification uses the minimal
timed `TONEINPUTS` load request, the complete macOS Wine smoke test, and the
same-input Rust analyzer fixture `reference-input-signatures.json`.

## Template for future entries

Append a dated section containing:

- the original failure or hang and the minimal reproducer;
- every modified path under `reference/emuera.em`;
- why a wrapper-only fix was insufficient;
- the headless/friend-only isolation mechanism;
- why normal game backend semantics are unchanged;
- the platform smoke command and same-input Rust comparison;
- any remaining platform or behavior limitation.
