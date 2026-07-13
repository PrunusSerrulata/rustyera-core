# Emuera reference CLI

This Windows-only command wraps the C# implementation pinned in
`reference/emuera.em` and exposes it as a persistent NDJSON oracle. It creates
the normal Emuera runtime objects but never shows the main window or modal
dialogs. The wrapper is intended for differential tests, not as a replacement
game launcher.

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
| `reset` | none | Disposes the hidden host and clears Emuera globals |
| `lex` | `source`; optional `endWith`, `flags` | Exact reference tokens and consumed UTF-16/UTF-8 lengths |
| `parseExpression` | `source` | Deterministic reflection graph and operand type |
| `parseLine` | `source`; optional `reduceArguments` | Logical-line summary and graph; reduction defaults off |
| `analyzeLine` | `source`; loaded game required | Parses and semantically reduces instruction arguments |
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

The test checks that malformed requests do not kill the persistent process,
then exercises lexer, expression parser, logical-line parser, game loading,
isolated function execution, input, watches, and reset.

## Fidelity notes

- The wrapper calls the original reference types directly; only small gated
  headless hooks were added for state access, warnings, limits, and independent
  function execution.
- The target is Windows x64 with .NET 10 because the reference assembly depends
  on WinForms and Windows Desktop runtime types.
- Protocol text is UTF-8. Game file encoding behavior remains the reference
  implementation's behavior; the Rust rewrite itself only accepts UTF-8.
- Modal dialogs are suppressed only while `Program.HeadlessMode` is active.
  Prompt questions conservatively resolve to “No”.
