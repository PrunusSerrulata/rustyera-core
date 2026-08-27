use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
};

use erabasic_lexer::{LexerConfig, MacroTable};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArgumentStyle {
    None,
    Expressions,
    Formatted,
    Raw,
    /// PRINTV family: apostrophe starts a raw string operand ending at comma.
    PrintV,
    /// `TIMES <place>, <real literal>` has a dedicated non-integer second operand.
    Times,
    /// Formatted function name followed by an optional parenthesized argument list.
    DynamicCall,
    /// One FORM operand ending at a comma, followed by ordinary expression operands.
    FormattedFirst,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstructionSpec {
    pub argument_style: ArgumentStyle,
}

/// Parser-side symbol services needed for context-dependent syntax decisions.
///
/// This registry interface is not a semantic-analysis pass.
pub trait ParserContext {
    /// The same resolved identity used by analysis and execution.
    fn compatibility(&self) -> erabasic_compat::CompatibilityIdentity {
        erabasic_compat::CompatibilityIdentity::reference()
    }
    fn lexer_config(&self) -> &LexerConfig;
    fn macros(&self) -> &MacroTable;
    fn macros_mut(&mut self) -> &mut MacroTable;
    fn instruction(&self, name: &str) -> Option<InstructionSpec>;
    fn is_function(&self, _name: &str) -> bool {
        false
    }
    fn is_variable(&self, _name: &str) -> bool {
        false
    }
    fn register_variable(&mut self, _name: &str) -> bool {
        true
    }
    fn preprocessor_symbol(&self, _name: &str) -> Option<i64> {
        None
    }
    /// Text inserted between physical lines inside Emuera's `{ ... }` continuation.
    ///
    /// The pinned default comes from `ReplaceContinuationBR`. Runtime project loading
    /// overrides it when a game supplies a different portable configuration value.
    #[allow(clippy::unnecessary_literal_bound)]
    fn continuation_separator(&self) -> &str {
        " "
    }
}

/// Compatibility-oriented defaults. Unknown instructions remain parseable,
/// because plugins can add instructions after Emuera starts.
#[derive(Clone, Debug)]
pub struct DefaultParserContext {
    compatibility: erabasic_compat::CompatibilityIdentity,
    lexer: LexerConfig,
    macros: MacroTable,
    variables: HashSet<String>,
    functions: HashSet<String>,
    symbols: HashMap<String, i64>,
}

impl Default for DefaultParserContext {
    fn default() -> Self {
        let variables = [
            "TARGET", "MASTER", "PLAYER", "ASSI", "RESULT", "RESULTS", "LOCAL", "LOCALS", "ARG",
            "ARGS",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        Self {
            compatibility: erabasic_compat::CompatibilityIdentity::reference(),
            lexer: LexerConfig::default(),
            macros: MacroTable::new(),
            variables,
            functions: HashSet::new(),
            symbols: HashMap::new(),
        }
    }
}

impl DefaultParserContext {
    /// Bind late-bound parsing to its artifact identity.
    pub fn set_compatibility(&mut self, compatibility: erabasic_compat::CompatibilityIdentity) {
        self.compatibility = compatibility;
    }

    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn define_preprocessor_symbol(&mut self, name: impl Into<String>, value: i64) {
        self.symbols.insert(name.into().to_uppercase(), value);
    }

    pub fn register_function(&mut self, name: impl Into<String>) {
        self.functions.insert(name.into().to_uppercase());
    }

    /// Apply the project lexer switches needed when parsing late-bound FORM text.
    pub fn set_lexer_compatibility(
        &mut self,
        allow_full_width_space: bool,
        debug_semicolon: bool,
        ignore_triple_symbols: bool,
    ) {
        self.lexer.allow_full_width_space = allow_full_width_space;
        self.lexer.debug_semicolon = debug_semicolon;
        self.lexer.ignore_triple_symbols = ignore_triple_symbols;
    }
}

impl ParserContext for DefaultParserContext {
    fn compatibility(&self) -> erabasic_compat::CompatibilityIdentity {
        self.compatibility.clone()
    }

    fn lexer_config(&self) -> &LexerConfig {
        &self.lexer
    }
    fn macros(&self) -> &MacroTable {
        &self.macros
    }
    fn macros_mut(&mut self) -> &mut MacroTable {
        &mut self.macros
    }

    fn instruction(&self, name: &str) -> Option<InstructionSpec> {
        let upper = uppercase_name(name);
        let style = if DYNAMIC_CALL_INSTRUCTIONS.contains(&upper.as_ref()) {
            ArgumentStyle::DynamicCall
        } else if NO_ARG_INSTRUCTIONS.contains(&upper.as_ref()) {
            ArgumentStyle::None
        } else if matches!(
            upper.as_ref(),
            "INPUTS" | "ONEINPUTS" | "BINPUTS" | "ONEBINPUTS"
        ) {
            ArgumentStyle::FormattedFirst
        } else if is_formatted_instruction(&upper) {
            ArgumentStyle::Formatted
        } else if matches!(upper.as_ref(), "PRINTV" | "PRINTVL" | "PRINTVW") {
            ArgumentStyle::PrintV
        } else if upper == "TIMES" {
            ArgumentStyle::Times
        } else if matches!(upper.as_ref(), "CASE" | "ALIGNMENT") || is_raw_print_instruction(&upper)
        {
            // CASE owns the reference-only `IS <expr>` and `<expr> TO <expr>`
            // selector grammar. Preserve it losslessly for semantic lowering.
            ArgumentStyle::Raw
        } else {
            ArgumentStyle::Expressions
        };
        Some(InstructionSpec {
            argument_style: style,
        })
    }

    fn is_function(&self, name: &str) -> bool {
        self.functions.contains(uppercase_name(name).as_ref())
    }
    fn is_variable(&self, name: &str) -> bool {
        self.variables.contains(uppercase_name(name).as_ref())
    }
    fn register_variable(&mut self, name: &str) -> bool {
        self.variables.insert(name.to_uppercase())
    }
    fn preprocessor_symbol(&self, name: &str) -> Option<i64> {
        self.symbols.get(uppercase_name(name).as_ref()).copied()
    }
}

fn uppercase_name(name: &str) -> Cow<'_, str> {
    if name.is_ascii() && !name.bytes().any(|byte| byte.is_ascii_lowercase()) {
        Cow::Borrowed(name)
    } else {
        Cow::Owned(name.to_uppercase())
    }
}

fn is_formatted_instruction(name: &str) -> bool {
    (name.contains("FORM") && !name.contains("FORMS"))
        || matches!(
            name,
            "PUTFORM" | "DRAWLINEFORM" | "RETURNFORM" | "REUSELASTLINE" | "THROW"
        )
}

fn is_raw_print_instruction(name: &str) -> bool {
    (name.starts_with("PRINT")
        && !name.starts_with("PRINTS")
        && !name.starts_with("PRINTV")
        && !name.contains("FORMS"))
        || matches!(
            name,
            "PRINTSINGLE" | "PRINTSINGLEK" | "PRINTSINGLED" | "DATA"
        )
}

const DYNAMIC_CALL_INSTRUCTIONS: &[&str] = &[
    "CALL",
    "CALLF",
    "JUMP",
    "BEGIN",
    "TRYCALL",
    "TRYJUMP",
    "GOTO",
    "TRYGOTO",
    "CALLFORM",
    "CALLFORMF",
    "JUMPFORM",
    "TRYCALLFORM",
    "TRYCALLFORMF",
    "TRYJUMPFORM",
    "TRYCCALLFORM",
    "TRYCCALL",
    "TRYCJUMP",
    "TRYCJUMPFORM",
    "TRYCGOTO",
    "TRYCGOTOFORM",
    "FUNC",
];

const NO_ARG_INSTRUCTIONS: &[&str] = &[
    "ELSE",
    "ENDIF",
    "NEXT",
    "REND",
    "WEND",
    "LOOP",
    "ENDSELECT",
    "ENDCATCH",
    "ENDFUNC",
    "ENDLIST",
    "DATALIST",
    "BREAK",
    "CONTINUE",
    "RESTART",
    "QUIT",
    "RETURN",
];
