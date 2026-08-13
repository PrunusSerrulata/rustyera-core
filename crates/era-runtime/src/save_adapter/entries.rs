//! Legacy save entry conversion, including the reference-defined entry order.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use era_runtime_save::{SaveCodecError, SaveEntry, SaveValue};
use erabasic_bytecode::{
    BytecodeArtifact, BytecodeGlobal, BytecodeStorage, BytecodeType, SymbolKey,
};
use erabasic_vm::{EraVariableState, VmValue};

use super::REFERENCE_VARIABLE_ORDER;

pub(super) struct SaveDefinitionIndex<'a> {
    pub(super) shared: HashMap<String, &'a BytecodeGlobal>,
    pub(super) character: HashMap<String, &'a BytecodeGlobal>,
}

impl<'a> SaveDefinitionIndex<'a> {
    pub(super) fn new(artifact: &'a BytecodeArtifact) -> Self {
        let mut shared = HashMap::new();
        let mut character = HashMap::new();
        for definition in &artifact.globals {
            let definitions = match definition.storage {
                BytecodeStorage::Project | BytecodeStorage::FunctionStatic => &mut shared,
                BytecodeStorage::Character => &mut character,
                _ => continue,
            };
            definitions
                .entry(definition.name.to_ascii_uppercase())
                .or_insert(definition);
        }
        Self { shared, character }
    }
}

pub(super) fn decode_entries(
    entries: &[SaveEntry],
    definitions: &HashMap<String, &BytecodeGlobal>,
) -> Result<BTreeMap<SymbolKey, EraVariableState>, SaveCodecError> {
    let mut result = BTreeMap::new();
    let mut names = BTreeSet::new();
    for entry in entries {
        let normalized = entry.name.to_ascii_uppercase();
        let definition = definitions.get(&normalized).copied();
        if !names.insert(normalized) {
            return Err(SaveCodecError::InvalidFormat(format!(
                "duplicate saved variable {}",
                entry.name
            )));
        }
        let Some(definition) = definition else {
            // The reference binary reader ignores names absent from the current project.
            continue;
        };
        let DecodedValue {
            value_type,
            dimensions,
            values,
            sparse_values,
        } = decode_value(&entry.value);
        if value_type != definition.value_type {
            return Err(SaveCodecError::InvalidFormat(format!(
                "saved variable {} has the wrong type",
                entry.name
            )));
        }
        result.insert(
            definition.key,
            EraVariableState {
                name: definition.name.clone(),
                value_type,
                dimensions,
                persistence: definition.persistence,
                storage: definition.storage,
                values,
                sparse_values,
            },
        );
    }
    Ok(result)
}

pub(super) struct DecodedValue {
    pub(super) value_type: BytecodeType,
    pub(super) dimensions: Vec<u64>,
    pub(super) values: Vec<VmValue>,
    pub(super) sparse_values: Option<Vec<(u64, VmValue)>>,
}

pub(super) fn decode_value(value: &SaveValue) -> DecodedValue {
    match value {
        SaveValue::Integer(value) => DecodedValue {
            value_type: BytecodeType::Integer,
            dimensions: Vec::new(),
            values: vec![VmValue::Integer(*value)],
            sparse_values: None,
        },
        SaveValue::String(value) => DecodedValue {
            value_type: BytecodeType::String,
            dimensions: Vec::new(),
            values: vec![VmValue::String(value.clone())],
            sparse_values: None,
        },
        SaveValue::Integers { dimensions, values } => DecodedValue {
            value_type: BytecodeType::Integer,
            dimensions: dimensions.iter().map(|value| u64::from(*value)).collect(),
            values: values.iter().copied().map(VmValue::Integer).collect(),
            sparse_values: None,
        },
        SaveValue::Strings { dimensions, values } => DecodedValue {
            value_type: BytecodeType::String,
            dimensions: dimensions.iter().map(|value| u64::from(*value)).collect(),
            values: values.iter().cloned().map(VmValue::String).collect(),
            sparse_values: None,
        },
        SaveValue::SparseIntegers { dimensions, values } => DecodedValue {
            value_type: BytecodeType::Integer,
            dimensions: dimensions.iter().map(|value| u64::from(*value)).collect(),
            values: Vec::new(),
            sparse_values: Some(
                values
                    .iter()
                    .map(|(index, value)| (*index, VmValue::Integer(*value)))
                    .collect(),
            ),
        },
        SaveValue::SparseStrings { dimensions, values } => DecodedValue {
            value_type: BytecodeType::String,
            dimensions: dimensions.iter().map(|value| u64::from(*value)).collect(),
            values: Vec::new(),
            sparse_values: Some(
                values
                    .iter()
                    .map(|(index, value)| (*index, VmValue::String(value.clone())))
                    .collect(),
            ),
        },
    }
}

pub(super) struct EncodedEntries {
    pub(super) entries: Vec<SaveEntry>,
    pub(super) user_defined_start: Option<usize>,
}

pub(super) fn encode_entries(
    variables: &BTreeMap<SymbolKey, EraVariableState>,
    artifact: &BytecodeArtifact,
    character: bool,
) -> Result<EncodedEntries, SaveCodecError> {
    let mut by_name = BTreeMap::new();
    for (key, variable) in variables {
        by_name.insert(variable.name.to_ascii_uppercase(), (*key, variable));
    }
    let mut ordered = Vec::with_capacity(variables.len());
    let mut seen = BTreeSet::new();
    for name in REFERENCE_VARIABLE_ORDER {
        if let Some((key, variable)) = by_name.get(*name)
            && seen.insert(*key)
        {
            ordered.push(*variable);
        }
    }
    let user_defined_start = (character
        && artifact
            .project_data
            .schema
            .user_variable_order
            .iter()
            .filter_map(|name| artifact.project_data.schema.variables.get(name))
            .any(|schema| schema.storage == erabasic_data::StorageScope::Character))
    .then_some(ordered.len());
    for name in &artifact.project_data.schema.user_variable_order {
        if let Some((key, variable)) = by_name.get(name)
            && seen.insert(*key)
        {
            ordered.push(*variable);
        }
    }
    for (key, variable) in variables {
        if seen.insert(*key) {
            ordered.push(variable);
        }
    }
    Ok(EncodedEntries {
        entries: ordered
            .into_iter()
            .map(encode_entry)
            .collect::<Result<_, _>>()?,
        user_defined_start,
    })
}

fn encode_entry(variable: &EraVariableState) -> Result<SaveEntry, SaveCodecError> {
    let dimensions = variable
        .dimensions
        .iter()
        .map(|value| {
            u32::try_from(*value).map_err(|_| SaveCodecError::LimitExceeded("array dimension"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let value = if let Some(values) = &variable.sparse_values {
        match variable.value_type {
            BytecodeType::Integer => SaveValue::SparseIntegers {
                dimensions,
                values: values
                    .iter()
                    .map(|(index, value)| match value {
                        VmValue::Integer(value) => Ok((*index, *value)),
                        _ => Err(SaveCodecError::InvalidFormat(format!(
                            "saved variable {} contains a non-integer value",
                            variable.name
                        ))),
                    })
                    .collect::<Result<_, _>>()?,
            },
            BytecodeType::String => SaveValue::SparseStrings {
                dimensions,
                values: values
                    .iter()
                    .map(|(index, value)| match value {
                        VmValue::String(value) => Ok((*index, value.clone())),
                        _ => Err(SaveCodecError::InvalidFormat(format!(
                            "saved variable {} contains a non-string value",
                            variable.name
                        ))),
                    })
                    .collect::<Result<_, _>>()?,
            },
            BytecodeType::IntegerPlace | BytecodeType::StringPlace => {
                return Err(SaveCodecError::InvalidFormat(
                    "a saved variable cannot contain places".into(),
                ));
            }
        }
    } else {
        match (variable.value_type, dimensions.is_empty()) {
            (BytecodeType::Integer, true) => SaveValue::Integer(integer_at(variable, 0)?),
            (BytecodeType::String, true) => SaveValue::String(string_at(variable, 0)?.to_owned()),
            (BytecodeType::Integer, false) => SaveValue::Integers {
                dimensions,
                values: (0..variable.values.len())
                    .map(|index| integer_at(variable, index))
                    .collect::<Result<_, _>>()?,
            },
            (BytecodeType::String, false) => SaveValue::Strings {
                dimensions,
                values: (0..variable.values.len())
                    .map(|index| string_at(variable, index).map(str::to_owned))
                    .collect::<Result<_, _>>()?,
            },
            (BytecodeType::IntegerPlace | BytecodeType::StringPlace, _) => {
                return Err(SaveCodecError::InvalidFormat(
                    "a saved variable cannot contain places".into(),
                ));
            }
        }
    };
    Ok(SaveEntry {
        name: variable.name.clone(),
        value,
    })
}

fn integer_at(variable: &EraVariableState, index: usize) -> Result<i64, SaveCodecError> {
    match variable.values.get(index) {
        Some(VmValue::Integer(value)) => Ok(*value),
        _ => Err(SaveCodecError::InvalidFormat(format!(
            "saved variable {} contains a non-integer value",
            variable.name
        ))),
    }
}

fn string_at(variable: &EraVariableState, index: usize) -> Result<&str, SaveCodecError> {
    match variable.values.get(index) {
        Some(VmValue::String(value)) => Ok(value),
        _ => Err(SaveCodecError::InvalidFormat(format!(
            "saved variable {} contains a non-string value",
            variable.name
        ))),
    }
}
