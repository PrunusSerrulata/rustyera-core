# Design principles

RustyEra uses the following priority order when requirements conflict:

1. cross-client and cross-platform support;
2. architectural purity;
3. strict behavioral compatibility with the pinned Emuera reference.

The order is a conflict-resolution rule, not permission to ignore the reference.
EraBasic game rules, input decisions, state transitions and other script-observable
effects should match the pinned implementation whenever that does not violate a
higher-ranked principle. Every intentional difference must be documented and covered
by a stable test where executable behavior is involved.

## Cross-client semantics

The runtime owns a canonical semantic presentation model. It describes logical text,
styles, buttons, aligned cells, separators, scenes, resources, interactions and
one-shot effects without embedding WinForms, GDI, operating-system device or pixel-cache
objects. Each frontend projects that one authoritative model to its own GUI, TUI,
accessibility tree or remote protocol.

When reference behavior depends on a particular renderer, font metric, input API or
platform facility, implementation work first identifies the script author's intent and
the observable game semantics. The runtime preserves those semantics; the frontend owns
device-specific rendering and input collection. Pixel-for-pixel reproduction is not a
goal unless it is itself required for script-visible game behavior and can be expressed
portably.

## Architectural boundary

The runtime is the sole authority for game state, rules, timeouts, interaction tokens,
session order, saves and recovery. It drives the VM through Rust interfaces. The VM does
not mutate runtime state from an instruction callback. Frontends perform filesystem I/O,
rendering, media playback, platform input collection and device/lifecycle reporting.
They exchange versioned messages with the runtime and never share internal objects.

Recoverable presentation state is separate from one-shot effects. Runtime and lower
layers do not operate GUI, audio or filesystem APIs. Portable, deterministic logical
representations are preferred over leaking client implementation details into shared
contracts.

## Reference compatibility

The fixed Emuera source tree remains the oracle for language syntax and semantics and
for portable runtime behavior. Reference investigation is also used to recover the
intent of UI-coupled commands. A behavior is classified as one of:

- compatible and implemented;
- intentionally different because a higher-ranked principle applies;
- explicitly unsupported with a stable error;
- missing and still to be implemented.

The current classification is maintained in
[Runtime compatibility status](runtime-compatibility-status.zh-CN.md). Protocol type
definitions and reference-CLI endpoints are not evidence that the corresponding Rust
capability is implemented.

## Determinism and persisted formats

Sources are UTF-8 and source locations are UTF-8 byte offsets. Compilation products are
deterministic for identical semantic inputs. Bytecode, traditional-save adapters and VM
snapshots carry explicit identities or versions and reject incompatible data instead of
guessing. Collections, diagnostics and wire encodings must not depend on randomized map
iteration or client-local state.
