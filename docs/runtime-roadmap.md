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
reassigned below; historical formats remain assigned to Batch 11.

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
Host/Native oracle coverage belongs to Batch 10, while current-save and system-flow oracle coverage
belongs to the final persistence closure in Batch 11.
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
- Batch 7 is closed at this checkpoint. Its unfinished higher-level features are reassigned by
  dependency: structured Native execution to Batch 8, running-reload rebinding to Batch 9, and the
  persistence controller plus runtime-owned menu snapshots to Batch 11. They must not be described
  as Batch 7 implementations before their destination batch lands.

## Batch 8: media and platform services

Implementation checkpoint:

- Native ABI 8 now supports immutable place views and transactional multi-place writes. Map is
  executable with insertion order and array outputs; the pinned XML read subset preserves mixed
  content and rejects unsupported XPath forms deterministically; DataTable supports typed columns,
  deterministic row handles, row/cell mutation and the approved filter/sort subset.
- Declared VAREXT state participates in ordinary/global reset and load transactions, binary
  `0x20`--`0x22` records, snapshots and hot reload. Unknown records are retained losslessly. VM
  snapshot v3 and Runtime Snapshot v2 reject older incompatible state.
- Runtime protocol 9.0 freezes the CHKFONT family set at handshake. Project configuration owns the
  logical viewport and PRINTC dimensions. Canonical presentation now retains style, alignment,
  HTML, image/shape and logical audio state and deterministically projects unsupported media to
  text/omission without moving game-rule decisions into the frontend.
- Batch 8 closes at this checkpoint. The remaining compatibility-heavy tail is assigned to the
  beginning of Batch 10: mutable XML operations and wider XPath, reference XSD/XML DataTable
  interchange, exact presentation line/button/message-skip behavior, resource-manifest sprite and
  canvas replay graphs, intrinsic image metrics, backgrounds/tooltips, typed audio/video effects,
  URL/update/focus services and their capability-failure matrix. These items must not be described
  as implemented before Batch 10 lands. They do not block the debugger work in Batch 9.

## Completed: batch 9, debugger implementation

- Before debugger dispatch, complete running-reload rebinding for stable VM and runtime waits,
  pending system-controller entries, presentation tokens and source-map identities. Stage every
  rebind against the target artifact and roll back the complete reload if any reference is absent
  or ambiguous; transient operations remain reload blockers.
- Resolve runtime-generated UnsupportedRuntimeFeature, input and Native faults through bytecode
  source maps so every available fault carries its command and UTF-8 source location.
- Dispatch the independent debug channel with explicit scope grants.
- Implement VM inspection/control ports, runtime game-field descriptors, global pause, stepping,
  breakpoints and hot-reload rebinding.
- Freeze QTE time during debug stops and bind every grant/stop token to epoch, generation and
  runtime revision. Console execution is limited to the reference-safe subset.

Batch 9 delivered runtime protocol 10.0, debug protocol 3.0 and C ABI 2.1. Runtime and debug
channels have independent ordered streams. Creator policy bounds renewable epoch/generation grants;
every stopped-state operation validates the session epoch, VM pause epoch, program generation and
runtime revision. The VM exposes global pause, instruction/source/call-aware stepping, fibers,
frames, read-only operand stacks, atomic variable mutations and source-map breakpoints. Statement
fingerprints relocate a source breakpoint only when its function-local target is unique; otherwise
the breakpoint remains unbound without rejecting reload. Existing frames remain pinned to their old
generation while new calls use the new artifact.

Runtime faults now preserve the originating Host/Native command and UTF-8 source position whenever
the VM supplied one. `WaitingExternal` and `DebugPaused` are distinct phases, and frontend time
samples observed during a debugger stop rebase rather than advance the authoritative QTE clock.
Runtime game fields are separately described, with only `input.message_skip` writable. Console
evaluation currently supports scalar literals and visible scalar variables; safe execution supports
one scalar assignment and rejects flow, waits, Host effects and unsupported expressions before any
mutation. This is deliberately narrower than the reference debug console's full method-safe
instruction set and can be extended additively inside a later debugger-compatibility slice.

## Remaining dependency split

The remaining work has two dependency layers. Runtime-surface operations must first receive final
transaction, persistence, snapshot, asynchronous-wait and capability classifications. A candidate
save cannot safely clone or reject execution until those classifications are closed. The save/menu
controller then depends on that candidate transaction, and runtime-owned menu snapshots depend on
the controller's final stable states. Historical readers and end-to-end compatibility tests come
last because they must target the final current-format and continuation behavior.

The split retains the already approved architectural decisions: presentation is canonical and
cross-platform rather than Windows-GDI pixel-identical; candidate-save failure rolls back buffered
effects instead of leaking them like the reference implementation; menu deletion remains an
explicit extension; filesystem I/O remains frontend-owned; and all submitted text remains UTF-8.
No additional reference conflict or user choice is introduced by this split.

## Batch 10: runtime-surface compatibility and transaction substrate

Status: reopened after the 2026-07-17 final-surface audit. The first implementation checkpoint is
published in [`runtime-operation-contracts.md`](runtime-operation-contracts.md), but the catalog
cannot be called closed while deterministic analyzer-visible operations still reach an
unregistered Native or generic Host fault. The six approved physical-history queries, raster/file
GDI operations and `GETMEMORYUSAGE` remain intentional stable unsupported differences.

Implement in this dependency order:

Items 1 and 2 are independent foundations and may proceed in parallel. Item 3 depends on the
canonical presentation model from item 2; item 4 may proceed beside item 3. Items 5--8 are the
convergence path and begin only after items 1--4 have fixed their public behavior.

1. Complete the structured Native tail: mutable XML operations, the required wider XPath subset,
   and reference XSD/XML DataTable interchange. Preserve prevalidate-then-commit behavior and give
   every mutation explicit ordinary/global/save-extension, snapshot and hot-reload treatment.
2. Complete canonical presentation semantics before media services consume them: exact logical
   line/button/message-skip behavior, recoverable display history, and the remaining presentation
   queries. For `GETDISPLAYLINE` and the Emuera HTML helpers, either implement deterministic
   canonical semantics or retain a tested `UnsupportedRuntimeFeature` difference where real-game
   usage and the approved cross-platform model do not justify pixel-dependent behavior.
3. Build resource-manifest sprite/canvas replay graphs, intrinsic image metadata handling,
   backgrounds and tooltips over canonical presentation state. The runtime owns semantic resource
   identities and replay state; the frontend continues to own decoding, rendering and file I/O.
4. Add typed audio/video effects and URL/update/focus platform services with complete capability,
   cancellation, ordering and failure matrices. Device actions remain one-shot effects; only
   recoverable semantic state may enter runtime snapshots.
5. Audit every non-persistence Host and Native operation and freeze its rollback safety,
   persistence scope, snapshot eligibility, hot-reload behavior, external-wait stability and
   capability fallback. No unclassified stateful operation may pass the Batch 10 completion gate.
6. Extend the debug console from its current scalar subset to the reference method-safe subset
   that can execute under the classifications above. Continue to reject flow control, waits,
   partial instructions and Host effects; parse/validation/execution failure must leave VM,
   Native, runtime and presentation state unchanged.
7. Extend the reference CLI and Rust differential fixtures for the changed Native, presentation,
   resource, media and platform Host paths. Record genuine headless or platform oracle gaps rather
   than treating Rust-only round trips as compatibility proof. Add focused real-game slices for
   presentation-heavy menus and resource/media dispatch without loading the full script corpus.
8. Require every pinned non-persistence built-in to have a working tested implementation or a
   documented stable intentional-difference diagnostic. Publish the finalized operation
   classification table as the explicit handoff gate to Batch 11.

Batch 10 must not implement candidate `SAVEINFO` writes against a partially classified runtime.
It is complete only when Batch 11 can clone or reject every operation reachable during candidate
execution without consulting frontend/device state synchronously.

First-checkpoint details: mutable XML and the fixed XPath subset; reference-shaped DataTable XSD/XML;
logical-line/button/skip behavior; resource manifests, image metadata and pixel services;
canvas/dynamic-sprite replay state; backgrounds and tooltip policy; logical audio plus exact
one-shot effect outcomes; typed update/open-URL services and focus state; bytecode-persisted
operation contracts; and parser-backed atomic safe-console expressions. New or changed image bytes
during hot reload deliberately require a full project load, while unchanged resource metadata and
runtime-created replay state are preserved. The reopened closure slice adds an independent
persisted candidate policy (`ReadOnly`/`CloneCommit`/`BufferedEffect`/`FrozenClock`/`Forbidden`),
container 8, ISA 3, HIR 7, compiler ABI 12, Native ABI 11, Host ABI 7 and VM ABI 4. It also closes the
high-use `SETBIT`/`CLEARBIT`/`INVERTBIT`, `SPLIT`, `GETNUM`, `GETPALAMLV`/`GETEXPLV`, `STRCOUNT`
and `ESCAPE` paths, including the missing mutable/reference analyzer signatures. The next closure
slice implements `STRLENS`, `STRLENSU`, `STRFINDU`, `CHARATU`, `ENCODETOUNI`, `UNICODEBYTE`,
`TOLOWER`, `TOUPPER` and reference-array `STRJOIN`; project-derived `BARSTR` is now a read-only Host
operation and is safe inside candidate `SAVEINFO`. BMP/ASCII behavior has a same-input C# oracle.
Under the repository's UTF-8-only rule, non-U lengths remain UTF-8 byte counts. U lengths match
.NET UTF-16 code-unit counts, while supplementary-plane U positions use Unicode scalar indices
because a valid Rust UTF-8 string cannot represent an isolated UTF-16 surrogate. The latter is a
tested intentional difference from .NET indexing.

Second closure checkpoint (2026-07-17): `UNICODE` now has the reference integer-to-string
signature and BMP/control-character behavior, replacing the reversed string-to-integer
approximation. `TOFULL`, `TOHALF` and `MONEYSTR` are deterministic read-only Host operations backed
by project configuration and version-1 width/format tables. Changed images now use a staged
metadata transaction during incremental reload; the live artifact and resource graph remain
unchanged until every response succeeds. Exact Runtime Snapshot v5 persists the selected locale and
culture-table version and rejects mismatched restores. The persisted compatibility set is container
9, ISA 3, HIR 8, compiler ABI 13, Native ABI 12, Host ABI 8 and VM ABI 5; VM Snapshot remains format
4 and is rejected through the new ProgramVersion when necessary.

This checkpoint still does not close Batch 10. Locale-aware casing and the complete .NET numeric
custom-format surface, the remaining reflection/metadata Native families, semantic canvas animation
tail, debug-console dispatch unification and final zero-unknown execution matrix remain. These must
finish before the Batch 11 controller is described as complete.

Third closure checkpoint (2026-07-17): analyzer-visible built-ins no longer default to an assumed
Native implementation. The compiler now emits a stable unsupported diagnostic unless a built-in is
in the explicit Native allowlist or is replaced by a classified Host binding; the execution-catalog
test checks this classification for the complete analyzer inventory. Exact `TOINT`, `ISNUMERIC`,
`CONVERT` and `COLOR_FROMRGB` behavior is implemented as pure Native code. `TOSTR`, `VARSIZE`,
`EXISTFUNCTION`, `EXISTVAR`, `GETDOINGFUNCTION`, `ENUMFUNC*` and `ENUMVAR*` are versioned Host
operations over the active artifact/fiber. Function enumeration excludes event handlers, matching
is case-insensitive, and the return value is the number actually copied into the optional output
array. Reference parameters now carry place types through HIR lowering, bytecode validation and VM
frames, so introspection and writes follow the bound variable instead of the zero-sized declaration
placeholder. The persisted compatibility set is container 10, ISA 3, HIR 8, compiler ABI 14,
Native ABI 13, Host ABI 9 and VM ABI 6.

The C# and Rust same-input reflection fixture covers ordinary function/variable enumeration, and
the numeric fixture covers the four newly exact Native functions. `ISDEFINED`, `ENUMMACRO*`,
`GETMETH`, `GETMETHS`, `EXISTMETH`, `GETVARS` and `ERDNAME` remain stable unsupported: the bytecode
does not yet persist the preprocessor macro table or the reference dynamic-expression name tables,
and the focused eraTW corpus contains no uses of these families. Returning zero or an empty string
would therefore be a false compatibility claim. Batch 10 still requires either a versioned metadata
section for those tables or a documented final intentional-difference decision, plus the semantic
canvas animation tail, locale-aware casing, the remaining .NET integer custom formats,
debug-console dispatch unification and the final Host implementation matrix.

### Final Batch 10 checkpoint (2026-07-17)

Batch 10 is complete. The debugger pure-method whitelist now calls the same CoreNative dispatcher
as VM execution, including the exact Era numeric parser. Runtime protocol 13 persists semantic
canvas animation frames and the `SETANIMETIMER` redraw cadence in renderer-independent resource
replay. The compiler inventory test gives every analyzer built-in exactly one Native, Host or
stable-unsupported class, while [`runtime-operation-contracts.md`](runtime-operation-contracts.md)
records the final physical-GDI, dynamic-metadata, invariant-casing and unsupported-format choices.
The selected eraTW casing call sites are ASCII and its integer format strings are all inside the
implemented deterministic subset.

## Batch 11: persistence, controller and final compatibility closure

### Final Batch 11 checkpoint (2026-07-17)

- Runtime protocol 13 negotiates revision, atomic-replace, missing-precondition and delete storage
  guarantees explicitly. Direct script storage commands retain reference-compatible unconditional
  overwrite behavior; candidate writes require the stronger negotiated guarantees.
- The VM can fork an isolated memory/Native timeline without live fibers and can later commit only
  its authoritative state while retaining the caller's stacks. Runtime Snapshot v6 and VM Snapshot
  v4 reject older layouts after the ABI change.
- Built-in shop autosave now performs `Stat`, chooses `Missing` or the observed `Revision`, obtains
  one Clock sample, executes a fresh bounded `SAVEINFO` fiber against cloned VM/Native/runtime and
  buffered presentation/effect state, and writes atomically. Candidate faults, forbidden waits,
  capability failures, conflicts and storage errors discard the candidate; a successful write is
  the commit point. Once the atomic write request is emitted, shutdown cannot cancel that commit
  window. Custom `SYSTEM_AUTOSAVE` and direct `SAVEDATA` retain their reference roles.

The runtime-owned title/save/load controller now scans and classifies slot metadata, displays fixed
pages of twenty, includes autosave slot 99 on the final load page, confirms overwrites and performs
revision-bound deletion when the frontend negotiated that extension. Nested `SAVEGAME` and
`LOADGAME` preserve their suspended Host continuation; cancellation resumes it, successful save
commits the isolated candidate before resuming it, and successful load replaces authoritative game
state. `TITLE_LOADGAME` has reference precedence. Ordinary load runs `SYSTEM_LOADEND`, then
`EVENTLOAD`, then enters SHOP without immediately repeating shop autosave.

Exact Runtime Snapshot v6 admits stable runtime-owned title/save/load/overwrite waits. It stores
semantic controller/menu state and the selected overwrite slot without transport tokens. Restore
increments `SessionEpoch`, rebinds fresh waits/buttons, and re-lists slot metadata so deletion never
reuses a stale revision. Structured interaction-key maps use an ordered pair representation inside
the JSON payload, avoiding JSON's string-key restriction while rejecting duplicate tokens.
Candidate transactions, QTEs, active storage/service work and mismatched bytecode generations remain
explicit blockers.

The UTF-8 text decoder accepts the markerless EraMaker envelope and reference historical extension
layouts 1700, 1708, 1729 and 1803 as read-only migrations. Current writes remain versioned current
format. The pinned reference binary reader accepts only its current 1808 binary layout, so there is
no fabricated historical binary migration path.

Small Rust fixtures cover corruption, paging, overwrite/delete preconditions, conflict recovery,
nested continuation, post-load ordering, candidate rollback and stable snapshot rebinding. A
read-only `eraTW-minimal` corpus audit pins the actual title, shop, `EVENTLOAD` and `SAVEINFO` call
sites; runtime execution tests continue to use reduced fixtures instead of making the full real-game
corpus a default test dependency. The audit also exposed and fixed diagnostic line counting at a
UTF-8 byte offset that is not a character boundary.

The headless C# CLI exposes parsing, evaluator/VM execution, `PUTFORM` and `SAVENOS`, but it does not
expose an in-memory persistence transport, revision conflicts, cancellation or runtime snapshots.
Those architecture-specific paths therefore have Rust state-machine and codec tests, not a claimed
C# differential. Current-format scalar/native behavior continues to use the existing same-input
oracle fixtures. Adding a reference persistence endpoint would require a new headless backend hook
or reference-owned filesystem transaction adapter and is deliberately recorded as an oracle gap,
not an unimplemented Rust runtime feature.

With those endpoint limits and the intentional differences in
[`runtime-operation-contracts.md`](runtime-operation-contracts.md), Batches 10 and 11 are closed.
Future compatibility work must be opened as a new audited roadmap rather than silently reopening
these completed batches.

## Verification required for every batch

Add focused Rust unit/integration tests, then run `cargo fmt --all -- --check`,
`cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and
`git diff --check`. Run `tools/emuera-reference-cli/test-macos-wine.sh` on macOS (or the documented
Windows smoke script), followed by a same-input Rust/C# comparison for changed behavior.
Reference source remains read-only unless the user separately authorizes an isolated headless
change.

## Post-Completion Review

All work assigned to Batches 10 and 11 is either implemented or closed by a tested, documented
intentional difference/oracle limitation above. There are no tasks left to redistribute within this
roadmap.
