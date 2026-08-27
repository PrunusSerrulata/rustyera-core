# Batch 0 compatibility audit tools

Run these commands only after the batch's single refactor review and inside its shared test budget.
They are not CI pass certificates. The tool remains a separate Cargo workspace.

```sh
cargo run --manifest-path tools/runtime-tester/Cargo.toml -- baseline ../games /absolute/output/baseline.json
cargo run --manifest-path tools/runtime-tester/Cargo.toml -- coverage ../games /absolute/output/coverage.json --all-games --markdown /absolute/output/coverage.md
cargo run --manifest-path tools/runtime-tester/Cargo.toml -- coverage ../games/eratw-sub-modding /absolute/output/snake.json --profile emuera.skia.snake
```

The output directory must already exist. Existing output files are never overwritten. Use the
snake worktree's isolated Cargo target directory, never the main workspace build products.

`baseline`, `coverage`, and `snake-observations` run beneath a separate supervisor process.
Every five seconds it writes the complete latest observed state to stderr; two identical samples
fail immediately and terminate the worker process group, including a worker blocked in a load or
compile call. Snapshots contain actual input paths/bytes, analysis or compilation progress,
diagnostics, pending requests and full responses where applicable, never synthetic polling progress.
`ERA_AUDIT_BUDGET_SECONDS` bounds the entire command (default 3600); callers must set it no larger
than the remaining shared batch budget. Failed snapshot files are retained in the reported temporary
directory. The outer test runner must also terminate descendant process groups on its own deadline.
JSON results remain exclusively on stdout or the requested output file.

## Baseline

`baseline [GAMES_ROOT] [OUTPUT.json]` inventories snake TW and the seven fixed regression games.
It records Git HEAD only when that project has its own `.git`, dirty paths, a hash of the tracked
patch, hashes of untracked files, and sorted BLAKE3 file/group/content manifests. Relative paths
and file bytes both contribute to identity. Original raw bytes are hashed without text decoding.

The three components and both reference wrappers have separate identities. Reference semantic
SHAs and initial wrapper SHAs are distinct fields; a dirty wrapper is not represented by HEAD
alone. Resources, source databases, ALS/ERD, and configuration are included independently of
today's frontend ingestion filters. Runtime saves/logs/cache and `Data/sql` overlays are excluded
with reasons. Interior symlinks are not followed and remain explicit incomplete inputs.

## Coverage

`coverage PROJECT [OUTPUT.json]` accepts `--all-games` (PROJECT becomes the games root),
`--profile emuera.em|emuera.skia.snake`, `--markdown OUTPUT.md`, `--analyzer-options OPTIONS.json`,
and `--csv-options OPTIONS.json`. Option JSON must deserialize to the current public option
structs. Analysis mode and inclusion of uncalled functions are always forced on. All effective
options are serialized into the report.

Configuration resolution uses the actual core resolver. An explicit audit profile override is
recorded separately from the project's resolved identity; invalid project configuration still
blocks compilation. This tool does **not** infer all legacy game configuration values. Default
audit options and the full analyzer/parser distinction are documented in `input_policy`. Use
explicit options when comparing a configuration-dependent analysis with an oracle.

Only UTF-8 source is accepted (an optional BOM is removed before parsing); invalid bytes are
retained by raw inventory hash and cause an input diagnostic plus blocked compilation. Locations
are byte offsets into the decoded UTF-8 input, not C# UTF-16 offsets. Canonical ERB/CSV roots win
over archived copies; all original files remain in the inventory. CSV/ALS/ERD inputs reach the
real CSV loader, and ERB/ERH reach the real analyzer/compiler.

Appearances come from the EraBasic parser, including uncalled function bodies, expression calls,
operators, declarations, arity/omitted arguments and raw dynamic targets. Statements that do not
appear in its active AST are reparsed as explicitly unverified candidates, so preprocessing and
unknown syntax do not silently remove names. The appearance pass uses the default parser context
with profile/lexer/debug switches and ERH macros, not the analyzer's complete symbol context.
Analyzer and compiler diagnostics remain independent evidence. A project failure preserves all
appearances and available diagnostic locations; it never produces a fabricated compile pass.

Registry `Native`, `Host`, and `Unsupported` are separate from VM and runtime source citations.
Exact string references are inspection pointers, not proof of a handler. The known
`DT_COLUMN_OPTIONS` registration/dispatch gap has a dedicated regression. Compiler Trap is
reported only from an actual `UnsupportedConstruct` diagnostic, not just an Unsupported registry
entry. Frontend TUI/Browser/Tauri fields remain `unverified` without runtime capability or execution
capture; `unsupported_capability` cannot be inferred from missing text matches. Every row's dynamic
verification is `not_run`; the separate oracle driver owns dynamic results.

Tests for these modules are written but were not executed during implementation. A baseline or
coverage JSON is generated only when the reviewed tool is actually run; no illustrative result is
checked in as evidence.
