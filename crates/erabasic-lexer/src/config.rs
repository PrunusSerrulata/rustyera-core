use std::collections::HashMap;

use erabasic_ast::Diagnostic;

use crate::Token;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LexEnd {
    EndOfLine,
    Operator,
    Question,
    Percent,
    RightCurlyBrace,
    Comma,
    GreaterThan,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LexFlags(u8);

impl LexFlags {
    pub const NONE: Self = Self(0);
    pub const ANALYZE_PRINT_V: Self = Self(1);
    pub const ALLOW_ASSIGNMENT: Self = Self(2);
    pub const ALLOW_SINGLE_QUOTED_STRING: Self = Self(4);
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

impl std::ops::BitOr for LexFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

#[derive(Clone, Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct LexerConfig {
    pub allow_full_width_space: bool,
    pub debug_semicolon: bool,
    pub ignore_triple_symbols: bool,
    pub expand_macros: bool,
    pub max_macro_expansion_depth: usize,
}

impl Default for LexerConfig {
    fn default() -> Self {
        Self {
            allow_full_width_space: false,
            debug_semicolon: false,
            ignore_triple_symbols: false,
            expand_macros: true,
            max_macro_expansion_depth: 64,
        }
    }
}

/// Object-like macros are stored as already-tokenized replacement sequences.
pub type MacroTable = HashMap<String, Vec<Token>>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LexOutput {
    pub tokens: Vec<Token>,
    pub diagnostics: Vec<Diagnostic>,
    /// Byte offset where a context terminator stopped the lexer.
    pub consumed: usize,
}
