use std::collections::{BTreeMap, BTreeSet};

use era_runtime_save::{
    OpaqueSaveExtension, SaveCodecError, SaveCodecLimits, SaveDocument, SaveEntry, SaveFileKind,
    SaveFormat, SaveMetadata, SaveValue, decode, encode,
};
use erabasic_bytecode::{BytecodeArtifact, BytecodeStorage, BytecodeType};
use erabasic_vm::{EraState, EraVariableState, VmValue};

pub(crate) struct DecodedEraSave {
    pub(crate) state: EraState,
    pub(crate) opaque_extensions: Vec<OpaqueSaveExtension>,
}

pub(crate) fn decode_era_save(
    bytes: &[u8],
    artifact: &BytecodeArtifact,
) -> Result<DecodedEraSave, SaveCodecError> {
    let document = decode(bytes, SaveCodecLimits::default())?;
    if document.kind != SaveFileKind::Normal {
        return Err(SaveCodecError::InvalidFormat(
            "start requires an ordinary save".into(),
        ));
    }
    if document.format == SaveFormat::Text1808 {
        return Err(SaveCodecError::InvalidFormat(
            "text save restoration requires the project positional-layout adapter".into(),
        ));
    }
    let variables = decode_entries(&document.variables, artifact, false)?;
    let characters = document
        .characters
        .iter()
        .map(|entries| decode_entries(entries, artifact, true))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(DecodedEraSave {
        state: EraState {
            unique_code: document.metadata.unique_code,
            version: document.metadata.version,
            variables,
            characters,
        },
        opaque_extensions: document.opaque_extensions,
    })
}

pub(crate) fn encode_era_save(
    state: &EraState,
    description: String,
    opaque_extensions: Vec<OpaqueSaveExtension>,
    format: SaveFormat,
) -> Result<Vec<u8>, SaveCodecError> {
    let document = SaveDocument {
        format,
        kind: SaveFileKind::Normal,
        metadata: SaveMetadata {
            unique_code: state.unique_code,
            version: state.version,
            description,
        },
        characters: state
            .characters
            .iter()
            .map(|variables| encode_entries(variables.values()))
            .collect::<Result<Vec<_>, _>>()?,
        variables: encode_entries(state.variables.values())?,
        opaque_extensions,
        text_payload: None,
    };
    encode(&document, format, SaveCodecLimits::default())
}

fn decode_entries(
    entries: &[SaveEntry],
    artifact: &BytecodeArtifact,
    character: bool,
) -> Result<BTreeMap<erabasic_bytecode::SymbolKey, EraVariableState>, SaveCodecError> {
    let mut result = BTreeMap::new();
    let mut names = BTreeSet::new();
    for entry in entries {
        let normalized = entry.name.to_ascii_uppercase();
        if !names.insert(normalized) {
            return Err(SaveCodecError::InvalidFormat(format!(
                "duplicate saved variable {}",
                entry.name
            )));
        }
        let Some(definition) = artifact.globals.iter().find(|definition| {
            definition.name.eq_ignore_ascii_case(&entry.name)
                && if character {
                    definition.storage == BytecodeStorage::Character
                } else {
                    matches!(
                        definition.storage,
                        BytecodeStorage::Project | BytecodeStorage::FunctionStatic
                    )
                }
        }) else {
            // The reference binary reader ignores names absent from the current project.
            continue;
        };
        let (value_type, dimensions, values) = decode_value(&entry.value);
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
            },
        );
    }
    Ok(result)
}

fn decode_value(value: &SaveValue) -> (BytecodeType, Vec<u64>, Vec<VmValue>) {
    match value {
        SaveValue::Integer(value) => (
            BytecodeType::Integer,
            Vec::new(),
            vec![VmValue::Integer(*value)],
        ),
        SaveValue::String(value) => (
            BytecodeType::String,
            Vec::new(),
            vec![VmValue::String(value.clone())],
        ),
        SaveValue::Integers { dimensions, values } => (
            BytecodeType::Integer,
            dimensions.iter().map(|value| u64::from(*value)).collect(),
            values.iter().copied().map(VmValue::Integer).collect(),
        ),
        SaveValue::Strings { dimensions, values } => (
            BytecodeType::String,
            dimensions.iter().map(|value| u64::from(*value)).collect(),
            values.iter().cloned().map(VmValue::String).collect(),
        ),
    }
}

fn encode_entries<'a>(
    variables: impl Iterator<Item = &'a EraVariableState>,
) -> Result<Vec<SaveEntry>, SaveCodecError> {
    variables
        .map(|variable| {
            let dimensions = variable
                .dimensions
                .iter()
                .map(|value| {
                    u32::try_from(*value)
                        .map_err(|_| SaveCodecError::LimitExceeded("array dimension"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let value = match (variable.value_type, dimensions.is_empty()) {
                (BytecodeType::Integer, true) => SaveValue::Integer(integer_at(variable, 0)?),
                (BytecodeType::String, true) => {
                    SaveValue::String(string_at(variable, 0)?.to_owned())
                }
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
            };
            Ok(SaveEntry {
                name: variable.name.clone(),
                value,
            })
        })
        .collect()
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
