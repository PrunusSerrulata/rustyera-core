use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap, hash_map::Entry},
    io::Write,
};

use erabasic_bytecode::{
    BytecodeConstant, BytecodeEventEntry, BytecodeEventGroup, BytecodeGlobal, BytecodePersistence,
    BytecodeStorage, BytecodeType, Digest, SymbolKey,
};
use erabasic_data::{Persistence, StorageScope};
use erabasic_hir::{ConstantValue, Function, FunctionKind, SemanticType, Variable, VariableScope};
use serde::Serialize;

use crate::{compile::DenseIdIndex, lowering::bytecode_type};

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
    keys: &DenseIdIndex<SymbolKey>,
) -> Vec<BytecodeEventGroup> {
    let mut groups: BTreeMap<Cow<'_, str>, Vec<&Function>> = BTreeMap::new();
    for function in functions
        .iter()
        .filter(|function| function.kind == FunctionKind::Event)
    {
        groups
            .entry(ascii_uppercase(&function.name))
            .or_default()
            .push(function);
    }
    groups
        .into_iter()
        .map(|(name, mut members)| {
            members.sort_by_key(|function| function.definition_order);
            let mut group = BytecodeEventGroup {
                name: name.into_owned(),
                only: Vec::new(),
                priority: Vec::new(),
                normal: Vec::new(),
                later: Vec::new(),
            };
            for function in members {
                let Some(function_key) = keys.get(function.id.0).copied() else {
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
    progress: impl Fn(),
) -> DenseIdIndex<SymbolKey> {
    let mut paths = DenseIdIndex::new(sources.len());
    for source in sources {
        paths.insert(source.id.0, source.relative_path.as_str());
    }
    let mut ordinals = HashMap::with_capacity(functions.len());
    let mut keys = DenseIdIndex::new(functions.len());
    let mut identity_bytes = Vec::new();
    for function in functions {
        let identity = (
            paths
                .get(function.location.source.0)
                .copied()
                .unwrap_or_default(),
            ascii_uppercase(&function.name),
            function_kind_tag(function.kind),
            function
                .parameters
                .iter()
                .map(|parameter| semantic_type_tag(parameter.target.value_type))
                .collect::<Vec<_>>(),
        );
        identity_bytes.clear();
        match ordinals.entry(identity) {
            Entry::Occupied(mut entry) => {
                let ordinal = *entry.get();
                serde_json::to_writer(&mut identity_bytes, &(entry.key(), ordinal))
                    .expect("function identity is serializable");
                *entry.get_mut() += 1;
            }
            Entry::Vacant(entry) => {
                serde_json::to_writer(&mut identity_bytes, &(entry.key(), 0u32))
                    .expect("function identity is serializable");
                entry.insert(1);
            }
        }
        keys.insert(
            function.id.0,
            SymbolKey::derive("rustyera.bytecode.function.v1", &identity_bytes),
        );
        progress();
    }
    keys
}

pub(super) fn variable_keys(
    variables: &[Variable],
    functions: &DenseIdIndex<SymbolKey>,
    progress: impl Fn(),
) -> DenseIdIndex<SymbolKey> {
    let mut keys = DenseIdIndex::new(variables.len());
    let mut identity = Vec::new();
    for variable in variables {
        let owner = variable
            .owner
            .and_then(|owner| functions.get(owner.0).copied());
        identity.clear();
        serde_json::to_writer(
            &mut identity,
            &(ascii_uppercase(&variable.name), variable.scope, owner),
        )
        .expect("variable identity is serializable");
        keys.insert(
            variable.id.0,
            SymbolKey::derive("rustyera.bytecode.variable.v2", &identity),
        );
        progress();
    }
    keys
}

pub(super) fn shared_variable_dependencies(variables: &[Variable], progress: impl Fn()) -> Digest {
    let mut dependencies = Vec::with_capacity(variables.len());
    for variable in variables {
        dependencies.push(canonical_digest(
            "rustyera.compiler.shared-variable.v1",
            variable,
        ));
        progress();
    }
    canonical_digest(
        "rustyera.compiler.shared-variable-dependencies.v1",
        &dependencies,
    )
}

pub(super) fn globals(
    variables: &[Variable],
    keys: &DenseIdIndex<SymbolKey>,
    functions: &DenseIdIndex<SymbolKey>,
) -> Vec<BytecodeGlobal> {
    variables
        .iter()
        .filter_map(|variable| {
            Some(BytecodeGlobal {
                key: *keys.get(variable.id.0)?,
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
                    .and_then(|owner| functions.get(owner.0).copied()),
            })
        })
        .collect()
}

fn ascii_uppercase(value: &str) -> Cow<'_, str> {
    if value.bytes().any(|byte| byte.is_ascii_lowercase()) {
        Cow::Owned(value.to_ascii_uppercase())
    } else {
        Cow::Borrowed(value)
    }
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
