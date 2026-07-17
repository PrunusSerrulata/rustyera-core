# Emuera reference CLI

This Windows-only command wraps the C# implementation pinned in
`reference/emuera.em` and exposes it as a persistent NDJSON oracle. It creates
the normal Emuera parser, data, process, and VM objects through a headless
console adapter; it does not construct a `MainWindow`, native window handle, or
modal dialog. The wrapper is intended for differential tests, not as a
replacement game launcher.

This tool exposes semantic analysis and VM behavior from the **C# reference
implementation**. RustyEra has separate Rust analyzer, bytecode, compiler, validator, VM and
runtime crates. This CLI does not expose every reference system-flow, presentation, storage or
client-rendering operation, and successful oracle calls must not be described as validation of a
Rust component without a matching differential comparison.

## Build and run

Use the NAudio configuration to avoid the Windows Media Player COM dependency:

```powershell
dotnet build tools/emuera-reference-cli/Emuera.ReferenceCli.csproj `
  -c Debug-NAudio -p:Platform=x64 -r win-x64

dotnet run --project tools/emuera-reference-cli/Emuera.ReferenceCli.csproj `
  -c Debug-NAudio -p:Platform=x64 -r win-x64
```

The process reads one UTF-8 JSON object per line from stdin and writes exactly
one compact JSON object per line to stdout. Keep it alive across a test suite so
that a loaded game and its VM state can be reused.

Every response contains `id`, `ok`, `schemaVersion`, `referenceCommit`, and
`diagnostics`. A successful response also has `result`; a failed response has
`error.type` and `error.message`. The `id` is copied from the request and can be
any JSON value.

```json
{"id":1,"op":"lex","source":"RESULT = 1 + 2"}
{"id":2,"op":"parseExpression","source":"1 + 2 * 3"}
{"id":3,"op":"parseLine","source":"PRINTL hello","reduceArguments":false}
```

## Operations

| Operation | Important request fields | Result |
| --- | --- | --- |
| `capabilities` | none | Protocol, version, platform, and operation list |
| `reset` | none | Disposes the headless console and clears Emuera globals |
| `lex` | `source`; optional `endWith`, `flags` | Exact reference tokens and consumed UTF-16/UTF-8 lengths |
| `parseExpression` | `source` | Deterministic reflection graph and operand type |
| `parseLine` | `source`; optional `reduceArguments` | Logical-line summary and graph; reduction defaults off |
| `analyzeLine` | `source`; loaded game required | Parses and semantically reduces instruction arguments |
| `analyzeProject` | loaded game required | Returns a deterministic summary of loaded functions, reduced lines, argument types, and jump links |
| `load` | `gameDir`; optional `debug` | Loads `csv/` and `erb/`, then returns a VM snapshot |
| `eval` | `source` | Parsed expression plus its current runtime value |
| `execute` | `statement`; optional limits and `watch` | Executes one non-control-flow instruction in the loaded VM |
| `run` | optional `entry`, `arguments`, `inputs`, limits, `watch` | Runs an isolated function or resumes pending input |

`lex.endWith` and each item in `lex.flags` are case-insensitive C# enum names
from the pinned reference implementation. They default to `EoL` and `None`.
Use `capabilities` to detect protocol changes instead of relying on the binary
version alone.

Game/VM example:

```json
{"id":"load","op":"load","gameDir":"C:\\games\\my-era"}
{"id":"set","op":"execute","statement":"FLAG:10 = 123","watch":["FLAG:10"]}
{"id":"run","op":"run","entry":"MY_TEST","arguments":"42, \"text\"","inputs":["0"],"watch":["RESULT","RESULTS"],"instructionLimit":1000000,"timeoutMs":10000}
```

`run.entry` uses Emuera's real CALL argument parser but treats the selected
function as the root of an isolated VM run. A snapshot reports `termination` as
`completed`, `waitingInput`, `instructionLimit`, `timeout`, `quit`, or `error`.
If a function waits for input, provide inputs in the same request or send a
later `run` request containing only `inputs`.

Use `run` for CALL/JUMP/BEGIN and other control flow. `execute` is deliberately
limited to standalone instructions such as assignment and printing, because a
synthetic control-flow line has no valid source-line return address.

The `output` field is the complete current display buffer, not a delta. Numeric
runtime values and token literals remain JSON numbers. Deep parser/AST objects
are represented by a deterministic graph with `$id`, `$ref`, `$type`, and
`$truncated` markers; this avoids adding a second hand-maintained C# AST merely
for comparison.

## Smoke test

After building on Windows, run:

```powershell
tools/emuera-reference-cli/tests/protocol-smoke.ps1
```

On macOS with the repository Wine prefix, run:

```sh
tools/emuera-reference-cli/test-macos-wine.sh
```

The test checks that malformed requests do not kill the persistent process,
then exercises lexer, expression parser, logical-line parser, game loading,
CSV-backed values, isolated function execution, input, watches, and reset. The
macOS test includes game loading and CSV evaluation specifically to catch any
accidental dependency on WinForms initialization under Wine.

Passing either script establishes that the oracle process is usable. A Rust/C#
differential test must additionally feed the same fixture or source to both
implementations and compare the fields relevant to the component under test.

## Oracle maintenance

Failure of this tool is a test-infrastructure defect, not a reason to skip
differential testing. If the process fails to start, exits early, stops
producing one response per request, or hangs:

1. reproduce the failure with the smallest request sequence;
2. identify whether startup, protocol projection, or a UI dependency is at
   fault;
3. repair this wrapper first when possible;
4. if necessary, add the smallest possible hook to `reference/emuera.em`, gated
   by `Program.HeadlessMode` or otherwise reachable only by
   `Emuera.ReferenceCli`;
5. rerun the failing sequence, the complete platform smoke test, and the
   matching Rust differential test.

Reference-tree changes may expose state, suppress invisible UI work, or add
deterministic test limits. They must not alter parsing, project loading,
validation, execution, state transitions, or other backend behavior used by
the normal game. Never change the oracle merely to make it agree with Rust.

Every reference-tree modification must be listed separately in
[REFERENCE_CHANGES.md](REFERENCE_CHANGES.md) with its purpose, headless gate,
effect on the normal game path, and verification. The task handoff must repeat
the files changed during that task rather than referring only to the log.

## Fidelity notes

- The wrapper calls the original reference types directly; only small gated
  headless hooks were added for constructing the console without a window,
  state access, warnings, limits, and independent function execution. Normal
  game startup still constructs `MainWindow` and follows the unchanged UI path.
- The complete inventory and rationale for reference-tree hooks is maintained
  in [REFERENCE_CHANGES.md](REFERENCE_CHANGES.md).
- The target is Windows x64 with .NET 10 because the reference assembly depends
  on WinForms and Windows Desktop runtime types.
- Protocol text is UTF-8. Game file encoding behavior remains the reference
  implementation's behavior; the Rust rewrite itself only accepts UTF-8.
- Modal dialogs are suppressed only while `Program.HeadlessMode` is active.
  Prompt questions conservatively resolve to “No”.
