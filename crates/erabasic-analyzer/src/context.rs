use std::collections::{BTreeMap, BTreeSet};

use erabasic_data::ProjectSchema;
use erabasic_lexer::{LexerConfig, MacroTable};
use erabasic_parser::{InstructionSpec, ParserContext};

use crate::{catalog::Catalog, options::AnalyzerOptions};

pub(crate) struct AnalysisParserContext {
    lexer: LexerConfig,
    macros: MacroTable,
    instructions: BTreeMap<String, InstructionSpec>,
    variables: BTreeSet<String>,
    functions: BTreeSet<String>,
    ignore_case: bool,
    debug_mode: bool,
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
        Self {
            lexer: LexerConfig {
                allow_full_width_space: options.allow_full_width_space,
                debug_semicolon: options.debug_semicolon,
                ..LexerConfig::default()
            },
            macros: MacroTable::new(),
            instructions: catalog
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
                .collect(),
            variables: schema
                .variables
                .keys()
                .map(|name| normalize(name))
                .collect(),
            functions: functions.into_iter().map(|name| normalize(&name)).collect(),
            ignore_case: options.ignore_case,
            debug_mode: options.debug_mode,
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
        &mut self.macros
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
        self.variables.insert(self.key(name))
    }

    fn preprocessor_symbol(&self, name: &str) -> Option<i64> {
        match self.key(name).as_str() {
            "__DEBUG__" => Some(i64::from(self.debug_mode)),
            _ => None,
        }
    }
}
