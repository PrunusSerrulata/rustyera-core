# EraBasic debugger protocol

`era-debug-protocol` 3.0 is independent from normal runtime control. It shares only the
common envelope and has its own version negotiation, payload tags and authorization. Active
debug traffic is bound to the runtime's current `SessionEpoch`.
No debugger functionality is enabled merely because a runtime session exists.

## Authorization

The session creator supplies an immutable debug-scope allowlist through the C ABI or a
future server policy. A debug frontend explicitly sends `DebugHello`; the grant is the
deterministic intersection of requested and allowed scopes. The runtime returns an opaque,
session-bound grant ID which is required by every subsequent debug request and may be
revoked at any time.

Scopes separately control variable read/write, game-field read/write, execution read,
execution control, expression evaluation, safe statement execution and breakpoint
management. A dynamic-library grant prevents accidental privilege use; a future server
transport must additionally authenticate the client.

## Coherent stopped state

A breakpoint or manual pause stops every fiber at an instruction boundary. The runtime
issues a `StopToken` containing pause epoch, program generation and runtime revision.
Every stopped-state read and mutation repeats that token. Continue, reload or any other
state transition invalidates it, making delayed requests fail with `stale_stop`.

Variable and game-field writes also include an expected revision. A batch is validated
fully before any item is changed. Type, dimension, index, mutability and frame/generation
checks remain mandatory even for a fully privileged debugger.

## Inspection and mutation

Variables use stable symbol bytes plus storage, generation, character, indices and, for
locals, fiber/frame identity. Lists and array/stack views are paginated. Runtime-owned game
fields are exposed through stable string keys and descriptors; only fields marked
`debug_writable` accept mutations. Raw Rust object addresses and field offsets never cross
the interface.

Fiber summaries, call stacks and operand stacks are read-only. `VmDebugInspect` contains
read methods, while `VmDebugControl` contains variable writes and execution control. There
is deliberately no operand-stack or call-frame mutation method.

## Breakpoints and stepping

The current protocol supports unconditional source and function-entry breakpoints. A source breakpoint
contains relative path, content hash and UTF-8 byte offset. Function breakpoints contain a
stable symbol key. Resolution reports verified, moved or unbound status for each program
generation, allowing a frontend to display hot-reload rebinding explicitly. Conditional
breakpoints and logpoints are future additive features.

Execution control supports instruction, source-line, into, over and out stepping on one
selected fiber while all other fibers remain paused. A step ends at its target, breakpoint,
Host wait, completion, fault or reload boundary. QTE time is frozen for the entire global
pause.

## Debug console

Expression evaluation is read-only and accepts the currently implemented EraBasic expression
subset, including operator precedence, ternary expressions and a pure-method whitelist. Safe
statement execution currently permits atomic scalar assignment. Flow control, input waits,
Host effects, increment/decrement, unsupported methods and partial instructions are rejected
without mutation. Allowed variable mutations remain committed. Synthetic control state, stacks
and normal presentation state are restored on success or failure; result text and diagnostics
return only on the debug channel.
