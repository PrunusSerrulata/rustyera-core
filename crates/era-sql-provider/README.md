# Native SQL service provider

This optional host-side crate implements the existing SQL service protocol. It is not a VM or
runtime dependency and performs no project filesystem I/O. Hosts implement `SqlStorage` using
their project-scoped Resource/Data adapter, including atomic writes and opaque revision CAS.

SQLite and its persistent readers live on one owner thread. The only unsafe boundary is the
private cursor module; connection ownership is `Rc`-based and is never made `Send` or `Sync`.
The caller services storage continuations synchronously and must bound its own storage callbacks.
SQL execution retains the protocol's 5-second budget; transport has a 30-second bound and shutdown
waits at most 5 seconds for confirmation. These are not performance acceptance thresholds.

Register a `ProviderRole::Live` before submitting requests. At most one detached `Candidate` may
coexist. Retiring a failed candidate preserves the live provider; promotion releases the old live
provider. Provider identifiers must increase lexicographically by `(service_epoch, id)` across an
owner's lifetime. Reset releases all providers but does not permit retired identifiers to return.
Each registered provider retains the protocol's connection and reader limits.

Resource seeds and exact revisions are hydrated from immutable bytes. Storage format v1 continues
to use its historical 3.53.0 identity anchor while the executing engine is exactly SQLite 3.53.4.
Reading an old revision does not rewrite it. Publication writes an immutable blob and then updates
the current pointer using host CAS. Errors with uncertain or committed-but-unresolved outcomes
close the affected connection and readers; `commit_outcome` remains in structured error context.
Never retry these operations on a different SQL backend.

Native builds require the verified prebuild described in
[`tools/sqlite-native/README.md`](../../tools/sqlite-native/README.md). A system SQLite installation
or a rusqlite `bundled` feature is not an alternative. The final linked engine is independently
checked for its source ID and compile options. Browser/WASM consumers must not depend on this crate.

Tauri routing, native audit evidence, and end-to-end performance acceptance are separate host
integration work; this crate alone does not establish any game-response latency improvement.
