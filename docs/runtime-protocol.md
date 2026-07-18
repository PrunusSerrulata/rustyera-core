# Runtime and frontend protocol

面向应用前端开发的中文 ABI、参数、返回值和消息流程说明见
[Runtime 前端公共 API 指南](runtime-frontend-api.zh-CN.md)。

This document specifies the interfaces used by the RustyEra runtime and its C ABI
dynamic library. Runtime protocol 19.0 and debug protocol 4.0 over common wire 2.0 are
development contracts: by explicit project policy they
do not promise backward compatibility until a frontend exists.

Design conflicts follow the project-wide order: cross-client/cross-platform support,
architectural purity, then strict reference behavior. See
[Design principles](design-principles.md) and the living
[compatibility status](runtime-compatibility-status.zh-CN.md).

## Authority and ownership

One runtime session is the aggregate authority for the game. It owns the VM instance,
game phase, logical clock, input waits, presentation model, pending external requests,
save/reload transactions and debugger pause state. VM variables and frames still have
one physical copy inside the VM; the runtime uses `VmRuntimePort`, `VmDebugInspect` and
`VmDebugControl` rather than mirroring or reaching into VM fields.

The frontend owns filesystem access, directory discovery, decoding, renderer objects,
audio devices, operating-system input and monotonic-clock sampling. Renderer and device
objects are projections. A frontend result that affects EraBasic execution is submitted
as an ordered message and becomes authoritative only when the runtime accepts it.

The runtime is authoritative for presentation intent, not realized frontend layout. It
does not measure fonts, reproduce a frontend's line wrapping or make raster caches part
of recoverable presentation. If an EraBasic operation explicitly queries such a result,
the target design obtains it from the single authoritative projection frontend as a
revision-bound external service response. The runtime validates and orders that response
without interpreting the renderer algorithm. Mirrors and debugger clients cannot answer
projection queries.

## Versioned envelope

`era-protocol` defines the common deterministic-CBOR envelope. Every request, response
and event carries:

- common wire and channel-specific major/minor versions;
- runtime or debug channel;
- session identity, timeline `SessionEpoch`, direction sequence and message identity;
- optional correlation identity;
- stable payload tag and payload bytes.

Numeric CBOR field and variant identifiers are permanent. Encoders use definite
collections, shortest scalar forms and ascending numeric map fields. Decoders validate
the raw data before typed decoding, rejecting alternate encodings and duplicate/reordered
fields while allowing canonical unknown minor-version fields to be ignored. Payload and
envelope byte limits are checked before dispatch.

Major versions must overlap. Minor versions are additive and selected during hello.
Unknown features are unavailable unless both peers negotiate them. A future client/server
adapter uses the same envelope with an outer length frame; authentication, encryption and
reconnection policy belong to that transport adapter.

## Lifecycle

The normative state flow is:

```text
ABI session allocation
  -> ClientHello / ServerHello
  -> ProjectManifest / ProjectLoadReport
  -> Ready
  -> Start(new game | traditional save | exact VM snapshot)
  -> Running <-> WaitingInput
  -> optional WaitingExternal / DebugPaused / Reloading
  -> ShutdownRequest -> Stopping -> ShutdownReady -> destroy
```

A version rejection, unrecoverable project error or execution fault enters `Faulted`.
Only diagnostic, resynchronization and shutdown traffic is accepted there. Destroying a
session without `ShutdownReady` is an emergency operation and may abandon unacknowledged
storage requests.

Messages in each direction have strictly increasing sequence numbers. Active-session envelopes
must carry the current epoch. Message IDs are
never zero. A response copies the initiating ID into `correlation_id`. Retransmission
uses the same message and idempotency IDs; a runtime must not apply it twice. The
frontend acknowledges runtime sequence numbers. A journal gap is repaired with a full
presentation/runtime snapshot rather than guessing missing deltas.

## Project and external services

At project load or reload, the frontend submits a deterministically sorted manifest.
Each entry contains a normalized relative path, category and one of UTF-8 text, binary
bytes or the I/O error observed by the frontend. Source positions always use UTF-8 byte
offsets. Absolute, drive-qualified and parent-traversing paths are invalid.

Runtime-initiated work is asynchronous:

- storage operations cover reads, metadata queries, atomic/idempotent writes, recursive or
  top-level listing and deletion in
  project, save, global-save, DAT, log and resource namespaces;
- platform services cover font metrics, image/canvas operations, audio, networking,
  opening URLs and negotiated extensions;
- every request has a stable request ID, idempotency key and optional runtime-logical deadline;
  writes and deletes carry an explicit `Any`, `Missing`, or `Revision(value)` commit precondition;
- shutdown and other cancellation policies use a typed cancellation message. Late responses are
  rejected by request ID and cannot modify the new runtime timeline.

Frontend-observation requests additionally bind the presentation revision and frontend
environment revision they observe. A stale response is rejected. If the selected
frontend lacks the exact operation/version, the query is unsupported; the runtime does
not substitute a logical-width or default-font approximation. These projection-query
operations and payloads are implemented for projection observations and pointer state in
protocol 14.0. Further font/raster operations must use the same revision-binding rule.

The runtime and all lower crates perform no concrete file I/O and sample no system clock.
Current ordinary text, binary and gzip traditional saves are encoded and decoded in memory. Export is
eligible only at a stable untimed input wait; restore validates a candidate VM state before an
atomic commit. Large explicit imports and exports use one negotiated transfer in each direction:
the sender declares kind, exact length and a raw BLAKE3 digest, chunks are contiguous and ordered,
and a committed import is consumed once by `Start`. Traditional saves therefore never rely on the
single-envelope payload limit. Slot and Host-command bytes cross only storage messages. Schema-aware UTF-8 text
saves use the reference positional and named-group layout with BOM/CRLF output. Current global,
character DAT, arbitrary UTF-8 text and canonical runtime-log operations are connected; the pinned
reference's unimplemented `SAVEVAR`/`LOADVAR` remain a stable unsupported operation. Exact VM
snapshots are available at stable, untimed VM input waits. Their checksummed runtime wrapper binds
the VM image to the exact artifact and normalized project/resource identity, restores canonical
presentation and pending input state atomically, and regenerates epoch-bound wait/interaction
identities. Stable runtime-owned title/save/load/overwrite waits are eligible: their semantic menu
state is persisted, while restore re-lists storage metadata and rebinds fresh revisions and tokens.

Storage is negotiated as `RuntimeFeature::Storage`. `Stat` returns metadata without transferring
contents. `List { pattern, recursive }` returns frontend-relative entries; runtime sorts any list
that is observable by EraBasic. `Missing` is create-only, `Revision` is compare-and-replace/delete,
and a precondition mismatch is reported as a storage error rather than silently overwriting data.
Protocol 12 additionally negotiates whether the frontend can return revisions, perform atomic
replacement, enforce `Missing`, and delete. Candidate saves require the first three guarantees;
menu deletion requires the fourth and always uses the revision observed during the current listing.

## Input, QTE and presentation

The wait contract represents all reference input kinds: enter, any key, integer,
string, void, any value, integer/string button and primitive mouse/key input. It retains
one-input, message-skip, system-input, mouse-input, default-value, timeout-display and
timeout-message fields. The frontend submits normalized UI intents such as committed text,
continue, primitive input or activation of an opaque token. Primitive input intentionally uses
frontend-normalized EraBasic-shaped `input_type` and `result_1..result_4` device fields. The
frontend never supplies game-internal `RESULT[5]`; an optional runtime-issued selection token
maps to integer `RESULT[5]`, or to `RESULTS` with `RESULT[5]=0`. Only runtime synthesizes timeout
type 4. The runtime parses EraBasic values
and validates the wait, token, epoch and frontend monotonic timestamp.

The runtime accepts frontend messages in sequence order. If input and a deadline share a
timestamp, the lower sequence wins. Timed/QTE waits are transient and block VM snapshots.
`FrontendInput.message_skip` updates the runtime-owned message-skip state. A timed input with
its sixth argument present may take the reference shortcut using its default; `FORCEWAIT`
clears that state. Countdown values are computed by the runtime, and a queued wait does not
receive a deadline until it becomes the visible active wait.
Stable input waits are snapshot candidates only when every other VM/runtime eligibility
condition also succeeds. Debug pause freezes logical time; time spent paused does not
consume a QTE deadline.

The instruction-level rules for `TINPUT`, `TONEINPUTS`, `TWAIT`, `FORCEWAIT` and
`GETKEY` are fixed in [input-wait-compatibility.md](input-wait-compatibility.md).
In particular, positive deadlines and fresh key-state queries are transient,
while deadline-free Enter/value input can be stable.

Protocol 15 adds the negotiated `InputUndo` feature. The runtime publishes
`InputUndoStateChanged` (tag 38) with the available history length and an epoch-scoped,
single-use token. A frontend maps Ctrl-Z, a gesture, or an accessibility action to
`InputUndoRequest` (tag 37); it never submits a platform key code or save bytes. The runtime
restores its retained traditional-save checkpoint and exact SFMT state, replays accepted scalar
inputs through normal adjudication, and exposes the resulting stable wait. Replay is transient
and therefore blocks VM snapshot creation. Successful bytecode hot reload invalidates the trace.

Protocol 16 adds one-shot project analysis, runtime-owned key macro profiles, and a
portable Host extension registry. Analysis returns structured diagnostics without replacing
the active project or creating a VM. Key macro edits expose canonical UTF-8 `macro.txt`
content and persist through the ordinary frontend storage contract. Extensions are declared
before project loading, compiled as `rustyera.extension` Host imports, and invoked through
negotiated `ServiceKind::Extension` operations. CLR `CALLSHARP` remains unsupported.

Protocol 19 separates canonical Era logical coordinates, canvas texels and frontend projection
units. `ProjectionObservation` declares a revisioned rational transform. Physical history, HTML
layout, font metrics and canvas pixels use typed operation payloads which echo that causal context;
canvas sampling additionally echoes the canvas replay revision. The runtime session has one
authoritative frontend and does not implement multi-client authority transfer.

Protocol 19 also replaces opaque HTML presentation strings with a fixed-dialect semantic document
tree. Text and element nodes retain UTF-8 byte ranges, button nodes carry runtime-issued interaction
tokens, and image/shape/div attributes remain available to a capable renderer. `PRINT_IMG`,
`PRINT_RECT`, and `PRINT_SPACE` preserve their optional resources and mixed font-relative/logical
lengths in canonical presentation runs. These changes are intentionally wire-incompatible with
protocol 17; no released frontend depended on the development protocol.

The frontend may send `ProjectAnalysisRequest` only after negotiating `ProjectAnalysis` and
while the runtime is idle before its first load (`Negotiating`) or in `Ready`. ERH files are always analyzed; an
empty ERB selection means all ERB files. `KeyMacroProfileSubmit`, `KeyMacroCommand`, and
`InputIntent::ActivateKeyMacro` require `KeyMacros`. Macro activation recalls text into the
runtime-owned textbox; ordinary `CommitText` performs expansion and submits the resulting
pieces across successive waits. `ExtensionRegistrySubmit` requires `ExternalServices`, is
accepted only before the first project load, and forms part of project/snapshot identity.
Each extension service result contains one typed return value and ordinal mutable-argument
writes; the runtime validates the entire response before committing any write.

`QUIT` and restart variants publish a persistent `ExitRequested` intent. It is repeated in
resynchronization state until the frontend completes the normal shutdown lifecycle; restart
creates a new session rather than reusing VM or runtime state.

The hello exchange includes ordered BCP-47 locale preferences. Runtime selects and persists one
supported locale. Canonical system text carries both the runtime-selected text and a semantic key
with arguments, allowing accessible clients to understand its role without becoming authoritative
for wording or game state.

The runtime stores a revisioned semantic presentation snapshot and emits deltas based on
that revision. It includes an ordered semantic history journal, text/styles/buttons, typed HTML,
image/shape intent, exact-rational backgrounds,
tooltip policy, parsed sprite definitions, canvas replay commands, logical audio state, title and
the current wait. Script-defined logical media coordinates use fixed
integer units rather than floating point. Recoverable state is separate from acknowledged
one-shot `EffectBatch` events. Every effect has an independent ID; the frontend returns an
exact completed/failed/cancelled outcome rather than acknowledging an ambiguous prefix. Device
failures produce diagnostics and never rewrite already-decided game state. `ColumnCell` preserves
`PRINTC`/`PRINTLC` alignment and preferred-column intent while keeping buttons and interaction
tokens runtime-owned. `Separator` preserves `DRAWLINE` as a semantic line role. Neither inserts
font-dependent padding into authoritative state; GUI clients may use grid/flex layout, TUI clients
may repeat a pattern, and clients without these nodes receive a deterministic plain projection.
Physical WinForms history, realized HTML layout and pixel-width queries are not canonical
state. Protocol 19.0 exposes command-specific, revision-bound services for physical history,
HTML measurement/splitting, text extents and canvas raster samples. A pending query is a transient
external wait and blocks snapshots and hot-reload commits. `HTML_POPPRINTINGSTR` is the exception:
it serializes and consumes the runtime-owned pending semantic buffer. Protocol 19 also carries
runtime-owned redraw policy, logical TextBox placement and per-button generation/enabled state.
REDRAW bit 2 is an acknowledged `PresentNow` effect, while snapshots remain synchronized when
automatic paint is disabled. Pixel buffers, font objects
and audio devices remain frontend caches. Content-addressed source-image facts remain distinct
from realized canvas and presentation observations.

Canvas replay covers pixel writes, fills, brush/pen/font/dash state, lines, text, sprite and
canvas composition, color matrices, masks and rotation. Source-canvas commands bind the source
revision captured at issue time. `GCREATEFROMFILE` uses Resource storage for a zero relative flag
and Data storage otherwise; `GLOAD`/`GSAVE` use `Save/imgNNNN.png`. All paths pass the shared
relative-path validator, image decode/PNG encode are typed Canvas services, and runtime performs
neither filesystem I/O nor rasterization.

## Runtime and VM boundary

`VmRuntimePort` is the runtime execution interface. `drive` executes a bounded slice
and returns `VmHostRequest` values only after instruction dispatch has unwound. The runtime
then stages its own transition, asks the VM to validate a typed completion, commits the VM
completion and finally publishes runtime events. No VM instruction invokes a callback that
can mutate runtime state.

`RuntimeVm` adapts the interpreter to this caller-pumped interface. Snapshot restore uses
the same prepare/rebind/commit shape. VM-native services may only
touch deterministic VM-owned state; anything involving runtime or frontend capabilities
is a `CallHost` request. Existing `VmHost` remains available for lower-level embedding;
the runtime itself uses the adapter and never invokes runtime code from instruction
dispatch.

The VM and runtime implement generation-pinned project-delta hot reload between VM slices.
Stable waits and source breakpoints are rebound only after a successful commit. Canvas and
dynamic-sprite replay state is preserved. New or changed image payloads use the same negotiated,
versioned image-metadata service as initial loading. The runtime keeps the old artifact and resource
graph authoritative until all candidate metadata has been validated, then commits them atomically.

## Current implementation status

The current runtime implements handshake, locale/semantic-system-text negotiation,
epoch-scoped sessions, normalized text input,
capability intersection, bounded journals with idempotent retransmission, full-state
resynchronization, full in-memory project load/analyze/compile/validate, deterministic new-game
startup, bounded VM driving, logical-line text/column/separator presentation, reference-shaped
waits and timeouts, fresh GETKEY-family queries, frontend-owned local time, seed acquisition,
SFMT-backed RAND/RANDOMIZE, transactional array/find/regex Native operations, current ordinary
text/binary/gzip traditional-save export and atomic restore, chunked exact snapshots at stable VM
input waits, normalized incremental reload with generation-pinned VM commit, correlated slot
listing/loading, faults and cancellation-aware shutdown.
The compiler uses a persisted, validator-checked operation contract rather than Host-name
heuristics. Configuration
and resource inputs receive deterministic identities and diagnostics; semantic configuration needed
by save/shop and logical layout is retained while GUI/device options remain frontend state. Map,
mutable XML, the fixed XPath subset and reference-shaped DataTable XSD/XML execute through
transactional Native place writes. Canonical presentation includes logical lines/buttons,
message-skip state, backgrounds, tooltips, resource sprites, canvas replay and logical audio state.
Image metadata and pixels are typed frontend services; audio actions use an exact-outcome effect
journal. `UPDATECHECK` uses typed network and open-URL services, while focus remains a reported
client state queried by `ISACTIVE`. Built-in shop autosave now uses the isolated candidate
`SAVEINFO` transaction and revision-checked storage commit. Runtime-owned fixed-page save/load
menus, overwrite confirmation and nested SAVEGAME/LOADGAME cancellation continuations are stable
input operations. Protocol 14.0 retains the canonical animation redraw interval in resource replay;
the frontend schedules redraws but never advances authoritative game time. Protocol 12.0 separates
external waits from debugger pauses. The independent debug channel supports creator-bounded scope
grants, coherent stop tokens, global pause/continue/stepping, source breakpoints, fiber/frame/stack
inspection, atomic variable writes and runtime game-field inspection. Only
`input.message_skip` is debug-writable. Debug console execution accepts the currently implemented
EraBasic expression subset (operator precedence, ternary expressions and a pure-method whitelist)
and atomic scalar assignment; Host calls, flow, waits, increment/decrement and unsupported methods
are rejected without mutation. Protocol 12.0 added operation-versioned service capabilities,
resource decoder services, exact effect outcomes, tooltip state and runtime diagnostics. The
session-fixed `available_fonts` list is used only for the script-observable `CHKFONT` result.
Canonical semantic layout intent remains runtime-owned; realized device layout does not.
Protocol 19.0 selects font metrics only when `gget_text_size` is advertised at its exact operation
version. Physical-history, HTML-layout and canvas-pixel operations are negotiated independently.
The frontend cannot change the available-font list after the handshake.
