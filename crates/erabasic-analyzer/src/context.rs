use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use erabasic_data::ProjectSchema;
use erabasic_lexer::{LexerConfig, MacroTable};
use erabasic_parser::{InstructionSpec, ParserContext};

use crate::{catalog::Catalog, options::AnalyzerOptions};

#[derive(Clone)]
pub(crate) struct AnalysisParserContext {
    lexer: LexerConfig,
    macros: Arc<MacroTable>,
    instructions: Arc<BTreeMap<String, InstructionSpec>>,
    variables: Arc<BTreeSet<String>>,
    functions: Arc<BTreeSet<String>>,
    ignore_case: bool,
    debug_mode: bool,
    continuation_separator: String,
}

impl AnalysisParserContext {
    pub fn new(
        schema: &ProjectSchema,
        catalog: &Catalog,
        functions: impl IntoIterator<Item = String>,
        options: &AnalyzerOptions,
    ) -> Self {
        let normalize = |name: &str| {
            if options.ignore_case {
                name.to_ascii_uppercase()
            } else {
                name.to_owned()
            }
        };
        let mut instructions: BTreeMap<_, _> = catalog
            .instructions
            .iter()
            .map(|(name, signature)| {
                (
                    normalize(name),
                    InstructionSpec {
                        argument_style: signature.argument_style,
                    },
                )
            })
            .collect();
        for name in catalog.functions.keys() {
            instructions
                .entry(normalize(name))
                .or_insert(InstructionSpec {
                    argument_style: if name.contains("FORM") && !name.contains("FORMS") {
                        erabasic_parser::ArgumentStyle::Formatted
                    } else {
                        erabasic_parser::ArgumentStyle::Expressions
                    },
                });
        }
        Self {
            lexer: LexerConfig {
                allow_full_width_space: options.allow_full_width_space,
                debug_semicolon: options.debug_semicolon,
                ignore_triple_symbols: options.ignore_triple_symbols,
                ..LexerConfig::default()
            },
            macros: Arc::new(MacroTable::new()),
            instructions: Arc::new(instructions),
            variables: Arc::new(
                schema
                    .variables
                    .keys()
                    .map(|name| normalize(name))
                    .collect(),
            ),
            functions: Arc::new(functions.into_iter().map(|name| normalize(&name)).collect()),
            ignore_case: options.ignore_case,
            debug_mode: options.debug_mode,
            continuation_separator: options.continuation_separator.clone(),
        }
    }

    fn key(&self, name: &str) -> String {
        if self.ignore_case {
            name.to_ascii_uppercase()
        } else {
            name.to_owned()
        }
    }
}

impl ParserContext for AnalysisParserContext {
    fn lexer_config(&self) -> &LexerConfig {
        &self.lexer
    }

    fn macros(&self) -> &MacroTable {
        &self.macros
    }

    fn macros_mut(&mut self) -> &mut MacroTable {
        Arc::make_mut(&mut self.macros)
    }

    fn instruction(&self, name: &str) -> Option<InstructionSpec> {
        self.instructions.get(&self.key(name)).cloned()
    }

    fn is_function(&self, name: &str) -> bool {
        self.functions.contains(&self.key(name))
    }

    fn is_variable(&self, name: &str) -> bool {
        self.variables.contains(&self.key(name))
    }

    fn register_variable(&mut self, name: &str) -> bool {
        let key = self.key(name);
        Arc::make_mut(&mut self.variables).insert(key)
    }

    fn preprocessor_symbol(&self, name: &str) -> Option<i64> {
        match self.key(name).as_str() {
            "__DEBUG__" => Some(i64::from(self.debug_mode)),
            _ => None,
        }
    }

    fn continuation_separator(&self) -> &str {
        &self.continuation_separator
    }
}
