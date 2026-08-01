use std::collections::{BTreeMap, BTreeSet};

use erabasic_hir::SemanticType;
use erabasic_parser::ArgumentStyle;
use serde::{Deserialize, Serialize};

mod functions;
mod instructions;

use functions::builtin_functions;
use instructions::builtin_instructions;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArgumentConstraint {
    Integer,
    String,
    Any,
    MutableInteger,
    MutableString,
    MutableAny,
    ReferenceAny,
    ReferenceOrString,
    MutableReferenceOrString,
    IntegerOrReference,
    /// An integer value or a mutable string place. This models legacy APIs
    /// which use a nonzero integer to select RESULTS and a string array as an
    /// explicit output target.
    IntegerOrMutableString,
    Formatted,
    Raw,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionCallableKind {
    Instruction,
    ExpressionFunction,
}

/// Portability provenance shared by semantic analysis and bytecode lowering.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallablePortability {
    Portable,
    FrontendObservation,
    PlatformIntent,
    ExtensionDefined,
}

/// Classify built-ins whose result depends on the authoritative frontend.
/// Keeping this list in the language catalog prevents analyzer and compiler
/// diagnostics from silently disagreeing as new compatibility calls are added.
#[must_use]
pub fn builtin_callable_portability(name: &str) -> CallablePortability {
    if matches!(
        name.to_ascii_uppercase().as_str(),
        "CHKFONT"
            | "GETTEXTBOX"
            | "MOUSEX"
            | "MOUSEY"
            | "MOUSEB"
            | "GETKEY"
            | "GETKEYTRIGGERED"
            | "CLIENTWIDTH"
            | "CLIENTHEIGHT"
            | "GETLINESTR"
            | "GETDISPLAYLINE"
            | "HTML_GETPRINTEDSTR"
            | "HTML_STRINGLEN"
            | "HTML_SUBSTRING"
            | "HTML_STRINGLINES"
            | "GGETTEXTSIZE"
            | "GGETCOLOR"
    ) {
        CallablePortability::FrontendObservation
    } else {
        CallablePortability::Portable
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CallableSignature {
    pub name: String,
    pub return_type: SemanticType,
    pub arguments: Vec<ArgumentConstraint>,
    pub minimum_arguments: usize,
    pub variadic: bool,
    pub allow_omitted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InstructionSignature {
    pub name: String,
    pub argument_style: ArgumentStyle,
    pub arguments: Vec<ArgumentConstraint>,
    pub minimum_arguments: usize,
    pub variadic: bool,
    pub allow_omitted: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExtensionRegistry {
    pub instructions: BTreeMap<String, InstructionSignature>,
    pub functions: BTreeMap<String, CallableSignature>,
}

impl ExtensionRegistry {
    pub fn register_instruction(&mut self, mut signature: InstructionSignature) -> bool {
        let key = signature.name.to_ascii_uppercase();
        signature.name.clone_from(&key);
        self.instructions.insert(key, signature).is_none()
    }

    pub fn register_function(&mut self, mut signature: CallableSignature) -> bool {
        let key = signature.name.to_ascii_uppercase();
        signature.name.clone_from(&key);
        self.functions.insert(key, signature).is_none()
    }
}

pub(crate) struct Catalog {
    pub instructions: BTreeMap<String, InstructionSignature>,
    pub functions: BTreeMap<String, CallableSignature>,
    pub extension_instructions: BTreeSet<String>,
    pub extension_functions: BTreeSet<String>,
}

/// Return the pinned built-in instruction namespace in deterministic order.
///
/// Execution layers use this inventory to require an explicit Native/Host or
/// unsupported classification for every analyzer-visible built-in.
#[must_use]
pub fn builtin_instruction_names() -> Vec<String> {
    builtin_instructions().into_keys().collect()
}

/// Return the pinned built-in expression-function namespace in deterministic order.
#[must_use]
pub fn builtin_function_names() -> Vec<String> {
    builtin_functions().into_keys().collect()
}

impl Catalog {
    pub fn build(extensions: &ExtensionRegistry) -> Self {
        let mut catalog = Self {
            instructions: builtin_instructions(),
            functions: builtin_functions(),
            extension_instructions: extensions
                .instructions
                .keys()
                .map(|name| name.to_ascii_uppercase())
                .collect(),
            extension_functions: extensions
                .functions
                .keys()
                .map(|name| name.to_ascii_uppercase())
                .collect(),
        };
        // Emuera plugins are registered after built-ins. They may add names but may
        // not silently replace the core instruction identity used by the loader.
        for (name, signature) in &extensions.instructions {
            catalog
                .instructions
                .entry(name.to_ascii_uppercase())
                .or_insert_with(|| signature.clone());
        }
        for (name, signature) in &extensions.functions {
            catalog
                .functions
                .entry(name.to_ascii_uppercase())
                .or_insert_with(|| signature.clone());
        }
        catalog
    }
}

pub(super) fn instruction(
    name: &str,
    style: ArgumentStyle,
    arguments: &[ArgumentConstraint],
    minimum_arguments: usize,
    variadic: bool,
    allow_omitted: bool,
) -> InstructionSignature {
    InstructionSignature {
        name: name.to_owned(),
        argument_style: style,
        arguments: arguments.to_vec(),
        minimum_arguments,
        variadic,
        allow_omitted,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ArgumentConstraint, ArgumentStyle, Catalog, ExtensionRegistry, InstructionSignature,
        builtin_function_names, builtin_instruction_names,
    };

    #[test]
    fn builtin_inventories_are_sorted_and_extensions_do_not_replace_them() {
        let instructions = builtin_instruction_names();
        let functions = builtin_function_names();
        assert!(instructions.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(functions.windows(2).all(|pair| pair[0] < pair[1]));

        let mut extensions = ExtensionRegistry::default();
        assert!(extensions.register_instruction(InstructionSignature {
            name: "PRINT".into(),
            argument_style: ArgumentStyle::None,
            arguments: vec![ArgumentConstraint::Integer],
            minimum_arguments: 1,
            variadic: false,
            allow_omitted: false,
        }));
        let catalog = Catalog::build(&extensions);
        let print = catalog.instructions.get("PRINT").expect("PRINT built-in");
        assert_ne!(print.argument_style, ArgumentStyle::None);
    }
}
