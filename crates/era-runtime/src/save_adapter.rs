use era_runtime_save::{
    OpaqueSaveExtension, SaveCodecError, SaveCodecLimits, SaveDocument, SaveExtension,
    SaveFileKind, SaveFormat, SaveMetadata, Text1808Layout, Text1808ValueType, Text1808Variable,
    decode_save_extension, decode_sparse, decode_text_with_layout, encode, encode_save_extension,
    encode_text_with_layout,
};
use erabasic_bytecode::{BytecodeArtifact, BytecodePersistence, BytecodeStorage, BytecodeType};
use erabasic_data::VariableId;
use erabasic_vm::{EraState, StructuredExtension};

mod entries;

use self::entries::{SaveDefinitionIndex, decode_entries, encode_entries};

// VariableData constructs this dictionary in a fixed order, then appends project-defined
// variables in declaration order. Its binary writer enumerates the same dictionary.
const REFERENCE_VARIABLE_ORDER: &[&str] = &[
    "DAY",
    "MONEY",
    "ITEM",
    "FLAG",
    "TFLAG",
    "UP",
    "PALAMLV",
    "EXPLV",
    "EJAC",
    "DOWN",
    "RESULT",
    "COUNT",
    "TARGET",
    "ASSI",
    "MASTER",
    "NOITEM",
    "LOSEBASE",
    "SELECTCOM",
    "ASSIPLAY",
    "PREVCOM",
    "TIME",
    "ITEMSALES",
    "PLAYER",
    "NEXTCOM",
    "PBAND",
    "BOUGHT",
    "A",
    "B",
    "C",
    "D",
    "E",
    "F",
    "G",
    "H",
    "I",
    "J",
    "K",
    "L",
    "M",
    "N",
    "O",
    "P",
    "Q",
    "R",
    "S",
    "T",
    "U",
    "V",
    "W",
    "X",
    "Y",
    "Z",
    "GLOBAL",
    "RANDDATA",
    "SAVESTR",
    "TSTR",
    "STR",
    "RESULTS",
    "GLOBALS",
    "SAVEDATA_TEXT",
    "ISASSI",
    "NO",
    "BASE",
    "MAXBASE",
    "ABL",
    "TALENT",
    "EXP",
    "MARK",
    "PALAM",
    "SOURCE",
    "EX",
    "CFLAG",
    "JUEL",
    "RELATION",
    "EQUIP",
    "TEQUIP",
    "STAIN",
    "GOTJUEL",
    "NOWEX",
    "DOWNBASE",
    "CUP",
    "CDOWN",
    "TCVAR",
    "NAME",
    "CALLNAME",
    "NICKNAME",
    "MASTERNAME",
    "CSTR",
    "CDFLAG",
    "DITEMTYPE",
    "DA",
    "DB",
    "DC",
    "DD",
    "DE",
    "TA",
    "TB",
];

pub(crate) struct DecodedEraSave {
    pub(crate) state: EraState,
    pub(crate) description: String,
    pub(crate) opaque_extensions: Vec<OpaqueSaveExtension>,
    pub(crate) structured_extensions: Vec<StructuredExtension>,
}

pub(crate) fn decode_era_save(
    bytes: &[u8],
    artifact: &BytecodeArtifact,
) -> Result<DecodedEraSave, SaveCodecError> {
    // Both binary headers begin with the non-UTF-8 0x89 signature byte.
    let document = if bytes.starts_with(&[0x89]) {
        decode_sparse(bytes, SaveCodecLimits::default())?
    } else {
        decode_text_with_layout(
            bytes,
            &text_layout(artifact, SaveFileKind::Normal)?,
            SaveCodecLimits::default(),
        )?
    };
    if document.kind != SaveFileKind::Normal {
        return Err(SaveCodecError::InvalidFormat(
            "start requires an ordinary save".into(),
        ));
    }
    let definitions = SaveDefinitionIndex::new(artifact);
    let variables = decode_entries(&document.variables, &definitions.shared)?;
    let characters = document
        .characters
        .iter()
        .map(|entries| decode_entries(entries, &definitions.character))
        .collect::<Result<Vec<_>, _>>()?;
    let (structured_extensions, opaque_extensions) = decode_extensions(document.opaque_extensions)?;
    Ok(DecodedEraSave {
        state: EraState {
            unique_code: document.metadata.unique_code,
            version: document.metadata.version,
            variables,
            characters,
        },
        description: document.metadata.description,
        opaque_extensions,
        structured_extensions,
    })
}

pub(crate) fn decode_scoped_save(
    bytes: &[u8],
    artifact: &BytecodeArtifact,
    kind: SaveFileKind,
) -> Result<DecodedEraSave, SaveCodecError> {
    let document = if bytes.starts_with(&[0x89]) {
        decode_sparse(bytes, SaveCodecLimits::default())?
    } else {
        decode_text_with_layout(
            bytes,
            &text_layout(artifact, kind)?,
            SaveCodecLimits::default(),
        )?
    };
    if document.kind != kind {
        return Err(SaveCodecError::InvalidFormat(
            "save file kind differs from the requested operation".into(),
        ));
    }
    let definitions = SaveDefinitionIndex::new(artifact);
    let variables = decode_entries(&document.variables, &definitions.shared)?;
    let characters = document
        .characters
        .iter()
        .map(|entries| decode_entries(entries, &definitions.character))
        .collect::<Result<Vec<_>, _>>()?;
    let (structured_extensions, opaque_extensions) = decode_extensions(document.opaque_extensions)?;
    Ok(DecodedEraSave {
        state: EraState {
            unique_code: document.metadata.unique_code,
            version: document.metadata.version,
            variables,
            characters,
        },
        description: document.metadata.description,
        opaque_extensions,
        structured_extensions,
    })
}

pub(crate) fn merge_structured_extensions(
    opaque: &[OpaqueSaveExtension],
    structured: Vec<StructuredExtension>,
) -> Result<Vec<OpaqueSaveExtension>, SaveCodecError> {
    let mut output = opaque.to_vec();
    for value in structured {
        let typed = match value {
            StructuredExtension::Map { key, entries } => SaveExtension::Map { key, entries },
            StructuredExtension::Xml { key, document } => SaveExtension::Xml { key, document },
            StructuredExtension::DataTable { key, schema, data } => {
                SaveExtension::DataTable { key, schema, data }
            }
        };
        let encoded = encode_save_extension(&typed, SaveCodecLimits::default())?;
        output.retain(|existing| {
            existing.type_tag != encoded.type_tag || existing.key != encoded.key
        });
        output.push(encoded);
    }
    output.sort_by(|left, right| (left.type_tag, &left.key).cmp(&(right.type_tag, &right.key)));
    Ok(output)
}

pub(crate) fn merge_opaque_extensions(
    current: &[OpaqueSaveExtension],
    incoming: Vec<OpaqueSaveExtension>,
) -> Vec<OpaqueSaveExtension> {
    let mut output = current.to_vec();
    for extension in incoming {
        output.retain(|existing| {
            existing.type_tag != extension.type_tag || existing.key != extension.key
        });
        output.push(extension);
    }
    output.sort_by(|left, right| (left.type_tag, &left.key).cmp(&(right.type_tag, &right.key)));
    output
}

fn decode_extensions(
    extensions: Vec<OpaqueSaveExtension>,
) -> Result<(Vec<StructuredExtension>, Vec<OpaqueSaveExtension>), SaveCodecError> {
    let mut structured = Vec::new();
    let mut opaque = Vec::new();
    for extension in extensions {
        opaque.push(extension.clone());
        if matches!(extension.type_tag, 0x20..=0x22) {
            structured.push(
                match decode_save_extension(&extension, SaveCodecLimits::default())? {
                    SaveExtension::Map { key, entries } => {
                        StructuredExtension::Map { key, entries }
                    }
                    SaveExtension::Xml { key, document } => {
                        StructuredExtension::Xml { key, document }
                    }
                    SaveExtension::DataTable { key, schema, data } => {
                        StructuredExtension::DataTable { key, schema, data }
                    }
                },
            );
        }
    }
    Ok((structured, opaque))
}

pub(crate) fn encode_era_save(
    state: &EraState,
    artifact: &BytecodeArtifact,
    description: String,
    opaque_extensions: Vec<OpaqueSaveExtension>,
    format: SaveFormat,
) -> Result<Vec<u8>, SaveCodecError> {
    let encoded_characters = state
        .characters
        .iter()
        .map(|variables| encode_entries(variables, artifact, true))
        .collect::<Result<Vec<_>, _>>()?;
    let character_user_defined_starts = encoded_characters
        .iter()
        .map(|encoded| encoded.user_defined_start)
        .collect();
    let characters = encoded_characters
        .into_iter()
        .map(|encoded| encoded.entries)
        .collect();
    let document = SaveDocument {
        format,
        kind: SaveFileKind::Normal,
        metadata: SaveMetadata {
            unique_code: state.unique_code,
            version: state.version,
            description,
        },
        characters,
        character_user_defined_starts,
        variables: encode_entries(&state.variables, artifact, false)?.entries,
        opaque_extensions,
        text_payload: None,
    };
    if format == SaveFormat::Text1808 {
        encode_text_with_layout(
            &document,
            &text_layout(artifact, SaveFileKind::Normal)?,
            SaveCodecLimits::default(),
        )
    } else {
        encode(&document, format, SaveCodecLimits::default())
    }
}

pub(crate) fn encode_scoped_save(
    state: &EraState,
    artifact: &BytecodeArtifact,
    kind: SaveFileKind,
    description: String,
    opaque_extensions: Vec<OpaqueSaveExtension>,
    format: SaveFormat,
) -> Result<Vec<u8>, SaveCodecError> {
    let encoded_characters = state
        .characters
        .iter()
        .map(|variables| encode_entries(variables, artifact, true))
        .collect::<Result<Vec<_>, _>>()?;
    let character_user_defined_starts = encoded_characters
        .iter()
        .map(|encoded| encoded.user_defined_start)
        .collect();
    let characters = encoded_characters
        .into_iter()
        .map(|encoded| encoded.entries)
        .collect();
    let document = SaveDocument {
        format,
        kind,
        metadata: SaveMetadata {
            unique_code: state.unique_code,
            version: state.version,
            description,
        },
        characters,
        character_user_defined_starts,
        variables: encode_entries(&state.variables, artifact, false)?.entries,
        opaque_extensions,
        text_payload: None,
    };
    if format == SaveFormat::Text1808 {
        encode_text_with_layout(
            &document,
            &text_layout(artifact, kind)?,
            SaveCodecLimits::default(),
        )
    } else {
        encode(&document, format, SaveCodecLimits::default())
    }
}

fn text_layout(
    artifact: &BytecodeArtifact,
    kind: SaveFileKind,
) -> Result<Text1808Layout, SaveCodecError> {
    let mut base_variables = Vec::new();
    let mut base_character_variables = Vec::new();
    // The first eight dictionaries are Emuera built-ins. Version 1808 appends six user-defined
    // array dictionaries (string/integer for ranks one through three).
    let mut extended_groups = vec![Vec::new(); 14];
    let mut extended_character_groups = vec![Vec::new(); 6];
    for definition in &artifact.globals {
        let schema = artifact
            .project_data
            .schema
            .variables
            .get(&definition.name.to_ascii_uppercase());
        let user_defined = schema.is_some_and(|schema| matches!(schema.id, VariableId::User(_)));
        let variable = Text1808Variable {
            name: definition.name.clone(),
            value_type: match definition.value_type {
                BytecodeType::Integer => Text1808ValueType::Integer,
                BytecodeType::String => Text1808ValueType::String,
                BytecodeType::IntegerPlace | BytecodeType::StringPlace => continue,
            },
            dimensions: definition
                .dimensions
                .iter()
                .map(|dimension| {
                    u32::try_from(*dimension)
                        .map_err(|_| SaveCodecError::LimitExceeded("array dimension"))
                })
                .collect::<Result<_, _>>()?,
        };
        if kind == SaveFileKind::Global {
            if definition.persistence != BytecodePersistence::GlobalSave {
                continue;
            }
            if matches!(definition.name.as_str(), "GLOBAL" | "GLOBALS") {
                base_variables.push(variable);
            } else if let Some(index) = extended_group(&variable) {
                extended_groups[index].push(variable);
            }
            continue;
        }
        if !matches!(
            definition.persistence,
            BytecodePersistence::GameSave | BytecodePersistence::ExtendedSave
        ) {
            continue;
        }
        let character = definition.storage == BytecodeStorage::Character;
        let positional = definition.persistence == BytecodePersistence::GameSave && !user_defined;
        if positional {
            if character {
                base_character_variables.push(variable);
            } else {
                base_variables.push(variable);
            }
        } else if let Some(index) = extended_group(&variable) {
            if character {
                // Reference text saves never added the later binary-only user character section.
                if !user_defined && index < extended_character_groups.len() {
                    extended_character_groups[index].push(variable);
                }
            } else if user_defined && !variable.dimensions.is_empty() {
                let user_index = 8
                    + (variable.dimensions.len() - 1) * 2
                    + usize::from(variable.value_type == Text1808ValueType::Integer);
                extended_groups[user_index].push(variable);
            } else {
                extended_groups[index].push(variable);
            }
        }
    }
    Ok(Text1808Layout {
        kind,
        base_variables,
        base_character_variables,
        extended_groups,
        extended_character_groups,
    })
}

fn extended_group(variable: &Text1808Variable) -> Option<usize> {
    let rank = variable.dimensions.len();
    (rank <= 3).then_some(
        rank * 2
            + match variable.value_type {
                Text1808ValueType::String => 0,
                Text1808ValueType::Integer => 1,
            },
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use era_runtime_save::{SaveEntry, SaveValue};
    use erabasic_bytecode::{
        ArtifactManifest, BytecodeCallCompatibility, BytecodeGlobal, Digest, SourceMap, SymbolKey,
    };
    use erabasic_csv::{CsvLoadOptions, ProjectFiles, load_project};
    use erabasic_data::{
        Persistence, StorageScope, ValueType, VariableId as DataVariableId, VariableSchema,
    };
    use erabasic_vm::{EraVariableState, VmValue};

    use super::entries::decode_value;
    use super::*;

    #[test]
    fn sparse_binary_values_cross_the_adapter_without_dense_materialization() {
        let decoded = decode_value(&SaveValue::SparseIntegers {
            dimensions: vec![1_000_000],
            values: vec![(17, 7), (999_999, 9)],
        });
        assert_eq!(decoded.value_type, BytecodeType::Integer);
        assert_eq!(decoded.dimensions, [1_000_000]);
        assert!(decoded.values.is_empty());
        assert_eq!(
            decoded.sparse_values.unwrap(),
            [(17, VmValue::Integer(7)), (999_999, VmValue::Integer(9))]
        );
    }

    #[test]
    fn structured_merge_replaces_declared_record_and_preserves_unknown_payload() {
        let unknown = OpaqueSaveExtension {
            type_tag: 0x7f,
            key: "future".into(),
            payload: vec![1, 2, 3],
        };
        let stale = encode_save_extension(
            &SaveExtension::Map {
                key: "state".into(),
                entries: vec![("old".into(), "value".into())],
            },
            SaveCodecLimits::default(),
        )
        .unwrap();
        let merged = merge_structured_extensions(
            &[unknown.clone(), stale],
            vec![StructuredExtension::Map {
                key: "state".into(),
                entries: vec![("new".into(), "value".into())],
            }],
        )
        .unwrap();

        assert!(merged.contains(&unknown));
        let map = merged
            .iter()
            .find(|value| value.type_tag == 0x20)
            .map(|value| decode_save_extension(value, SaveCodecLimits::default()).unwrap())
            .unwrap();
        assert_eq!(
            map,
            SaveExtension::Map {
                key: "state".into(),
                entries: vec![("new".into(), "value".into())],
            }
        );
    }

    #[test]
    fn binary_entries_follow_reference_and_user_declaration_order() {
        let mut project_data = load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
            .data
            .unwrap();
        for name in ["Z_USER", "A_USER"] {
            project_data.schema.register_user_variable(VariableSchema {
                id: DataVariableId::user(name),
                value_type: ValueType::Integer,
                storage: StorageScope::Character,
                dimensions: Vec::new(),
                mutable: true,
                persistence: Persistence::GameSave,
                can_forbid: false,
            });
        }
        let artifact = BytecodeArtifact {
            manifest: ArtifactManifest::new(Digest::default()),
            call_compatibility: BytecodeCallCompatibility::default(),
            project_data,
            globals: Vec::new(),
            native_imports: Vec::new(),
            host_imports: Vec::new(),
            functions: Vec::new(),
            event_groups: Vec::new(),
            source_map: SourceMap::default(),
        };
        let mut variables = BTreeMap::new();
        for (name, value) in [("A_USER", 4), ("CFLAG", 3), ("Z_USER", 2), ("NO", 1)] {
            let key = SymbolKey::derive("save-order-test", name.as_bytes());
            variables.insert(
                key,
                EraVariableState {
                    name: name.into(),
                    value_type: BytecodeType::Integer,
                    dimensions: Vec::new(),
                    persistence: BytecodePersistence::GameSave,
                    storage: BytecodeStorage::Character,
                    values: vec![VmValue::Integer(value)],
                    sparse_values: None,
                },
            );
        }

        let encoded = encode_entries(&variables, &artifact, true).unwrap();

        assert_eq!(
            encoded
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["NO", "CFLAG", "Z_USER", "A_USER"]
        );
        assert_eq!(encoded.user_defined_start, Some(2));
    }

    #[test]
    fn save_definition_index_separates_shared_and_character_entries() {
        let project_data = load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
            .data
            .unwrap();
        let definitions = [
            BytecodeGlobal {
                key: erabasic_bytecode::SymbolKey::derive("save-index", b"shared"),
                name: "SHARED_VALUE".into(),
                value_type: BytecodeType::Integer,
                dimensions: Vec::new(),
                mutable: true,
                storage: BytecodeStorage::Project,
                persistence: BytecodePersistence::GameSave,
                initial_values: Vec::new(),
                owner: None,
            },
            BytecodeGlobal {
                key: erabasic_bytecode::SymbolKey::derive("save-index", b"character"),
                name: "CHARACTER_VALUE".into(),
                value_type: BytecodeType::Integer,
                dimensions: Vec::new(),
                mutable: true,
                storage: BytecodeStorage::Character,
                persistence: BytecodePersistence::GameSave,
                initial_values: Vec::new(),
                owner: None,
            },
        ];
        let artifact = BytecodeArtifact {
            manifest: ArtifactManifest::new(Digest::default()),
            call_compatibility: BytecodeCallCompatibility::default(),
            project_data,
            globals: definitions.to_vec(),
            native_imports: Vec::new(),
            host_imports: Vec::new(),
            functions: Vec::new(),
            event_groups: Vec::new(),
            source_map: SourceMap::default(),
        };
        let index = SaveDefinitionIndex::new(&artifact);
        let shared = decode_entries(
            &[
                SaveEntry {
                    name: "shared_value".into(),
                    value: SaveValue::Integer(7),
                },
                SaveEntry {
                    name: "CHARACTER_VALUE".into(),
                    value: SaveValue::Integer(8),
                },
            ],
            &index.shared,
        )
        .unwrap();
        let character = decode_entries(
            &[
                SaveEntry {
                    name: "SHARED_VALUE".into(),
                    value: SaveValue::Integer(9),
                },
                SaveEntry {
                    name: "character_value".into(),
                    value: SaveValue::Integer(10),
                },
            ],
            &index.character,
        )
        .unwrap();

        assert_eq!(shared.len(), 1);
        assert_eq!(shared[&definitions[0].key].values, [VmValue::Integer(7)]);
        assert_eq!(character.len(), 1);
        assert_eq!(
            character[&definitions[1].key].values,
            [VmValue::Integer(10)]
        );
    }
}
