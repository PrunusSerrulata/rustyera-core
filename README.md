# RustyEra parser

RustyEra is a UTF-8 Rust implementation of the EraBasic syntax used by
Emuera.EM. Compatibility is pinned to reference commit
`26a35dc9334bb67590b96f7b8efbefbf199e391e` (Emuera 1.824 family).

The workspace deliberately separates the public layers:

- `erabasic-ast` contains source spans, diagnostics, and the semantic AST.
- `erabasic-lexer` implements context-sensitive EraBasic tokenization and FORM
  string decomposition.
- `erabasic-parser` parses expressions, logical lines, ERH declarations, ERB
  functions, preprocessors, and block structure.
- `erabasic-repl` builds the `rustyera` demonstration binary.

Only UTF-8 input is supported. The crates do not attempt to detect or decode
Shift-JIS, GBK, or other legacy encodings.

## Library use

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
registries can implement `ParserContext`. This reflects Emuera's original
separation between syntax parsing and runtime symbol tables.

## REPL

Run `cargo run -p erabasic-repl`. A plain line is parsed as one EraBasic logical
line. The following explicit modes are also available:

```text
:lex SOURCE
:expr SOURCE
:line SOURCE
:file PATH
:help
:quit
```

`:file` treats `.erh` files as headers and all other extensions as ERB. The REPL
keeps one parser context, so declarations and macros loaded from an ERH file are
visible to files parsed later.

## Design notes

Emuera's lexer changes terminators according to its caller and permits nested
expressions inside FORM strings. Object-like macro expansion also modifies the
token stream. These behaviors are not a good fit for a single regular lexer, so
the implementation uses an explicit UTF-8 cursor. Expressions use a Pratt
parser whose binding powers correspond to `OperatorCode.cs`.

The output is a semantic AST: meaningful names and byte spans are retained,
while whitespace and comments are discarded. Diagnostics use stable English
categories and UTF-8 byte spans rather than reproducing localized C# messages.

## Verification

For differential tests against the pinned C# engine, see the Windows-only
[reference CLI](tools/emuera-reference-cli/README.md). It exposes the original
lexer, parser, evaluator, and VM through a persistent NDJSON process without
starting the normal UI.

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```
