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
            | "GETPLATFORM"
            | "ENV_HAS_CAPABILITY"
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

impl CallableSignature {
    /// Argument contract after applying the built-in's arity-specific overload.
    #[must_use]
    pub fn arguments_for_arity(&self, arity: usize) -> &[ArgumentConstraint] {
        // The two-argument form replaces a stored document by numeric or string key.
        // Only the longer inline-XML form writes back into a mutable string argument.
        if self.name == "MAP_VALUES" && arity == 1 {
            &[ArgumentConstraint::String]
        } else if self.name == "MAP_VALUES" && arity == 2 {
            &[ArgumentConstraint::String, ArgumentConstraint::Integer]
        } else if self.name == "XML_REPLACE" && arity == 2 {
            &[ArgumentConstraint::Any, ArgumentConstraint::String]
        } else {
            &self.arguments
        }
    }
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

/// Built-in signatures retained for parse-only runtime expression validation.
#[must_use]
pub fn builtin_function_signatures(
    identity: &erabasic_compat::CompatibilityIdentity,
) -> Vec<CallableSignature> {
    builtin_functions()
        .into_values()
        .filter(|signature| builtin_function_available(&signature.name, identity))
        .collect()
}

pub(crate) fn builtin_instruction_available(
    name: &str,
    identity: &erabasic_compat::CompatibilityIdentity,
) -> bool {
    match name.to_ascii_uppercase().as_str() {
        "SETANIMETIMER"
        | "BITMAP_CACHE_ENABLE"
        | "TEXT_BGC_ON"
        | "TEXT_BGC_OFF"
        | "HTML_PRINTC"
        | "HTML_PRINTLC" => identity.supports_snake_display_state(),
        _ => builtin_shared_available(name, identity),
    }
}

pub(crate) fn builtin_function_available(
    name: &str,
    identity: &erabasic_compat::CompatibilityIdentity,
) -> bool {
    match name.to_ascii_uppercase().as_str() {
        "SETANIMETIMER" | "BITMAP_CACHE_ENABLE" => !identity.is_experimental(),
        "GETANIMETIMER"
        | "SPRITECREATEFROMFILE"
        | "G_POLYGON_DRAW"
        | "G_POLYGON_FILL"
        | "G_POLYGON_POINT_ADD"
        | "G_POLYGON_POINT_CLEAR" => identity.supports_snake_display_state(),
        _ => builtin_shared_available(name, identity),
    }
}

fn builtin_shared_available(name: &str, identity: &erabasic_compat::CompatibilityIdentity) -> bool {
    match name.to_ascii_uppercase().as_str() {
        name if name.starts_with("SQL_") => identity.supports_safe_sql(),
        "CALLSTR" | "JUMPSTR" | "TRYCALLSTR" | "TRYJUMPSTR" | "TRYCCALLSTR" | "TRYCJUMPSTR" => {
            identity.supports_call_text()
        }
        "STRFORMCHECK" => identity.supports_checked_runtime_forms(),
        "TINPUTNF"
        | "TINPUTSNF"
        | "TONEINPUTNF"
        | "TONEINPUTSNF"
        | "SEQUENCEINPUT"
        | "DISABLE_INPUT_MACRO"
        | "ENABLE_INPUT_MACRO"
        | "ENV_HAS_CAPABILITY"
        | "GETPLATFORM" => identity.supports_snake_input(),
        "GETCSVNOBYNAME"
        | "GETCSVNOBYCALLNAME"
        | "GETCSVNOBYNICKNAME"
        | "GETCSVNOBYMASTERNAME"
        | "BITSET"
        | "BITGET"
        | "BITTOGGLE"
        | "BITINDEXOFFIRST"
        | "MATCHALL"
        | "MATCHALLEX" => identity.supports_snake_data_apis(),
        "MAP_VALUES" | "MAP_MERGE" | "MAP_REMOVEIF" | "MAP_FINDKEY" | "MAP_TOSTRING"
        | "MAP_FROMSTRING" => identity.supports_map_extensions(),
        _ => true,
    }
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
    use std::collections::BTreeMap;

    use super::{
        ArgumentConstraint, ArgumentStyle, Catalog, ExtensionRegistry, InstructionSignature,
        builtin_function_available, builtin_function_names, builtin_function_signatures,
        builtin_instruction_available, builtin_instruction_names,
    };
    use erabasic_compat::{CompatibilityIdentity, CompatibilityProfileId};
    use erabasic_hir::SemanticType;

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

    #[test]
    fn animation_and_display_builtins_have_profile_specific_source_forms() {
        let original = CompatibilityIdentity::reference();
        let snake = CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake);
        for name in ["SETANIMETIMER", "BITMAP_CACHE_ENABLE"] {
            assert!(builtin_function_available(name, &original));
            assert!(!builtin_instruction_available(name, &original));
            assert!(!builtin_function_available(name, &snake));
            assert!(builtin_instruction_available(name, &snake));
        }
        assert!(!builtin_function_available("GETANIMETIMER", &original));
        assert!(builtin_function_available("GETANIMETIMER", &snake));
        for name in ["TEXT_BGC_ON", "TEXT_BGC_OFF", "HTML_PRINTC", "HTML_PRINTLC"] {
            assert!(!builtin_instruction_available(name, &original));
            assert!(builtin_instruction_available(name, &snake));
        }
    }

    #[test]
    fn safe_sql_catalog_is_snake_only_and_preserves_parameter_physical_arity() {
        let reference = CompatibilityIdentity::reference();
        let snake = CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake);
        let supported = [
            "SQL_CONNECT",
            "SQL_DISCONNECT",
            "SQL_EXECUTE_NONQUERY",
            "SQL_P_EXECUTE_NONQUERY",
            "SQL_EXECUTE_SCALAR_LONG",
            "SQL_EXECUTE_SCALAR_STRING",
            "SQL_P_EXECUTE_SCALAR_LONG",
            "SQL_P_EXECUTE_SCALAR_STRING",
            "SQL_EXECUTE_READER",
            "SQL_P_EXECUTE_READER",
            "SQL_READER_READ",
            "SQL_READER_GET_LONG",
            "SQL_READER_GET_STRING",
            "SQL_READER_ISNULL",
            "SQL_READER_CLOSE",
            "SQL_IMPORT_MAP_XML",
        ];
        for name in supported {
            assert!(!builtin_function_available(name, &reference));
            assert!(builtin_function_available(name, &snake));
        }

        let signatures = builtin_function_signatures(&snake)
            .into_iter()
            .map(|signature| (signature.name.clone(), signature))
            .collect::<BTreeMap<_, _>>();
        let connect = &signatures["SQL_CONNECT"];
        assert_eq!(connect.minimum_arguments, 1);
        assert_eq!(connect.arguments, [ArgumentConstraint::String; 2]);
        assert!(!connect.variadic);
        assert!(connect.allow_omitted);

        let parameterized = &signatures["SQL_P_EXECUTE_NONQUERY"];
        assert_eq!(parameterized.minimum_arguments, 2);
        assert_eq!(parameterized.arguments, [ArgumentConstraint::String; 3]);
        assert!(parameterized.variadic);
        assert!(parameterized.allow_omitted);

        let deferred_float = &signatures["SQL_READER_GET_FLOAT"];
        assert_eq!(deferred_float.return_type, SemanticType::Error);
    }
}
