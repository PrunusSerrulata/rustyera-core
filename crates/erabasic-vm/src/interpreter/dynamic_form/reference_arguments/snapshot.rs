use super::{
    BytecodeFunctionKind, BytecodeType, ReferenceTermCall, ReferenceTermKind,
    RuntimeFormContinuation, RuntimeFormTask,
};
use erabasic_bytecode::{
    BytecodeArtifact, BytecodeGlobal, ReferenceTermArgument, ReferenceTermGraph, ReferenceTermNode,
    RuntimeImport, SymbolKey,
};
use std::collections::BTreeMap;
impl RuntimeFormContinuation {
    pub(crate) fn valid_reference_argument_symbols(
        &self,
        artifact: &erabasic_bytecode::BytecodeArtifact,
    ) -> bool {
        if !self.reference_arguments_valid() {
            return false;
        }
        let Some(pending) = &self.reference_arguments else {
            return true;
        };
        let graph = &pending.graph.template;
        let globals = artifact
            .globals
            .iter()
            .map(|variable| (variable.key, variable))
            .collect::<std::collections::BTreeMap<_, _>>();
        let functions = artifact
            .functions
            .iter()
            .map(|function| (function.key, function))
            .collect::<std::collections::BTreeMap<_, _>>();
        let imports = artifact
            .native_imports
            .iter()
            .map(|native| (native.import.key, &native.import))
            .collect::<std::collections::BTreeMap<_, _>>();
        graph.nodes.iter().all(|node| match &node.kind {
            ReferenceTermKind::Variable { key, indices } => {
                globals.get(key).is_some_and(|variable| {
                    variable.value_type == node.value_type
                        && (variable.owner.is_none() || variable.owner == Some(self.function))
                        && indices.len()
                            <= variable.dimensions.len()
                                + usize::from(
                                    variable.storage
                                        == erabasic_bytecode::BytecodeStorage::Character,
                                )
                        && artifact
                            .runtime_variables
                            .binary_search_by_key(key, |symbol| symbol.key)
                            .is_ok()
                })
            }
            ReferenceTermKind::Call {
                target: ReferenceTermCall::User { key },
                arguments,
            } => functions.get(key).is_some_and(|function| {
                function.kind == BytecodeFunctionKind::Method
                    && function.result == Some(node.value_type)
                    && arguments.iter().enumerate().all(|(index, argument)| {
                        if index >= function.parameters.len() {
                            return argument.node.is_none() && !argument.place;
                        }
                        argument.place == function.parameters[index].by_reference
                    })
            }),
            ReferenceTermKind::Call {
                target: ReferenceTermCall::Native { key, name },
                arguments,
            } => valid_static_native(&imports, graph, node, *key, name, arguments),
            ReferenceTermKind::Call {
                target: ReferenceTermCall::DynamicNative { key, name },
                arguments,
            } => valid_dynamic_native(artifact, &globals, graph, node, *key, name, arguments),
            ReferenceTermKind::Call {
                target: ReferenceTermCall::Host { key, name },
                arguments,
            } => valid_dynamic_host(artifact, &globals, graph, node, *key, name, arguments),
            ReferenceTermKind::Call {
                target: ReferenceTermCall::Intrinsic { name },
                arguments,
            } => valid_intrinsic(artifact, node, name, arguments),
            _ => true,
        }) && self.work.iter().all(|task| match task {
            RuntimeFormTask::FinishCallTextArguments { target, .. } => {
                functions.get(target).is_some_and(|function| {
                    function.kind != BytecodeFunctionKind::Method
                        && (function.kind != BytecodeFunctionKind::Event
                            || artifact.call_compatibility.allow_event_as_normal)
                })
            }
            RuntimeFormTask::CaptureReferencePlace { key, indices } => {
                globals.get(key).is_some_and(|variable| {
                    (variable.owner.is_none() || variable.owner == Some(self.function))
                        && *indices
                            <= variable.dimensions.len()
                                + usize::from(
                                    variable.storage
                                        == erabasic_bytecode::BytecodeStorage::Character,
                                )
                })
            }
            _ => true,
        })
    }
}

fn valid_static_native(
    imports: &BTreeMap<SymbolKey, &RuntimeImport>,
    graph: &ReferenceTermGraph,
    node: &ReferenceTermNode,
    key: SymbolKey,
    name: &str,
    arguments: &[ReferenceTermArgument],
) -> bool {
    imports.get(&key).is_some_and(|import| {
        import.name.eq_ignore_ascii_case(name)
            && import.result == Some(node.value_type)
            && import.parameters.len() == arguments.len()
            && import
                .parameters
                .iter()
                .zip(arguments)
                .all(|(expected, argument)| {
                    argument.node.is_some_and(|id| {
                        let actual = &graph.nodes[id as usize];
                        if matches!(
                            expected,
                            BytecodeType::IntegerPlace | BytecodeType::StringPlace
                        ) {
                            argument.place
                                && matches!(actual.kind, ReferenceTermKind::Variable { .. })
                                && actual.value_type
                                    == if *expected == BytecodeType::IntegerPlace {
                                        BytecodeType::Integer
                                    } else {
                                        BytecodeType::String
                                    }
                        } else {
                            !argument.place && actual.value_type == *expected
                        }
                    })
                })
    })
}

fn valid_dynamic_native(
    artifact: &BytecodeArtifact,
    globals: &BTreeMap<SymbolKey, &BytecodeGlobal>,
    graph: &ReferenceTermGraph,
    node: &ReferenceTermNode,
    key: SymbolKey,
    name: &str,
    arguments: &[ReferenceTermArgument],
) -> bool {
    let family = artifact
        .runtime_native_authorizations
        .iter()
        .find(|family| family.key == key && family.name.eq_ignore_ascii_case(name));
    let shapes = arguments
        .iter()
        .map(|argument| {
            argument.node.map(|id| {
                let node = &graph.nodes[id as usize];
                let variable = if let ReferenceTermKind::Variable { key, .. } = &node.kind {
                    globals.get(key).copied()
                } else {
                    None
                };
                erabasic_bytecode::RuntimeExpressionShape {
                    value_type: node.value_type,
                    variable: variable.is_some(),
                    mutable: variable.is_some_and(|variable| variable.mutable),
                }
            })
        })
        .collect::<Vec<_>>();
    family
        .and_then(|family| family.bind(&shapes))
        .is_some_and(|bound| {
            bound.import.result == Some(node.value_type)
                && bound
                    .import
                    .parameters
                    .iter()
                    .zip(arguments)
                    .all(|(parameter, argument)| {
                        argument.place
                            == matches!(
                                parameter,
                                BytecodeType::IntegerPlace | BytecodeType::StringPlace
                            )
                    })
        })
}

fn valid_intrinsic(
    artifact: &BytecodeArtifact,
    node: &ReferenceTermNode,
    name: &str,
    arguments: &[ReferenceTermArgument],
) -> bool {
    let result = match name {
        "STRFORM" | "GETMETHS" => BytecodeType::String,
        "STRFORMCHECK" | "GETMETH" | "EXISTMETH" | "EXISTVAR" => BytecodeType::Integer,
        _ => return false,
    };
    (matches!(name, "GETMETH" | "GETMETHS" | "EXISTVAR")
        || artifact
            .runtime_native_authorizations
            .iter()
            .any(|family| family.name.eq_ignore_ascii_case(name)))
        && result == node.value_type
        && arguments.iter().all(|argument| !argument.place)
        && (name != "STRFORMCHECK"
            || artifact
                .manifest
                .compatibility
                .supports_checked_runtime_forms())
}

fn valid_dynamic_host(
    artifact: &BytecodeArtifact,
    globals: &BTreeMap<SymbolKey, &BytecodeGlobal>,
    graph: &ReferenceTermGraph,
    node: &ReferenceTermNode,
    key: SymbolKey,
    name: &str,
    arguments: &[ReferenceTermArgument],
) -> bool {
    let family = artifact
        .runtime_host_authorizations
        .iter()
        .find(|family| family.key == key && family.name.eq_ignore_ascii_case(name));
    let shapes = arguments
        .iter()
        .map(|argument| {
            argument.node.map(|id| {
                let node = &graph.nodes[id as usize];
                let variable = if let ReferenceTermKind::Variable { key, .. } = &node.kind {
                    globals.get(key).copied()
                } else {
                    None
                };
                erabasic_bytecode::RuntimeExpressionShape {
                    value_type: node.value_type,
                    variable: variable.is_some(),
                    mutable: variable.is_some_and(|variable| variable.mutable),
                }
            })
        })
        .collect::<Vec<_>>();
    arguments.iter().enumerate().all(|(slot, argument)| {
        let Some((ranks, kind)) = erabasic_bytecode::host_source_place_ranks(name, slot) else {
            return true;
        };
        let Some(id) = argument.node else {
            return true;
        };
        let ReferenceTermKind::Variable { key, .. } = &graph.nodes[id as usize].kind else {
            return false;
        };
        globals.get(key).is_some_and(|variable| {
            variable.value_type == kind && ranks.contains(&variable.dimensions.len())
        })
    }) && family
        .and_then(|family| family.bind(&shapes))
        .is_some_and(|bound| {
            bound.import.result == Some(node.value_type)
                && bound
                    .import
                    .parameters
                    .iter()
                    .zip(arguments)
                    .all(|(parameter, argument)| {
                        argument.place
                            == matches!(
                                parameter,
                                BytecodeType::IntegerPlace | BytecodeType::StringPlace
                            )
                    })
        })
}
