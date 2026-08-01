use era_runtime_protocol::{ProtocolDiagnostic, RuntimeLogLevel};
use erabasic_analyzer::{
    ArgumentConstraint, CallableSignature, ExtensionRegistry, InstructionSignature,
    builtin_function_names, builtin_instruction_names,
};
use erabasic_compiler::{default_host_registry, extension_binding};
use erabasic_hir::SemanticType;
use erabasic_parser::ArgumentStyle;

use super::project_diagnostic;

pub(super) fn category_relative_path(path: &str, category: &str) -> String {
    let Some((first, remaining)) = path.split_once('/') else {
        return path.to_owned();
    };
    if first.eq_ignore_ascii_case(category) && !remaining.is_empty() {
        remaining.to_owned()
    } else {
        path.to_owned()
    }
}

pub(super) fn is_deferred_index_source(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("erd"))
}

#[allow(clippy::too_many_lines)]
pub(super) fn prepare_extensions(
    declarations: &[era_runtime_protocol::ExtensionDeclaration],
    diagnostics: &mut Vec<ProtocolDiagnostic>,
) -> (
    ExtensionRegistry,
    erabasic_compiler::HostRegistry,
    std::collections::BTreeMap<String, era_runtime_protocol::ExtensionDeclaration>,
) {
    use era_runtime_protocol::{ExtensionArgumentStyle, ExtensionCallableKind, ExtensionValueType};
    let builtins = builtin_instruction_names()
        .into_iter()
        .chain(builtin_function_names())
        .collect::<std::collections::BTreeSet<_>>();
    let mut analyzer = ExtensionRegistry::default();
    let mut hosts = default_host_registry();
    let mut map = std::collections::BTreeMap::new();
    let mut ids = std::collections::BTreeSet::new();
    for declaration in declarations {
        let name = declaration.era_name.to_ascii_uppercase();
        let operation = declaration.operation.to_ascii_lowercase();
        let invalid = declaration.id.is_empty()
            || name.is_empty()
            || operation.is_empty()
            || builtins.contains(&name)
            || map.contains_key(&operation)
            || !ids.insert(declaration.id.clone())
            || declaration
                .arguments
                .iter()
                .any(|argument| argument.value_type == ExtensionValueType::Void)
            || declaration
                .arguments
                .windows(2)
                .any(|pair| pair[0].optional && !pair[1].optional)
            || declaration.variadic && declaration.arguments.is_empty()
            || matches!(
                (declaration.kind, declaration.return_type),
                (
                    ExtensionCallableKind::Instruction,
                    ExtensionValueType::Integer
                        | ExtensionValueType::String
                        | ExtensionValueType::Any
                ) | (
                    ExtensionCallableKind::Function,
                    ExtensionValueType::Void | ExtensionValueType::Any
                )
            )
            || declaration.kind == ExtensionCallableKind::Function
                && declaration.argument_style != ExtensionArgumentStyle::Normal;
        if invalid {
            diagnostics.push(project_diagnostic(
                "runtime.invalid_extension_declaration",
                RuntimeLogLevel::Error,
                format!(
                    "extension declaration {:?} is empty, duplicated, or conflicts with a built-in",
                    declaration.id
                ),
                None,
            ));
            continue;
        }
        let constraints = declaration
            .arguments
            .iter()
            .map(|argument| match (argument.mutable, argument.value_type) {
                (true, ExtensionValueType::Integer) => ArgumentConstraint::MutableInteger,
                (true, ExtensionValueType::String) => ArgumentConstraint::MutableString,
                (true, ExtensionValueType::Any | ExtensionValueType::Void) => {
                    ArgumentConstraint::MutableAny
                }
                (false, ExtensionValueType::Integer) => ArgumentConstraint::Integer,
                (false, ExtensionValueType::String) => ArgumentConstraint::String,
                (false, ExtensionValueType::Any | ExtensionValueType::Void) => {
                    ArgumentConstraint::Any
                }
            })
            .collect::<Vec<_>>();
        let minimum_arguments = declaration
            .arguments
            .iter()
            .take_while(|argument| !argument.optional)
            .count();
        let return_type = match declaration.return_type {
            ExtensionValueType::Integer => SemanticType::Integer,
            ExtensionValueType::String => SemanticType::String,
            ExtensionValueType::Void => SemanticType::Void,
            ExtensionValueType::Any => SemanticType::Error,
        };
        let registered = match declaration.kind {
            ExtensionCallableKind::Instruction => {
                analyzer.register_instruction(InstructionSignature {
                    name: name.clone(),
                    argument_style: match declaration.argument_style {
                        ExtensionArgumentStyle::Normal => ArgumentStyle::Expressions,
                        ExtensionArgumentStyle::Formatted => ArgumentStyle::Formatted,
                        ExtensionArgumentStyle::Raw => ArgumentStyle::Raw,
                    },
                    arguments: constraints,
                    minimum_arguments,
                    variadic: declaration.variadic,
                    allow_omitted: declaration
                        .arguments
                        .iter()
                        .any(|argument| argument.optional),
                })
            }
            ExtensionCallableKind::Function => analyzer.register_function(CallableSignature {
                name: name.clone(),
                return_type,
                arguments: constraints,
                minimum_arguments,
                variadic: declaration.variadic,
                allow_omitted: declaration
                    .arguments
                    .iter()
                    .any(|argument| argument.optional),
            }),
        };
        if !registered {
            diagnostics.push(project_diagnostic(
                "runtime.duplicate_extension_name",
                RuntimeLogLevel::Error,
                format!("duplicate extension callable {name}"),
                None,
            ));
            continue;
        }
        let mut binding = extension_binding(&name);
        binding.name.clone_from(&operation);
        binding.abi_version = u32::from(declaration.operation_version.major);
        hosts.register(name, binding);
        map.insert(operation, declaration.clone());
    }
    (analyzer, hosts, map)
}
