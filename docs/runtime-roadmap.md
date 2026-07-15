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

## Completed: batch 2, deterministic Native, input and text foundation

Completed in the first Batch 2 implementation slice:

- Runtime protocol 4.0 and C ABI 2.0 add semantic column/separator capability negotiation.
  Wire 2.0 and debug protocol 2.0 are unchanged.
- HIR format 3 preserves mutable instruction arguments as places. Container 3.0, ISA 2.0,
  compiler ABI 3 and Native ABI 2 add `MakePlace`; bytecode validation and the interpreter
  understand the operand. Compiler control placeholders now fault instead of returning defaults.
- SFMT-19937 uses the pinned 624-word algorithm, high-word-first `u64`, low-32-bit seeds and
  snapshot state. New-game seed acquisition initializes the Native registry. `RAND` and
  `RANDOMIZE` share this state and reject invalid operands.
- The initial stateless Native set covers integer/bit operations, UTF-8 and scalar string lengths,
  conversions, Unicode, MIN/MAX/LIMIT/INRANGE, search, replace and byte/scalar substring. Non-U
  positions use UTF-8 bytes and advance partial positions to valid boundaries; this is the
  documented difference from reference code-page mode.
- Primitive input deliberately remains frontend-normalized. The frontend sends EraBasic-shaped
  device fields and an optional opaque selection token; runtime atomically commits `RESULT[0..5]`
  and `RESULTS`. Only runtime can synthesize timeout type 4. This is an explicit exception to raw
  device collection, not delegation of acceptance, timeout or game rules.
- INPUTMOUSEKEY uses its positive deadline. Timed completions update sticky `ISTIMEOUT`; untimed
  waits leave it unchanged. `AWAIT` accepts 0..10000 ms and positive values use logical deadlines.
- One wait is visible and later fiber waits queue in scheduler order. A foreground wait no longer
  stops runnable fibers; stable `WaitingInput` begins after runnable work is exhausted. PRINTW
  commits its logical line before opening an Enter wait.
- Presentation has an uncommitted logical-line buffer. PRINTC/PRINTFORMC retain right-aligned
  cells, PRINTLC/PRINTFORMLC left-aligned cells, and DRAWLINE is an independent separator after a
  flush. Capability fallback uses one ASCII space and deterministic plain separator text.
- `GETLINESTR` uses 75 deterministic Unicode logical columns, does not split graphemes and rejects
  empty/zero-width patterns. Deferred display/HTML queries fault as
  `UnsupportedRuntimeFeature` rather than returning placeholders.

Batch 2 closes at the reusable execution, input-wait and presentation foundations above. Work
that requires authoritative game state, storage transactions, full presentation services or
debugger source inspection is assigned to the corresponding later batch below.

## Batch 3: reference system controller

- Add VM-mediated Native place transactions. Use them to make `INITRAND`/`DUMPRAND` atomically
  exchange all 625 RANDDATA cells and to implement mutable array/sort/find operations. Runtime
  always uses SFMT; `UseNewRandom=true` is ignored with one stable compatibility warning.
- Complete the remaining game-rule input behavior: TINPUT sixth-argument message-skip, actual
  message-skip state, countdown updates, queued-wait deadline activation, BINPUT button
  construction, INPUTANY parsing and cancellation. Preserve normalized `PrimitiveInput`; runtime
  must not map operating-system keys, buttons or coordinates.
- Add the remaining math, regex and CSV Native functions, followed by character, Map, XML and
  DataTable state needed by system functions. Define deterministic snapshot schemas for every
  stateful Native service. Missing functions must fault rather than return type-default values.
- Implement pure runtime states for TITLE, TRAIN, AFTERTRAIN, ABLUP, TURNEND, SHOP, FIRST and
  NORMAL, including BEGIN/QUIT/restart legality and system function transitions.
- Drive VM entrypoints only through runtime ports and atomically commit RESULT, SOURCE, BOUGHT,
  training/shop and other authoritative game fields.
- Parse and apply submitted configuration/resource manifests; retain the normalized project
  snapshot required for reload.

## Batch 4: current traditional saves and storage

- Generalize input waits, frontend services and storage work into a typed pending-operation
  registry with deadlines, operation-specific errors, cancellation policies and atomic shutdown.
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

- Complete canonical text presentation semantics: style and alignment state, buttons, line
  mutation, skip/log behavior and HTML_PRINT logical text. Keep GETDISPLAYLINE and the deferred
  HTML query family unavailable until a deterministic non-GDI contract is approved.
- Implement images, shapes, backgrounds, sprite/canvas, font/image metrics, tooltips, logical
  audio/BGM, video, URL, network update and focus/device services.
- Runtime owns resource identities, canonical scene and logical channels. Frontends only measure,
  cache, render and play; deterministic fallback is used unless missing capability changes an
  observable script result, in which case startup is rejected.

## Batch 7: debugger implementation

- Resolve runtime-generated UnsupportedRuntimeFeature, input and Native faults through bytecode
  source maps so every available fault carries its command and UTF-8 source location.
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
