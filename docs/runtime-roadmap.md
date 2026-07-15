# Runtime implementation roadmap

This is the persistent handoff plan for runtime development. Compatibility is pinned to Emuera
commit `26a35dc9334bb67590b96f7b8efbefbf199e391e`. Development protocols are intentionally not
backward compatible until the project explicitly changes that policy.

## Completed: batch 1, protocol and execution catalog foundation

- Common wire 2.0, runtime protocol 3.0 and debug protocol 2.0 bind active traffic to a
  `SessionEpoch`.
- Frontends submit normalized UI intents and opaque interaction tokens. EraBasic parsing,
  timeout decisions and result writes remain runtime responsibilities.
- Client capabilities, ordered device/lifecycle reports, recoverable presentation state and
  acknowledged one-shot effect contracts are separate.
- Runtime output is journaled, acknowledgements prune it, exact retransmissions are idempotent,
  and resynchronization returns epoch, runtime state and a complete presentation snapshot.
- Analyzer built-ins have a deterministic Native/Host/unsupported execution classification.
  Compiler Host routing no longer uses name-prefix heuristics. CLR `CALLSHARP` is an intentional
  compile-time incompatibility; future extensions use the versioned Host ABI.
- Configuration, resource manifests and resource payloads are no longer silently discarded:
  invalid I/O/encoding is diagnosed and valid deferred handling has a stable warning.

## Batch 2: deterministic Native, input and text runtime

- Implement reference SFMT/RAND, seed/RANDDATA, RANDOMIZE/INITRAND/DUMPRAND and snapshot-capable
  Native state, followed by core string, array, character, map, XML and DataTable operations.
- Complete INPUT/TINPUT/BINPUT/TWAIT/AWAIT/FORCEWAIT/GETKEY/primitive input, message-skip,
  ISTIMEOUT, countdown updates and concurrent runnable fibers during QTE waits.
- Implement the text presentation reducer: PRINT suffixes, waits, style/alignment, buttons,
  line mutation, skip/log, HTML text semantics and script-observable presentation queries.
- Add typed cancellation, operation-specific service errors and pending-operation deadlines.

## Batch 3: reference system controller

- Implement pure runtime states for TITLE, TRAIN, AFTERTRAIN, ABLUP, TURNEND, SHOP, FIRST and
  NORMAL, including BEGIN/QUIT/restart legality and system function transitions.
- Drive VM entrypoints only through runtime ports and atomically commit RESULT, SOURCE, BOUGHT,
  training/shop and other authoritative game fields.
- Parse and apply submitted configuration/resource manifests; retain the normalized project
  snapshot required for reload.

## Batch 4: current traditional saves and storage

- Add an I/O-free `era-runtime-save` crate for the pinned current ordinary/global save formats,
  variable/character DAT, text and log formats.
- Implement slots, SAVEINFO, overwrite, autosave, TITLE_LOADGAME, SYSTEM_LOAD and EVENTLOAD.
- Make save and restore prepare/validate/commit transactions. Storage bytes only cross versioned
  frontend messages, and failed restore leaves the live timeline unchanged.

## Batch 5: exact snapshot restore and hot replacement

- Wrap VM snapshot plus runtime system state, presentation, stable waits, logical clock, IDs and
  Native state in a checksummed exact-artifact container.
- Implement `VmRestorePort`, wait rebinding and atomic restore; reject every transient QTE,
  service, storage or old-generation state.
- Stage project deltas with incremental analyze/compile/validate, then migrate compatible Native
  state and rebind waits/breakpoints atomically. A successful commit advances `SessionEpoch`.

## Batch 6: media and platform services

- Implement images, shapes, backgrounds, sprite/canvas, font/image metrics, tooltips, logical
  audio/BGM, video, URL, network update and focus/device services.
- Runtime owns resource identities, canonical scene and logical channels. Frontends only measure,
  cache, render and play; deterministic fallback is used unless missing capability changes an
  observable script result, in which case startup is rejected.

## Batch 7: debugger implementation

- Dispatch the independent debug channel with explicit scope grants.
- Implement VM inspection/control ports, runtime game-field descriptors, global pause, stepping,
  breakpoints and hot-reload rebinding.
- Freeze QTE time during debug stops and bind every grant/stop token to epoch, generation and
  runtime revision. Console execution is limited to the reference-safe subset.

## Batch 8: legacy saves and compatibility closure

- Add reference-supported historical save readers after current formats are stable.
- Require every pinned built-in to have tests and a working implementation or a documented,
  stable intentional-difference diagnostic.
- Run focused real-game project slices for startup, system flow, saves, reload and long sessions;
  keep ordinary unit fixtures small.

## Verification required for every batch

Add focused Rust unit/integration tests, then run `cargo fmt --all -- --check`,
`cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and
`git diff --check`. Run `tools/emuera-reference-cli/test-macos-wine.sh` on macOS (or the documented
Windows smoke script), followed by a same-input Rust/C# comparison for changed behavior.
Reference source remains read-only unless the user separately authorizes an isolated headless
change.
