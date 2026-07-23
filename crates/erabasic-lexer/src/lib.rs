//! Context-sensitive lexer for the Emuera.EM `EraBasic` dialect.
//!
//! `EraBasic` cannot be described as one regular token language: callers choose
//! different terminators, semicolons have debug variants, and formatted strings
//! recursively contain expressions. The implementation is split into data
//! types, stateless lexical rules, and the stateful scanner.

mod config;
mod rules;
mod scanner;
mod token;

pub use config::{LexEnd, LexFlags, LexOutput, LexerConfig, MacroTable};
pub use scanner::{lex, lex_formatted, lex_formatted_until_comma, lex_with};
pub use token::{FormattedToken, FormattedTokenPart, Operator, Token, TokenKind};
