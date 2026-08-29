//! Serialize catalog signatures, not only the Native/Host imports reached by codegen.
use erabasic_analyzer::{ArgumentConstraint as A, CallableSignature};
use erabasic_bytecode::{
    RuntimeArgumentConstraint as B, RuntimeBuiltinSymbol, RuntimeCallableShape,
};

pub(super) fn runtime_builtin_symbols(
    signatures: impl IntoIterator<Item = CallableSignature>,
) -> Vec<RuntimeBuiltinSymbol> {
    let mut symbols = signatures
        .into_iter()
        .filter_map(|signature| {
            let result = crate::lowering::bytecode_type(signature.return_type)?;
            let mut shapes = Vec::new();
            // Materialize catalog arity-specific contracts (e.g. XML_REPLACE's short form)
            // here, so the VM never recognizes builtin names with its own rule table.
            for arity in signature.minimum_arguments..=signature.arguments.len() {
                shapes.push(RuntimeCallableShape {
                    minimum: arity,
                    maximum: Some(arity),
                    omitted_from: signature.minimum_arguments,
                    arguments: signature
                        .arguments_for_arity(arity)
                        .iter()
                        .copied()
                        .map(constraint)
                        .collect(),
                    allow_omitted: signature.allow_omitted,
                });
            }
            if signature.variadic {
                shapes.push(RuntimeCallableShape {
                    minimum: signature.arguments.len().max(signature.minimum_arguments),
                    maximum: None,
                    omitted_from: signature.minimum_arguments,
                    arguments: signature
                        .arguments
                        .iter()
                        .copied()
                        .map(constraint)
                        .collect(),
                    allow_omitted: signature.allow_omitted,
                });
            }
            Some(RuntimeBuiltinSymbol {
                name: signature.name,
                result,
                shapes,
            })
        })
        .collect::<Vec<_>>();
    symbols.sort_by(|left, right| left.name.cmp(&right.name));
    symbols
}
fn constraint(value: A) -> B {
    match value {
        A::Integer => B::Integer,
        A::String => B::String,
        A::Any => B::Any,
        A::MutableInteger => B::MutableInteger,
        A::MutableString => B::MutableString,
        A::MutableAny => B::MutableAny,
        A::ReferenceAny => B::ReferenceAny,
        A::ReferenceOrString => B::ReferenceOrString,
        A::MutableReferenceOrString => B::MutableReferenceOrString,
        A::IntegerOrReference => B::IntegerOrReference,
        A::IntegerOrMutableString => B::IntegerOrMutableString,
        A::Formatted => B::Formatted,
        A::Raw => B::Raw,
    }
}

pub(super) fn runtime_variable_symbols(
    variables: &[erabasic_hir::Variable],
    keys: &super::DenseIdIndex<erabasic_bytecode::SymbolKey>,
) -> Vec<erabasic_bytecode::RuntimeVariableSymbol> {
    let mut symbols = variables
        .iter()
        .map(|variable| erabasic_bytecode::RuntimeVariableSymbol {
            key: *keys.get(variable.id.0).expect("validated variable key"),
            reference: variable.reference,
            match_name_rejection: variable.match_name_rejection.map(|kind| match kind {
                erabasic_hir::MatchNameRejectionKind::Script => {
                    erabasic_bytecode::MatchNameRejectionKind::Script
                }
                erabasic_hir::MatchNameRejectionKind::Internal => {
                    erabasic_bytecode::MatchNameRejectionKind::Internal
                }
            }),
            character_disposal: match variable.character_disposal {
                erabasic_hir::CharacterArrayDisposal::Preserve => {
                    erabasic_bytecode::CharacterArrayDisposal::Preserve
                }
                erabasic_hir::CharacterArrayDisposal::ClearSparse => {
                    erabasic_bytecode::CharacterArrayDisposal::ClearSparse
                }
            },
            reference_semantics: erabasic_bytecode::RuntimeReferenceSemantics {
                is_const: variable.reference_semantics.is_const,
                can_restructure: variable.reference_semantics.can_restructure,
            },
        })
        .collect::<Vec<_>>();
    symbols.sort_unstable_by_key(|symbol| symbol.key);
    symbols
}

/// `HostRegistry` is the execution authority; parse-only catalog membership is insufficient.
pub(super) fn runtime_native_authorizations(
    symbols: &[RuntimeBuiltinSymbol],
    registry: &crate::HostRegistry,
) -> Vec<erabasic_bytecode::RuntimeNativeAuthorization> {
    let mut families = symbols
        .iter()
        .filter_map(|symbol| {
            if symbol.name.starts_with("__") || symbol.name.starts_with("DT__COLUMN_") {
                return None;
            }
            let crate::ExecutionBinding::Native(contract) =
                registry.classification(&symbol.name)?
            else {
                return None;
            };
            Some(erabasic_bytecode::RuntimeNativeAuthorization::new(
                symbol, *contract,
            ))
        })
        .collect::<Vec<_>>();
    families.sort_unstable_by_key(|family| family.key);
    families
}

/// Staged VM operations require their own trusted registry classification.
pub(super) fn runtime_staged_authorizations(
    symbols: &[RuntimeBuiltinSymbol],
    registry: &crate::HostRegistry,
) -> Vec<erabasic_bytecode::RuntimeStagedAuthorization> {
    use erabasic_bytecode::{RuntimeStagedAuthorization, RuntimeStagedKind};
    let mut values = symbols
        .iter()
        .filter_map(|symbol| {
            let kind = RuntimeStagedKind::from_name(&symbol.name)?;
            let matches = matches!(
                (kind, registry.classification(&symbol.name)),
                (
                    RuntimeStagedKind::Bit(_),
                    Some(crate::ExecutionBinding::BitArray)
                ) | (
                    RuntimeStagedKind::MatchAll | RuntimeStagedKind::MatchAllEx,
                    Some(crate::ExecutionBinding::ArrayMatch)
                )
            );
            matches.then(|| RuntimeStagedAuthorization::new(symbol, kind))
        })
        .collect::<Vec<_>>();
    values.sort_unstable_by_key(|value| value.key);
    values
}

/// Reached imports do not determine dynamic callable availability. Source registration
/// and the caller-owned `HostRegistry` both have to grant this exact Host family.
pub(super) fn runtime_host_authorizations(
    symbols: &[RuntimeBuiltinSymbol],
    registry: &crate::HostRegistry,
    compatibility: &erabasic_compat::CompatibilityIdentity,
) -> Vec<erabasic_bytecode::RuntimeHostAuthorization> {
    use erabasic_bytecode::{
        BytecodeType as T, RuntimeHostAuthorization, RuntimeHostLowering as L,
        RuntimeHostStage as S,
    };
    let snake = compatibility.profile == erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake;
    let mut families = symbols
        .iter()
        .filter_map(|symbol| {
            let crate::ExecutionBinding::Host(binding) = registry.classification(&symbol.name)?
            else {
                return None;
            };
            let shapes = erabasic_bytecode::canonical_host_source_shapes(&symbol.name, snake)?;
            let prototype = host_import(binding, Vec::new(), symbol.result);
            let (lowering, steps) = match symbol.name.to_ascii_uppercase().as_str() {
                "HTML_STRINGLEN" => (
                    L::HtmlLength,
                    vec![
                        (
                            S::MeasureLength,
                            "HTML__MEASURE_LENGTH",
                            vec![T::String],
                            T::Integer,
                        ),
                        (
                            S::LengthUnit,
                            "HTML__LENGTH_UNIT",
                            vec![T::Integer, T::Integer],
                            T::Integer,
                        ),
                    ],
                ),
                "HTML_STRINGLINES" => (
                    L::HtmlLines,
                    vec![
                        (
                            S::LinesBegin,
                            "HTML__LINES_BEGIN",
                            vec![T::String],
                            T::String,
                        ),
                        (
                            S::LinesMore,
                            "HTML__LINES_MORE",
                            vec![T::String],
                            T::Integer,
                        ),
                        (
                            S::LinesStep,
                            "HTML__LINES_STEP",
                            vec![T::String, T::Integer],
                            T::Integer,
                        ),
                        (S::LinesEnd, "HTML__LINES_END", vec![T::String], T::Integer),
                    ],
                ),
                _ => (L::Eager, Vec::new()),
            };
            let stages = steps
                .into_iter()
                .map(|(stage, name, parameters, result)| {
                    (
                        stage,
                        host_import(
                            &crate::registry::html_query_binding(name),
                            parameters,
                            result,
                        ),
                    )
                })
                .collect();
            Some(RuntimeHostAuthorization::new(
                symbol, shapes, prototype, lowering, stages,
            ))
        })
        .collect::<Vec<_>>();
    families.sort_unstable_by_key(|family| family.key);
    families
}
fn host_import(
    binding: &crate::HostBinding,
    parameters: Vec<erabasic_bytecode::BytecodeType>,
    result: erabasic_bytecode::BytecodeType,
) -> erabasic_bytecode::HostImport {
    let prototype = erabasic_bytecode::RuntimeImport {
        key: erabasic_bytecode::SymbolKey::default(),
        namespace: binding.namespace.clone(),
        name: binding.name.clone(),
        abi_version: binding.abi_version,
        parameters: Vec::new(),
        result: None,
    };
    erabasic_bytecode::HostImport {
        import: erabasic_bytecode::runtime_host_import(&prototype, parameters, Some(result)),
        effect: binding.effect,
        capability: binding.capability,
        snapshot_capability: binding.snapshot_capability,
        contract: binding.contract,
    }
}
