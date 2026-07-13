//! `EraBasic` expression, logical-line, ERH, and ERB parser.
//!
//! Public entry points are re-exported from focused modules. Internal modules
//! follow the data flow from context and tokens through expressions and lines
//! to complete ERH/ERB scripts.

mod context;
mod expression;
mod formatted;
mod line;
mod preprocessor;
mod script;
mod util;

pub use context::{ArgumentStyle, DefaultParserContext, InstructionSpec, ParserContext};
pub use expression::parse_expression;
pub use line::parse_line;
pub use script::{parse_erb, parse_erh};

#[cfg(test)]
mod tests;
