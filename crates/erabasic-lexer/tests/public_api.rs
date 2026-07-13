use erabasic_lexer::{LexerConfig, TokenKind, lex};

#[test]
fn crate_root_keeps_the_lexer_api() {
    let output = lex("VALUE + 1", &LexerConfig::default());
    assert!(output.diagnostics.is_empty());
    assert!(matches!(output.tokens[0].kind, TokenKind::Identifier(_)));
}
