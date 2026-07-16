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

## Completed: batch 3, reference system controller foundation

- HIR 4 and container 4 retain event attributes/definition order. Bytecode event groups preserve
  `ONLY`, `PRI`, normal and `LATER` ordering (including PRI+LATER duplication), and the runtime
  controller implements exact-one `#SINGLE` group skipping.
- `LOCAL`, `LOCALS`, `ARG` and `ARGS` use persistent storage keyed by normalized function name;
  same-name event definitions share it, while private dynamic variables remain frame-local.
- Native ABI 3 adds VM-mediated RANDDATA exchange. `INITRAND`/`DUMPRAND` validate and atomically
  exchange all 625 cells; `SWAP` uses the same prevalidated-place transaction rule. The initial
  remaining math set is implemented and `UseNewRandom=true` produces one stable warning while
  SFMT remains authoritative.
- Runtime protocol 5.0 carries message-skip state, runtime countdown values and persistent
  `ExitRequested` state. Sixth-argument timed-input shortcuts, queued deadline activation,
  INPUTANY integer/string routing and forced wait skip cancellation are implemented.
- Submitted configuration is parsed in manifest order, semantic loader/analyzer options are
  applied, `SortWithFilename=false` preserves manifest order, and filename sorting gives
  directories containing `#` reference priority. The normalized project snapshot is retained.
- `BEGIN`/`FORCE_BEGIN` share the pinned fork's forced-transition behavior. Initial entry dispatch
  exists for TITLE/FIRST/TRAIN/AFTERTRAIN/ABLUP/TURNEND/SHOP; QUIT/restart variants emit the
  persistent exit intent.

## Completed: batch 4, traditional-save and storage foundation

Completed in the first Batch 4 implementation slice:

- Runtime protocol 6.0 negotiates and persists a supported locale, carries semantic system-text
  identities beside canonical runtime-selected text, and gives storage/service requests optional
  logical deadlines plus explicit cancellation messages. Development protocol compatibility is
  still intentionally not retained.
- HIR 5, compiler ABI 5 and Native ABI 4 preserve read-only as well as mutable array places.
  ARRAYSHIFT, ARRAYREMOVE, ARRAYSORT, direct-place ARRAYCOPY, FINDELEMENT/FINDLASTELEMENT and
  REGEXPMATCH execute as prevalidated VM transactions. String search uses a documented common
  .NET/Rust regex subset; lookaround, backreferences, conditionals and atomic groups fail closed.
- The I/O-free `era-runtime-save` crate reads and writes current 1808 ordinary binary and gzip
  containers, enforces resource limits, round-trips named scalar/1D/2D/3D values, and preserves
  Map/XML/DataTable extension payloads opaquely. It validates and losslessly retains UTF-8 current
  text payloads, but does not guess their project-specific positional layout.
- Traditional binary save export is restricted to stable untimed input waits. Restore decodes and
  validates a complete candidate memory image before commit, advances `SessionEpoch`, discards
  execution state, preserves opaque extensions, then dispatches SYSTEM_LOADEND followed by
  EVENTLOAD. A failed restore leaves the active VM timeline unchanged.
- The title fallback can list and read save slots through correlated frontend storage messages.
  Runtime sorts and bounds entries, owns interaction tokens and semantic slot selection, and
  cancels outstanding storage/service requests during shutdown. Frontend code never parses save
  bytes.
- Runtime configuration now retains autosave, binary/compression selection, slots per page,
  currency placement and maximum shop-item settings. The canonical system menu has Japanese,
  English and Simplified Chinese projections; locale is part of resynchronization.

The unfinished portions of the original Batch 4 plan are reassigned below instead of keeping this
batch indefinitely open. Batch 5 establishes the remaining execution/state prerequisites. Batch 6
then closes current-format persistence and the system flows that depend on it. Exact snapshots and
hot replacement move to Batch 7 because they must not freeze incomplete Native, pending-operation
or authoritative runtime-state schemas.

## Batch 5: Native, authoritative game state and operation foundations (implemented)

Implemented in dependency order:

1. Finish the remaining mutable array family and dynamically evaluated variable-name form of
   ARRAYCOPY. Preserve the existing prevalidate-then-commit transaction rule for every mutation.
2. Implement the pinned CSV and character Native services on those place transactions. Unsupported
   extensions continue to fault explicitly; they must not silently manufacture defaults.
3. Define authoritative runtime fields and VM ports for SOURCE, BOUGHT, training state, shop state
   and other controller-owned values. VM requests mutations through ports; it never gains direct
   ownership of runtime state.
4. Complete the storage-independent TITLE, FIRST, TRAIN, AFTERTRAIN, ABLUP, TURNEND, SHOP and
   NORMAL transitions, including purchase validation and retained currency/shop configuration.
   Persistence-dependent title/load and shop/autosave continuations close in Batch 6.
5. Replace the separate input, service and storage pending maps with one typed pending-operation
   registry. It owns deadlines, epoch binding, interaction tokens, cancellation policy, shutdown
   behavior and operation-specific recoverable errors. This registry is the only asynchronous
   operation substrate used by later storage, media, snapshot and debugger work.

Batch 5 acceptance requires transaction rollback tests for every mutable Native family, controller
state-transition tests, and pending-operation tests for completion, timeout, cancellation, stale
epoch, duplicate response and shutdown.

Batch 5 landed the dynamic ARRAYCOPY form, VARSET/CVARSET, ARRAYMSORT/ARRAYMSORTEX, array queries,
the pinned character/CSV query and mutation layer, SORTCHARA and RESET_STAIN. Mutable operations
validate a cloned candidate or every destination before their first write. The runtime state port
now supports whole-array fills across shared or all-character storage, so the controller resets
SOURCE, training scratch arrays and character fields without mirroring EraBasic variables in
runtime-owned structs.

The controller now drives TITLE/FIRST, TRAIN (including COM_ABLE discovery, NEXTCOM, DOTRAIN and
continuous CALLTRAIN queues), AFTERTRAIN, ABLUP, TURNEND, SHOP and NORMAL termination. Presentation
buttons become runtime-owned command intents identified by epoch-bound tokens. Purchases validate
the retained maximum item count, item name, sale flag, price and money before atomically updating
MONEY, ITEM and BOUGHT. A project-defined SYSTEM_AUTOSAVE runs; when autosave is enabled but that
function is absent, runtime reports the documented unsupported feature instead of pretending the
Batch 6 storage transaction succeeded.

All input, service, storage and delay waits now live in one typed pending-operation registry. It
separates overlapping external ID domains, consumes completions once, orders ready deadlines
deterministically, binds operations to SessionEpoch, enforces the common pending limit and owns
shutdown cancellation. Minimal last-line temporary/empty/delete/replace semantics required by
system error recovery are canonical runtime presentation state, not frontend behavior.

The Batch 5 reference differential covers VARSET ranges, CSVNAME/CSVBASE, ADDCHARA/ADDVOIDCHARA,
GETCHARA and ARRAYMSORT with identical fixture data and watched values. The current reference CLI
does not expose the internal system-state pump independently from its coupled console, so aggregate
runtime controller sequencing, pending-operation ownership and token validation remain a documented
oracle endpoint gap; they are verified by Rust actor/port tests derived from the audited reference
transition order.

## Batch 6: current saves, storage and system-flow closure

Implement in this dependency order:

1. Add the project-schema positional adapter for generating and restoring current text saves.
   Binary remains the safe runtime export when text layout cannot be proven. UTF-8-only text is an
   intentional encoding difference and must remain explicit in diagnostics and tests.
2. Complete current global saves, variable/character DAT, text and log codecs, then expose
   SAVEDATA/LOADDATA, SAVEGLOBAL/LOADGLOBAL, SAVEVAR/LOADVAR, SAVECHARA/LOADCHARA and related Host
   paths without performing filesystem I/O.
3. Build slot writes, SAVEINFO/CHKDATA, overwrite confirmation, deletion and autosave as atomic
   transactions over the Batch 5 pending-operation registry. Decode and validate before commit;
   failed or cancelled operations leave both VM and runtime timelines unchanged.
4. Finish TITLE_LOADGAME, SHOP/SYSTEM_AUTOSAVE and their failure/any-key continuations. Complete
   the authoritative TITLE through NORMAL system-flow integration and apply the retained save,
   autosave, currency and shop settings.
5. Add same-input reference comparisons for every current format and Host path the reference CLI
   can expose. Where the CLI lacks an endpoint, add a documented oracle gap rather than treating a
   Rust round trip as compatibility proof.

Batch 6 establishes current-format traditional persistence and the Host/storage foundation for the
system-flow work inherited from the original Batch 4 plan. Its controller-dependent remainder is
reassigned below; historical formats remain assigned to Batch 10.

### Batch 6 implementation checkpoint (2026-07-16)

The dependency and Host layers are implemented:

- Runtime protocol 7.0 negotiates `Storage`, adds `Stat`, recursive lists and explicit
  `Any`/`Missing`/`Revision` preconditions. No protocol-6 compatibility adapter is retained during
  the pre-frontend development period.
- The current UTF-8 text codec now has a project-schema positional adapter, emits BOM plus CRLF,
  and supports ordinary and global 1808 layouts. Binary and gzip codecs continue to cover normal,
  global, variable and character file kinds. User `CHARADATA` and multidimensional saved strings
  are rejected for text-save projects, matching the reference format's representability limits.
- VM exports separate ordinary/global/character scopes. Ordinary restore preserves live globals;
  global load overlays only global storage; character DAT load appends atomically. Runtime-only
  reset and `LASTLOAD_*` transactions avoid exposing calculated fields to scripts.
- `SAVEDATA`, `LOADDATA`, `DELDATA`, `SAVEGLOBAL`, `LOADGLOBAL`, `SAVECHARA`, `LOADCHARA`,
  `CHKDATA`, `CHKCHARADATA`, `FIND_CHARADATA`, `SAVETEXT`, `LOADTEXT`, `EXISTFILE`, `ENUMFILES`,
  `OUTPUTLOG`, `PUTFORM`, `SAVENOS`, `RESETDATA` and `RESETGLOBAL` use the pending-operation
  registry and never perform filesystem I/O. The pinned reference deliberately throws for
  `SAVEVAR`/`LOADVAR`; Rust reports the same stable unsupported feature instead of inventing data.
- `SaveDataNos` now means the clamped total ordinary slot count (20--80); the built-in page size is
  fixed at twenty and autosave uses slot 99. A missing project `SYSTEM_AUTOSAVE` now performs the
  built-in storage write instead of faulting.
- The macOS/Windows reference smoke fixture now executes the same `PUTFORM suffix` and `SAVENOS()`
  inputs covered by the Rust runtime test; both produce `SAVEDATA_TEXT == "suffix"` and `20`.

The unfinished controller work from this batch has been reassigned by dependency instead of
leaving Batch 6 open. Candidate save transactions, slot interaction and load continuations are
prerequisites of an exact runtime snapshot, so they form the first phase of Batch 7. Broader
current-save and Host oracle coverage belongs to the final compatibility closure in Batch 10.
Until those destinations land, the corresponding behavior must not be described as implemented.

Intentional architecture-first differences remain: failed/cancelled loads do not clear opaque
state, arbitrary paths reject traversal rather than sanitizing it, filesystem results are sorted
before becoming script-visible, and `OUTPUTLOG` contains canonical runtime presentation plus a
stable runtime/game header instead of UI/device/patch-directory details.

## Batch 7: exact snapshot restore and hot replacement

Implementation checkpoint:

- Runtime protocol 8.0 now uses checksummed, ordered chunk transfers for traditional saves and VM
  snapshots instead of embedding potentially large payloads in `Start`.
- Submitted resource manifests and payloads have deterministic normalized identities. Current
  binary save extension records have typed, order-preserving Map/XML/DataTable codecs.
- Exact Runtime Snapshot v1 and VM Snapshot v2 round-trip stable untimed VM input waits, including
  VM/native state, canonical presentation, controller state, logical time and token rebinding.
- Project deltas are normalized, compiled incrementally and committed through the VM's
  generation-pinned hot-reload path. Native state shared by stable imports is migrated instead of
  being silently reset.
- The remaining items below are still required before Batch 7 is complete: executable
  Map/XML/DataTable Native builtins, candidate SAVEINFO transactions, full slot/delete controller,
  runtime-owned menu snapshots, and exact wait/controller rebind validation during running reloads.

Implement in this dependency order:

1. Add a candidate save transaction that runs frontend Clock and speculative `SAVEINFO` against
   cloned VM/runtime/presentation state. Reject external waits from the candidate and publish its
   buffered presentation only when the storage commit succeeds.
2. Build the complete title/save controller over `CHKDATA` metadata: fixed-size pages, empty-slot
   selection, overwrite confirmation, deletion, any-key recovery and revision-bound interaction
   tokens. Storage writes must use `Missing` or the observed `Revision`, never an unqualified
   overwrite.
3. Suspend nested `SAVEGAME`/`LOADGAME` only in the reference `__CAN_SAVE__` states. Cancellation
   and successful save resume the suspended continuation; a successful load discards it with the
   replaced VM timeline.
4. Complete `TITLE_LOADGAME` precedence and the exact post-load `SYSTEM_LOADEND` -> `EVENTLOAD` ->
   SHOP sequence, including suppression of the immediately following shop autosave.
5. Route built-in autosave through the same Clock/`SAVEINFO` transaction, add its failure any-key
   continuation, and select `Missing` versus `Revision` from the observed slot metadata. This
   replaces the temporary `SAVEDATA_TEXT` plus `Any` baseline retained by Batch 6.
6. Implement Map, XML and DataTable Native state after the ordinary game-rule Native layer is
   stable. Give every stateful Native service a deterministic schema and migration policy before
   exact snapshots or hot replacement can include it.
7. Normalize submitted resource manifests and payload identities as opaque project state before
   reload is implemented. Media-specific validation and capability projection remain deferred.
8. Wrap VM snapshot plus runtime system state, presentation, stable waits, logical clock, IDs and
   Native state in a checksummed exact-artifact container. Only the stable waits established after
   the persistence-controller phase are snapshot-eligible.
9. Implement `VmRestorePort`, wait rebinding and atomic restore; reject every transient QTE,
   service, storage or old-generation state.
10. Stage project deltas with incremental analyze/compile/validate, then migrate compatible Native
    state and rebind waits/breakpoints atomically. A successful commit advances `SessionEpoch`.

## Batch 8: media and platform services

- Complete canonical text presentation semantics: style and alignment state, buttons, line
  mutation, skip/log behavior and HTML_PRINT logical text. Build BINPUT choices from canonical
  runtime buttons and finish message-skip suppression/retention rules. Keep GETDISPLAYLINE and
  the deferred HTML query family unavailable until a deterministic non-GDI contract is approved.
- Consume the normalized resource identities from Batch 7 and apply media-specific validation,
  capability projection and deterministic fallback here.
- Implement images, shapes, backgrounds, sprite/canvas, font/image metrics, tooltips, logical
  audio/BGM, video, URL, network update and focus/device services.
- Runtime owns resource identities, canonical scene and logical channels. Frontends only measure,
  cache, render and play; deterministic fallback is used unless missing capability changes an
  observable script result, in which case startup is rejected.

## Batch 9: debugger implementation

- Resolve runtime-generated UnsupportedRuntimeFeature, input and Native faults through bytecode
  source maps so every available fault carries its command and UTF-8 source location.
- Dispatch the independent debug channel with explicit scope grants.
- Implement VM inspection/control ports, runtime game-field descriptors, global pause, stepping,
  breakpoints and hot-reload rebinding.
- Freeze QTE time during debug stops and bind every grant/stop token to epoch, generation and
  runtime revision. Console execution is limited to the reference-safe subset.

## Batch 10: legacy saves and compatibility closure

- Add reference-supported historical save readers after current formats are stable.
- Extend the reference CLI with current-format ordinary/global/character/text/log and Host-path
  fixtures, then run same-input semantic comparisons. Cover metadata, failure and continuation
  behavior where the headless reference exposes it; record genuine endpoint gaps explicitly.
  Rust-only codec round trips remain coverage and never count as compatibility proof.
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

## Post-Completion Review

After completing a batch of tasks, redistribute any tasks from that batch that remain unimplemented
into the remaining batches according to their dependencies. No new batches may be added.
