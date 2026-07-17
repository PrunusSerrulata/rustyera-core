# Runtime operation contract table

This table is the Batch 10 handoff gate for candidate-save work. Every analyzer-visible built-in
has exactly one compiler catalog entry. Native and Host imports persist the resolved contract in
bytecode container 8; the validator rejects incoherent combinations or legacy flags which do not
match the contract. Unknown runtime dispatch never silently succeeds: it emits
`UnsupportedRuntimeFeature` with the import name.

## Contract dimensions

| Dimension | Persisted values | Runtime meaning |
| --- | --- | --- |
| State owner | `Pure`, `Vm`, `Native`, `Presentation`, `Controller`, `External` | The only authority an operation may mutate. |
| Transaction | `ReadOnly`, `CloneCommit`, `BufferedEffect`, `Forbidden` | Normal-execution mutation/rollback boundary. |
| Candidate SAVEINFO | `ReadOnly`, `CloneCommit`, `BufferedEffect`, `FrozenClock`, `Forbidden` | Isolated-save behavior, independently of the normal execution transaction. |
| Persistence | `None`, `Ordinary`, `Global`, `VariableScoped`, `ExtensionScoped`, `ProjectDerived`, `RuntimeOnly` | Which durable state receives a committed mutation. |
| Snapshot | `Included`, `Rebuild`, `Excluded`, `PendingBlocks` | Whether exact snapshots contain, reconstruct, omit, or reject pending state. |
| Hot reload | `Preserve`, `Rebuild`, `Invalidate`, `ActiveBlocks` | Treatment when a new artifact generation is committed. |
| Wait | `Immediate`, `StableInput`, `TransientExternal` | Scheduler and snapshot stability of an unfinished call. |
| Capability fallback | `NotApplicable`, `CanonicalProjection`, `IntentNoOp`, `ScriptResult`, `Unsupported` | Deterministic behavior when a frontend capability is absent. |
| Debug | `Pure`, `Transactional`, `Forbidden` | Safe-console eligibility. |

## Final operation classes

| Operation family | State / transaction | Persistence | Snapshot / reload / wait | Capability fallback | Debug |
| --- | --- | --- | --- | --- | --- |
| Arithmetic, comparison, conversion and other pure Native methods | Pure / read-only | None | Included / preserve / immediate | Not applicable | Pure |
| VM variable and character mutations | VM / clone-commit | Variable-scoped | Included / preserve / immediate | Not applicable | Transactional |
| Ordered map, XML and DataTable mutations | Native / clone-commit | Extension-scoped | Included / preserve / immediate | Not applicable | Transactional |
| Random generation | Native / clone-commit | Runtime-only | Included / preserve / immediate | Not applicable | Forbidden |
| Text, HTML, logical lines, styles, buttons, backgrounds and tooltips | Presentation / clone-commit | Runtime-only | Included / preserve / immediate | Canonical projection | Forbidden |
| Audio/video/animation device actions | Presentation / buffered effect | Runtime-only | Excluded / preserve / immediate | Intent no-op | Forbidden |
| Stable untimed input | Controller / forbidden | Runtime-only | Included / preserve / stable input | Script result | Forbidden |
| Timed input, key state, image pixels, clock and network services | External / forbidden | Runtime-only | Pending blocks / active blocks / transient external | Script result or unsupported | Forbidden |
| Storage and system-flow operations | Controller or external / forbidden | Ordinary, global or runtime-only | Per-operation included or pending-blocking policy | Script result or unsupported | Forbidden |
| Extension Host imports | External / forbidden | Runtime-only | Pending blocks / active blocks / transient external | Unsupported | Forbidden |

The compiler test `every_analyzer_builtin_has_one_explicit_execution_class` enumerates both
instruction and function catalogs and verifies classification completeness, contract coherence,
derived effects and snapshot capability. Artifact validation repeats the checks for untrusted
containers.

## Stable intentional differences and unsupported surface

- `GETDISPLAYLINE`, `HTML_GETPRINTEDSTR`, `HTML_POPPRINTINGSTR`, `HTML_STRINGLEN`,
  `HTML_SUBSTRING` and `HTML_STRINGLINES` return `UnsupportedRuntimeFeature`. Their reference
  results depend on the WinForms physical line/history model; RustyEra keeps canonical logical
  lines and does not invent pixel-dependent values.
- XPath supports the documented deterministic element/attribute, descendant, wildcard, numeric,
  `last()`, attribute and text predicate subset. Namespace axes, unions and arbitrary XPath
  functions return `native.xpath.unsupported` without committing a mutation.
- DataTable IDs are deterministic monotonic integers. The reference implementation embeds a
  stopwatch timepoint, which makes identical inputs produce different XML.
- Image decoding stays frontend-owned. Project load obtains versioned intrinsic metadata before
  commit and `SPRITEGETCOLOR` requests a pixel on demand. Hot reload preserves canvas and dynamic
  sprite replay state for unchanged image bytes; a new or changed image returns
  `runtime.reload_image_metadata_requires_full_load` and requires a full load.
- Resource-backed canvas animation construction not covered by the implemented replay subset,
  physical GDI drawing helpers and any other catalogued but unimplemented non-persistence Host
  operation fail with the stable `UnsupportedRuntimeFeature` import diagnostic. This is preferable
  to a frontend-dependent partial result and is mechanically distinct from VM faults.
- Primitive mouse/key events remain frontend-normalized into EraBasic-shaped fields by design.
  Runtime validates the wait, token, selection and timeout but does not interpret platform events.

Candidate `SAVEINFO` execution accepts `ReadOnly`, `CloneCommit`, and `BufferedEffect` operations,
and resolves `FrozenClock` from the single sample obtained before the candidate starts. It rejects
`Forbidden`, every other wait, and every operation whose persisted contract is missing or invalid.
Normal transaction safety remains a separate field: for example, system-flow commands can be
transactional during ordinary execution while still being forbidden in a candidate.
