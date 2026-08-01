use std::{collections::BTreeMap, io::Write};

use erabasic_bytecode::{
    BytecodeConstant, BytecodeEventEntry, BytecodeEventGroup, BytecodeGlobal, BytecodePersistence,
    BytecodeStorage, BytecodeType, Digest, SymbolKey,
};
use erabasic_data::{Persistence, StorageScope};
use erabasic_hir::{
    ConstantValue, Function, FunctionId, FunctionKind, SemanticType, Variable, VariableId,
    VariableScope,
};
use serde::Serialize;

use crate::lowering::bytecode_type;

pub(super) fn canonical_digest<T: Serialize + ?Sized>(domain: &str, value: &T) -> Digest {
    let mut writer = DigestWriter {
        hasher: blake3::Hasher::new_derive_key(domain),
    };
    serde_json::to_writer(&mut writer, value).expect("compiler identity values are serializable");
    Digest(*writer.hasher.finalize().as_bytes())
}

struct DigestWriter {
    hasher: blake3::Hasher,
}

impl Write for DigestWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.hasher.update(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(super) fn event_groups(
    functions: &[Function],
    keys: &BTreeMap<FunctionId, SymbolKey>,
) -> Vec<BytecodeEventGroup> {
    let mut groups: BTreeMap<String, Vec<&Function>> = BTreeMap::new();
    for function in functions
        .iter()
        .filter(|function| function.kind == FunctionKind::Event)
    {
        groups
            .entry(function.name.to_ascii_uppercase())
            .or_default()
            .push(function);
    }
    groups
        .into_iter()
        .map(|(name, mut members)| {
            members.sort_by_key(|function| function.definition_order);
            let mut group = BytecodeEventGroup {
                name,
                only: Vec::new(),
                priority: Vec::new(),
                normal: Vec::new(),
                later: Vec::new(),
            };
            for function in members {
                let Some(function_key) = keys.get(&function.id).copied() else {
                    continue;
                };
                let entry = BytecodeEventEntry {
                    function: function_key,
                    single: function.event_attributes.single,
                };
                if function.event_attributes.only {
                    group.only.push(entry);
                }
                if function.event_attributes.priority {
                    group.priority.push(entry);
                }
                if function.event_attributes.later {
                    group.later.push(entry);
                }
                if !function.event_attributes.priority && !function.event_attributes.later {
                    group.normal.push(entry);
                }
            }
            group
        })
        .collect()
}

pub(super) fn function_keys(
    functions: &[Function],
    sources: &[erabasic_hir::SourceFile],
) -> BTreeMap<FunctionId, SymbolKey> {
    let paths: BTreeMap<_, _> = sources
        .iter()
        .map(|source| (source.id, source.relative_path.as_str()))
        .collect();
    let mut ordinals = BTreeMap::new();
    functions
        .iter()
        .map(|function| {
            let identity = (
                paths
                    .get(&function.location.source)
                    .copied()
                    .unwrap_or_default(),
                function.name.to_ascii_uppercase(),
                function_kind_tag(function.kind),
                function
                    .parameters
                    .iter()
                    .map(|parameter| semantic_type_tag(parameter.target.value_type))
                    .collect::<Vec<_>>(),
            );
            let ordinal = ordinals.entry(identity.clone()).or_insert(0u32);
            let bytes = serde_json::to_vec(&(identity, *ordinal))
                .expect("function identity is serializable");
            *ordinal += 1;
            (
                function.id,
                SymbolKey::derive("rustyera.bytecode.function.v1", &bytes),
            )
        })
        .collect()
}

pub(super) fn variable_keys(
    variables: &[Variable],
    functions: &BTreeMap<FunctionId, SymbolKey>,
) -> BTreeMap<VariableId, SymbolKey> {
    variables
        .iter()
        .map(|variable| {
            let owner = variable
                .owner
                .and_then(|owner| functions.get(&owner).copied());
            let identity =
                serde_json::to_vec(&(variable.name.to_ascii_uppercase(), variable.scope, owner))
                    .expect("variable identity is serializable");
            (
                variable.id,
                SymbolKey::derive("rustyera.bytecode.variable.v2", &identity),
            )
        })
        .collect()
}

pub(super) fn globals(
    variables: &[Variable],
    keys: &BTreeMap<VariableId, SymbolKey>,
    functions: &BTreeMap<FunctionId, SymbolKey>,
) -> Vec<BytecodeGlobal> {
    variables
        .iter()
        .filter_map(|variable| {
            Some(BytecodeGlobal {
                key: keys[&variable.id],
                name: variable.name.clone(),
                value_type: bytecode_type(variable.value_type)?,
                dimensions: variable
                    .dimensions
                    .iter()
                    .map(|dimension| *dimension as u64)
                    .collect(),
                mutable: variable.mutable,
                storage: variable_storage(variable),
                persistence: persistence(variable.persistence),
                initial_values: variable
                    .initial_values
                    .iter()
                    .map(|value| match value {
                        ConstantValue::Integer(value) => BytecodeConstant::Integer(*value),
                        ConstantValue::String(value) => BytecodeConstant::String(value.clone()),
                    })
                    .collect(),
                owner: variable
                    .owner
                    .and_then(|owner| functions.get(&owner).copied()),
            })
        })
        .collect()
}

fn variable_storage(variable: &Variable) -> BytecodeStorage {
    if matches!(
        variable.scope,
        VariableScope::EraFunction | VariableScope::Function | VariableScope::Parameter
    ) {
        if variable.scope == VariableScope::EraFunction {
            return BytecodeStorage::FunctionPersistent;
        }
        return if variable.static_lifetime {
            BytecodeStorage::FunctionStatic
        } else {
            BytecodeStorage::FunctionLocal
        };
    }
    match variable.storage {
        StorageScope::Normal | StorageScope::Global | StorageScope::Local => {
            BytecodeStorage::Project
        }
        StorageScope::Character => BytecodeStorage::Character,
        StorageScope::Constant => BytecodeStorage::Constant,
        StorageScope::Calculated => BytecodeStorage::Calculated,
    }
}

const fn persistence(value: Persistence) -> BytecodePersistence {
    match value {
        Persistence::None => BytecodePersistence::None,
        Persistence::GameSave => BytecodePersistence::GameSave,
        Persistence::GlobalSave => BytecodePersistence::GlobalSave,
        Persistence::ExtendedSave => BytecodePersistence::ExtendedSave,
    }
}

const fn function_kind_tag(kind: FunctionKind) -> u8 {
    match kind {
        FunctionKind::Normal => 0,
        FunctionKind::Event => 1,
        FunctionKind::System => 2,
        FunctionKind::Method => 3,
    }
}

const fn semantic_type_tag(value_type: SemanticType) -> u8 {
    match value_type {
        SemanticType::Integer => 0,
        SemanticType::String => 1,
        SemanticType::Void => 2,
        SemanticType::Error => 3,
    }
}

#[allow(dead_code)]
const fn type_for_semantic(value_type: SemanticType) -> Option<BytecodeType> {
    match value_type {
        SemanticType::Integer => Some(BytecodeType::Integer),
        SemanticType::String => Some(BytecodeType::String),
        SemanticType::Void | SemanticType::Error => None,
    }
}
