# Runtime and frontend protocol

This document specifies interfaces for a future RustyEra runtime. The repository does
not yet contain that runtime or a usable dynamic library.

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

## Versioned envelope

`era-protocol` defines the common deterministic-CBOR envelope. Every request, response
and event carries:

- common wire and channel-specific major/minor versions;
- runtime or debug channel;
- session identity, direction sequence and message identity;
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
  -> optional Paused / Reloading
  -> ShutdownRequest -> Stopping -> ShutdownReady -> destroy
```

A version rejection, unrecoverable project error or execution fault enters `Faulted`.
Only diagnostic, resynchronization and shutdown traffic is accepted there. Destroying a
session without `ShutdownReady` is an emergency operation and may abandon unacknowledged
storage requests.

Messages in each direction have strictly increasing sequence numbers. Message IDs are
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

- storage operations cover reads, atomic/idempotent writes, listing and deletion in
  project, save, global-save, DAT, log and resource namespaces;
- platform services cover font metrics, image/canvas operations, audio, networking,
  opening URLs and negotiated extensions;
- every request has a stable request ID; writes additionally have an idempotency key and
  may carry an expected revision.

The runtime and all lower crates perform no concrete file I/O and sample no system clock.
Traditional save bytes are serialized/deserialized by a future runtime but persisted by
storage requests. `StateExportRequest` selects traditional data or a VM snapshot; an
ineligible snapshot returns structured reasons instead of silently falling back. VM
snapshots are accepted only for the exact artifact identity.

## Input, QTE and presentation

The wait contract represents all reference input kinds: enter, any key, integer,
string, void, any value, integer/string button and primitive mouse/key input. It retains
one-input, message-skip, system-input, mouse-input, default-value, timeout-display and
timeout-message fields. Every input includes the wait ID, button generation and frontend
monotonic timestamp. Stale IDs and generations are rejected.

The runtime accepts frontend messages in sequence order. If input and a deadline share a
timestamp, the lower sequence wins. Timed/QTE waits are transient and block VM snapshots.
Stable input waits are snapshot candidates only when every other VM/runtime eligibility
condition also succeeds. Debug pause freezes logical time; time spent paused does not
consume a QTE deadline.

The instruction-level rules for `TINPUT`, `TONEINPUTS`, `TWAIT`, `FORCEWAIT` and
`GETKEY` are fixed in [input-wait-compatibility.md](input-wait-compatibility.md).
In particular, positive deadlines and fresh key-state queries are transient,
while deadline-free Enter/value input can be stable.

The runtime stores a revisioned semantic presentation snapshot and emits deltas based on
that revision. It includes text/styles/buttons, HTML, image/shape placement, backgrounds,
logical audio state, title and the current wait. Numeric media measurements use fixed
integer units rather than floating point. Pixel buffers, font objects and audio devices
remain frontend caches; script-observable service results return through ordered service
responses and update the runtime's logical resource revision.

## Runtime and VM boundary

`VmRuntimePort` is the normative prospective interface. `drive` executes a bounded slice
and returns `VmHostRequest` values only after instruction dispatch has unwound. The runtime
then stages its own transition, asks the VM to validate a typed completion, commits the VM
completion and finally publishes runtime events. No VM instruction invokes a callback that
can mutate runtime state.

Snapshot restore uses the same prepare/rebind/commit shape. VM-native services may only
touch deterministic VM-owned state; anything involving runtime or frontend capabilities
is a `CallHost` request. Existing `VmHost` remains temporarily available for the current
interpreter, and `Vm` intentionally does not implement the prospective port yet.

Hot reload is supplied as a project revision/delta, compiled and validated by a future
runtime, and committed only between VM slices. Old frames remain generation-pinned.
Presentation waits and source breakpoints are rebound only after successful commit.
