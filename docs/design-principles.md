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

Canonical presentation records what the game intends to express, not the realized layout
of a particular frontend. It may contain logical content, styles, interaction tokens,
layout constraints and script-defined coordinate spaces. It must not contain positions,
wrapping, raster caches or physical history derived from a frontend's font engine,
viewport, DPI or paint implementation. Script-defined media coordinates are logical
presentation data and do not imply a device-pixel coordinate system.

When reference behavior depends on a particular renderer, font metric, input API or
platform facility, implementation work first identifies the script author's intent and
the observable game semantics. The runtime preserves portable semantics; the frontend
owns device-specific rendering and input collection. Pixel-for-pixel reproduction is not
a goal.

An EraBasic operation may explicitly observe the frontend environment or the realized
presentation. Such a result is an external observation, not canonical presentation state.
The runtime obtains it from the session's authoritative frontend through an ordered,
versioned service request. The runtime validates identity, revision, operation version
and result shape, but does not reproduce or interpret the frontend's rendering algorithm.
A missing capability produces an explicit unsupported result; the runtime never invents
a canonical approximation for a renderer-observation query.

Frontend observations are distinguished from portable resource facts. For example,
decoded source-image dimensions and pixels may be content-addressed external services,
while a rasterized canvas pixel or measured text extent is frontend-projection dependent.
Queries of runtime-owned style or alignment intent remain runtime operations.

## Portability and projection-dependent scripts

Frontend observations are ordered external inputs. Execution is deterministic only
relative to the complete input and service-response trace. A pending observation is a
transient external wait and blocks stable snapshots and hot-reload commits. The current
contract has exactly one authoritative frontend per runtime session: the session envelope,
epoch, sequence and observation revisions identify it. Multi-client sessions and authority
transfer are outside the supported model and require a future major protocol redesign.

Frontend-environment queries may be legitimate for presentation adaptation. A font
availability check used to select an ASCII-art fallback or a viewport query used only to
choose layout is not itself an architectural violation. However, projection-dependent
results are non-portable and must carry a source-located portability diagnostic. If such
a value influences persistent game state, gameplay-affecting control flow, random seeds,
dynamic dispatch, save contents or other game content instead of presentation adaptation,
a stronger diagnostic reports that dependency as compatibility-only and a candidate for
future deprecation.

Deprecation is not automatic. It requires a documented portable replacement, review of
real-script usage and an explicit language or protocol version transition. Until then,
supported compatibility-only operations return the authoritative frontend's real result;
they are neither silently approximated nor silently removed.

## Architectural boundary

The runtime is the sole authority for game state, rules, timeouts, interaction tokens,
session order, saves and recovery. It drives the VM through Rust interfaces. The VM does
not mutate runtime state from an instruction callback. Frontends perform filesystem I/O,
rendering, media playback, platform input collection and device/lifecycle reporting.
They exchange versioned messages with the runtime and never share internal objects.

Sole authority does not mean that the runtime manufactures external facts. A frontend
service response becomes script-visible only after the runtime accepts it in session
order; the frontend supplies the observation, while the runtime remains authoritative
for all resulting validation and game-state transitions.

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

Compatibility status and portability status are orthogonal. An implemented operation is
also classified as portable, frontend-environment dependent, frontend-projection
dependent, compatibility-only/discouraged, deprecated or unsupported. A command can be
reference-compatible while still being non-portable and diagnostically discouraged.

The current classification is maintained in
[Runtime compatibility status](runtime-compatibility-status.zh-CN.md). Protocol type
definitions and reference-CLI endpoints are not evidence that the corresponding Rust
capability is implemented.

## Determinism and persisted formats

Sources are UTF-8 and source locations are UTF-8 byte offsets. Compilation products are
deterministic for identical semantic inputs. Bytecode, traditional-save adapters and VM
snapshots carry explicit identities or versions and reject incompatible data instead of
guessing. Collections, diagnostics and wire encodings must not depend on randomized map
iteration or unrecorded client-local state. Runtime execution that consumes clock, input,
entropy or frontend-observation services is reproducible only with the same ordered
external-response trace.
