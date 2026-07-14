# RustyEra

RustyEra is a UTF-8 Rust implementation of the EraBasic language front end and
project-data loading contracts used by Emuera.EM. Compatibility is pinned to
reference commit `26a35dc9334bb67590b96f7b8efbefbf199e391e` (the Emuera 1.824
family). Compatibility with that fixed implementation takes priority over
redesigning the language.

Only UTF-8 input is supported. The Rust crates do not detect or decode
Shift-JIS, GBK, or other legacy encodings.

## Current architecture

The workspace currently contains these implemented components:

| Crate | Responsibility |
| --- | --- |
| `erabasic-ast` | Syntax AST, UTF-8 byte spans, and stable diagnostics shared by the lexer and parser. |
| `erabasic-lexer` | Context-sensitive tokenization, caller-selected terminators, macros, and FORM string decomposition. |
| `erabasic-parser` | Expressions, logical lines, ERH declarations, ERB functions, preprocessors, and block structure. |
| `erabasic-data` | Deterministic, Serde-compatible project schema, static data, and initialization/save-loading contracts for future consumers. |
| `erabasic-csv` | Emuera-compatible loading of an in-memory project file snapshot. It performs no filesystem I/O. |
| `erabasic-hir` | Deterministic, Serde-compatible typed expressions, variables, functions, lines, and control-flow links. |
| `erabasic-analyzer` | Project-level ERH/ERB symbol resolution, type checking, declaration processing, instruction reduction, and control-flow analysis. |
| `erabasic-bytecode` | Versioned VM-native instructions, the single `CallHost` boundary, source maps, canonical `.erbc` containers, and patches. |
| `erabasic-compiler` | Deterministic parallel HIR lowering with function-level incremental reuse. |
| `erabasic-validator` | Structural, type, control-flow, stack, capability, and ABI validation for HIR and untrusted bytecode. |
| `erabasic-vm` | Deterministic interpretation, cooperative multi-fiber scheduling, Host/native calls, snapshots, traditional save-state views, and generation-pinned hot reload. |
| `erabasic-repl` | A small development REPL for manually inspecting lexer and parser behavior. |

The currently implemented data flows are:

```text
EraBasic UTF-8 source -> erabasic-lexer -> erabasic-parser -> erabasic-ast

frontend paths + UTF-8 contents/I/O errors -> erabasic-csv -> erabasic-data

ProjectData + frontend ERH/ERB paths + UTF-8 contents/I/O errors
    -> erabasic-analyzer -> enriched ProjectData + erabasic-hir

AnalyzedProject -> erabasic-compiler -> erabasic-validator
    -> self-contained .erbc bytes + incremental cache/patch

ValidatedArtifact -> erabasic-vm <-> runtime-provided Host/native services
```

Public types are re-exported from each crate root. Larger implementations are split
into modules by syntax, data domain, executable format, or compilation phase.

## Scope and unimplemented components

The Rust implementation does **not** currently include the host runtime.

The parser still produces a syntax AST. `ParserContext` supports syntax decisions
that depend on registries, while `erabasic-analyzer` owns the project-level semantic
passes and produces HIR. CSV loading checks and normalizes project data, but it is
separate from executable artifact validation. The C# reference CLI can invoke Emuera's existing evaluator
and VM for oracle purposes. The Rust VM is a separate implementation; the reference
runtime operations do not imply that an equivalent Rust runtime exists.

RustyEra does not implement a concrete application frontend: no GUI, TUI, game
launcher, filesystem scanner, renderer, audio system, or input loop belongs in
this repository. Here, “application frontend” means the host/UI layer and is
distinct from the implemented EraBasic *language* front end.

## Bytecode contract

The compiler emits a deterministic, self-contained `.erbc` container. Its execution
identity covers code, project data, variable layout, semantic compiler options, and
native/Host ABI requirements; debug source locations use a separate artifact identity.
The bytecode and compiler crates perform no filesystem I/O.

VM-native instructions own data, arithmetic, control flow, EraBasic calls, yielding,
and continuation values. Every operation that crosses into an application frontend
uses the one `CallHost` opcode plus a typed, capability-tagged import. Printing text,
showing images, audio, input, clocks, storage, and extension plugins therefore do not
receive dedicated VM opcodes.

Decoded bytes are intentionally returned as `UnvalidatedArtifact`. A caller must bind
the declared native and Host imports and pass the value through `validate_bytecode`
before `erabasic-vm` may execute it. Source maps resolve function/code offsets to a
relative path, UTF-8 byte span, line, and byte column.

The intended project boundary is a runtime library plus public interfaces
between that runtime and an external frontend. The frontend owns filesystem
I/O and submits relative paths together with decoded UTF-8 content or the I/O
error it observed. The VM is already runtime-independent; the higher-level runtime
and its frontend event contract remain to be implemented. No specific frontend
architecture is prescribed here.

## Library use

Parse ERH and ERB source with one persistent parser context:

```rust
use erabasic_parser::{DefaultParserContext, parse_erb, parse_erh};

let mut context = DefaultParserContext::default();
let header = parse_erh("#DEFINE TEN 10\n", &mut context);
assert!(!header.has_errors());

let script = parse_erb("@TEST\nRESULT = TEN + 1\n", &mut context);
assert!(!script.has_errors());
println!("{:#?}", script.value);
```

Applications with their own instruction, variable, function, or configuration
registries can implement `ParserContext`.

Load project data without giving the CSV crate filesystem access:

```rust
use erabasic_csv::{CsvLoadOptions, FilePayload, FrontendFile, ProjectFiles, load_project};

let report = load_project(
    &ProjectFiles {
        csv: vec![FrontendFile {
            relative_path: "ABL.csv".into(),
            payload: FilePayload::Utf8("0,Strength\n".into()),
        }],
        erb: vec![],
    },
    &CsvLoadOptions::default(),
);
assert!(report.data.is_some());
```

## REPL

Run `cargo run -p erabasic-repl`. A plain line is parsed as one EraBasic logical
line. Explicit modes are also available:

```text
:lex SOURCE
:expr SOURCE
:line SOURCE
:file PATH
:analyze PATH...
:help
:quit
```

`:file` treats `.erh` files as headers and all other extensions as ERB. The REPL
keeps one parser context, so declarations and macros loaded from an ERH file are
visible to files parsed later. The REPL is a developer tool, not an application
frontend or runtime.

## Compatibility and testing

Every change must have focused Rust tests and pass the workspace checks:

```sh
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Compatibility work also uses the pinned C# implementation through the
persistent NDJSON [reference CLI](tools/emuera-reference-cli/README.md):

```powershell
# Windows
tools/emuera-reference-cli/tests/protocol-smoke.ps1
```

```sh
# macOS through the repository Wine prefix
tools/emuera-reference-cli/test-macos-wine.sh
```

A platform smoke test proves only that the oracle works. Differential tests
must also give the Rust and C# implementations the same source or fixture and
compare the relevant tokens, syntax shape, diagnostics, schema/static data,
output, state, values, and termination reason. Request IDs, absolute paths, and
other explicitly environmental metadata may be ignored.

If `emuera-reference-cli` fails to start, exits unexpectedly, or hangs, it must
be repaired instead of skipping the oracle comparison. A repair may add a
minimal headless/reference-only hook under `reference/emuera.em` when necessary,
but it must not change the normal game's backend execution semantics. Every
reference-tree modification must be listed separately in both the task handoff
and [the reference CLI change log](tools/emuera-reference-cli/REFERENCE_CHANGES.md).

Detailed contributor rules and the required test/reporting workflow are in
[AGENTS.md](AGENTS.md).

## Design notes

Emuera's lexer changes terminators according to its caller and permits nested
expressions inside FORM strings. Object-like macro expansion also modifies the
token stream. These behaviors are not a good fit for one regular lexer, so the
implementation uses an explicit UTF-8 cursor. Expressions use a Pratt parser
whose binding powers correspond to the pinned `OperatorCode.cs` behavior.

Whitespace and comments are discarded from the syntax AST, while meaningful
nodes retain UTF-8 byte spans. Diagnostics use stable English categories and
byte spans rather than reproducing localized C# messages. C# projections must
distinguish those offsets from the reference implementation's UTF-16 code-unit
positions.
