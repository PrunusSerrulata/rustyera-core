use era_runtime_protocol::{SqlDatabaseIdentityV1, SqlRevisionV1};
use era_runtime_save::{
    OpaqueSaveExtension, SaveCodecError, SaveCodecLimits, SaveDocument, SaveExtension,
    SaveFileKind, SaveFormat, SaveMetadata, Text1808Layout, Text1808ValueType, Text1808Variable,
    decode_save_extension, decode_sparse, decode_text_with_layout, encode, encode_save_extension,
    encode_text_with_layout,
};
use erabasic_bytecode::{BytecodeArtifact, BytecodePersistence, BytecodeStorage, BytecodeType};
use erabasic_compat::CompatibilityProfileId;
use erabasic_data::VariableId;
use erabasic_vm::{EraState, StructuredExtension};
use minicbor::{Decode, Encode};

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
    pub(crate) owned_state: Option<DecodedOwnedSaveState>,
}

pub(crate) struct DecodedOwnedSaveState {
    pub(crate) global_state: EraState,
    pub(crate) global_opaque_extensions: Vec<OpaqueSaveExtension>,
    pub(crate) global_structured_extensions: Vec<StructuredExtension>,
    pub(crate) sfmt_state: Vec<i64>,
    pub(crate) databases: Vec<OwnedDatabaseRevisionV1>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq)]
#[cbor(map)]
pub(crate) struct OwnedSaveStateV1 {
    #[n(0)]
    pub(crate) format_version: u32,
    #[n(1)]
    pub(crate) global_payload: minicbor::bytes::ByteVec,
    #[n(2)]
    pub(crate) sfmt_state: Vec<i64>,
    #[n(3)]
    pub(crate) databases: Vec<OwnedDatabaseRevisionV1>,
}

impl OwnedSaveStateV1 {
    pub(crate) const FORMAT_VERSION: u32 = 1;
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq)]
#[cbor(map)]
pub(crate) struct OwnedDatabaseRevisionV1 {
    #[n(0)]
    pub(crate) logical_name: String,
    #[n(1)]
    pub(crate) identity: SqlDatabaseIdentityV1,
    #[n(2)]
    pub(crate) exact_durable_revision: SqlRevisionV1,
}

pub(crate) fn decode_era_save(
    bytes: &[u8],
    artifact: &BytecodeArtifact,
) -> Result<DecodedEraSave, SaveCodecError> {
    decode_scoped_save(bytes, artifact, SaveFileKind::Normal)
}

pub(crate) fn decode_scoped_save(
    bytes: &[u8],
    artifact: &BytecodeArtifact,
    kind: SaveFileKind,
) -> Result<DecodedEraSave, SaveCodecError> {
    let envelope = era_runtime_save::unwrap_compatible_envelope(
        bytes,
        &artifact.manifest.compatibility,
        SaveCodecLimits::default(),
    )?;
    let mut decoded = decode_scoped_payload(envelope.payload, artifact, kind)?;
    match (artifact.manifest.compatibility.profile, kind) {
        (CompatibilityProfileId::EmueraSkiaSnake, SaveFileKind::Normal) => {
            if envelope.state.is_empty() {
                return Err(SaveCodecError::InvalidFormat(
                    "snake ordinary save is missing OwnedSaveStateV1".into(),
                ));
            }
            let owned = decode_owned_state(envelope.state, artifact)?;
            if owned.global_state.unique_code != decoded.state.unique_code
                || owned.global_state.version != decoded.state.version
            {
                return Err(SaveCodecError::InvalidFormat(
                    "OwnedSaveStateV1 GLOBAL identity differs from its ordinary payload".into(),
                ));
            }
            decoded.owned_state = Some(owned);
        }
        (CompatibilityProfileId::EmueraSkiaSnake, _) if !envelope.state.is_empty() => {
            return Err(SaveCodecError::InvalidFormat(
                "scoped snake save unexpectedly contains ordinary owned state".into(),
            ));
        }
        _ => {}
    }
    Ok(decoded)
}

fn decode_scoped_payload(
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
        owned_state: None,
    })
}

fn decode_owned_state(
    bytes: &[u8],
    artifact: &BytecodeArtifact,
) -> Result<DecodedOwnedSaveState, SaveCodecError> {
    preflight_owned_state(bytes)?;
    let state: OwnedSaveStateV1 = era_protocol::decode_canonical(bytes)
        .map_err(|error| SaveCodecError::InvalidFormat(error.to_string()))?;
    let canonical = era_protocol::encode_canonical(&state)
        .map_err(|error| SaveCodecError::InvalidFormat(error.to_string()))?;
    if canonical.as_slice() != bytes {
        return Err(SaveCodecError::InvalidFormat(
            "OwnedSaveStateV1 does not exactly match its canonical schema".into(),
        ));
    }
    validate_owned_state(&state)?;
    let global = decode_scoped_payload(
        state.global_payload.as_ref(),
        artifact,
        SaveFileKind::Global,
    )?;
    Ok(DecodedOwnedSaveState {
        global_state: global.state,
        global_opaque_extensions: global.opaque_extensions,
        global_structured_extensions: global.structured_extensions,
        sfmt_state: state.sfmt_state,
        databases: state.databases,
    })
}

fn preflight_owned_state(bytes: &[u8]) -> Result<(), SaveCodecError> {
    if bytes.len() > SaveCodecLimits::default().maximum_bytes {
        return Err(SaveCodecError::LimitExceeded("OwnedSaveStateV1 bytes"));
    }
    let mut decoder = minicbor::Decoder::new(bytes);
    let fields = decoder
        .map()
        .map_err(|error| SaveCodecError::InvalidFormat(error.to_string()))?
        .ok_or_else(|| {
            SaveCodecError::InvalidFormat("OwnedSaveStateV1 must use a definite map".into())
        })?;
    for _ in 0..fields {
        let field = decoder
            .u32()
            .map_err(|error| SaveCodecError::InvalidFormat(error.to_string()))?;
        match field {
            2 => {
                let count = decoder
                    .array()
                    .map_err(|error| SaveCodecError::InvalidFormat(error.to_string()))?
                    .ok_or_else(|| {
                        SaveCodecError::InvalidFormat(
                            "OwnedSaveStateV1 SFMT must use a definite array".into(),
                        )
                    })?;
                if count > 625 {
                    return Err(SaveCodecError::LimitExceeded("SFMT state elements"));
                }
                for _ in 0..count {
                    decoder
                        .skip()
                        .map_err(|error| SaveCodecError::InvalidFormat(error.to_string()))?;
                }
            }
            3 => {
                let count = decoder
                    .array()
                    .map_err(|error| SaveCodecError::InvalidFormat(error.to_string()))?
                    .ok_or_else(|| {
                        SaveCodecError::InvalidFormat(
                            "OwnedSaveStateV1 databases must use a definite array".into(),
                        )
                    })?;
                if count > u64::from(era_runtime_protocol::SqlLimitsV1::FIXED.maximum_connections) {
                    return Err(SaveCodecError::LimitExceeded("SQL connection count"));
                }
                for _ in 0..count {
                    decoder
                        .skip()
                        .map_err(|error| SaveCodecError::InvalidFormat(error.to_string()))?;
                }
            }
            _ => decoder
                .skip()
                .map_err(|error| SaveCodecError::InvalidFormat(error.to_string()))?,
        }
    }
    Ok(())
}

fn validate_owned_state(state: &OwnedSaveStateV1) -> Result<(), SaveCodecError> {
    if state.format_version != OwnedSaveStateV1::FORMAT_VERSION {
        return Err(SaveCodecError::InvalidFormat(format!(
            "unsupported OwnedSaveStateV1 format {}",
            state.format_version
        )));
    }
    if state.sfmt_state.len() != 625 || !(0..=624).contains(&state.sfmt_state[624]) {
        return Err(SaveCodecError::InvalidFormat(
            "OwnedSaveStateV1 contains an invalid SFMT snapshot".into(),
        ));
    }
    if state.databases.len() > era_runtime_protocol::SqlLimitsV1::FIXED.maximum_connections as usize
    {
        return Err(SaveCodecError::LimitExceeded("SQL connection count"));
    }
    let mut previous = None;
    for database in &state.databases {
        let normalized =
            crate::sql::normalize_sql_name(&database.logical_name).ok_or_else(|| {
                SaveCodecError::InvalidFormat(
                    "OwnedSaveStateV1 contains an invalid SQL logical identity".into(),
                )
            })?;
        if previous
            .as_ref()
            .is_some_and(|previous| previous >= &normalized)
        {
            return Err(SaveCodecError::InvalidFormat(
                "OwnedSaveStateV1 SQL identities are duplicated or not sorted".into(),
            ));
        }
        let source_valid = match &database.identity.source {
            era_runtime_protocol::SqlDatabaseSourceV1::Memory => true,
            era_runtime_protocol::SqlDatabaseSourceV1::ResourceSeed(seed) => {
                seed.sha256.as_slice().len() == 32
            }
        };
        if !source_valid
            || database.identity.sqlite_version != era_runtime_protocol::SQL_SQLITE_VERSION
            || database.identity.format_version != era_runtime_protocol::SQL_DATABASE_FORMAT_VERSION
            || database.exact_durable_revision.sha256.as_slice().len() != 32
        {
            return Err(SaveCodecError::InvalidFormat(
                "OwnedSaveStateV1 contains an unsupported SQL identity or revision".into(),
            ));
        }
        previous = Some(normalized);
    }
    Ok(())
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
    if artifact.manifest.compatibility.profile == CompatibilityProfileId::EmueraSkiaSnake {
        return Err(SaveCodecError::InvalidFormat(
            "snake ordinary saves require canonical OwnedSaveStateV1".into(),
        ));
    }
    let payload = encode_scoped_payload(
        state,
        artifact,
        SaveFileKind::Normal,
        description,
        opaque_extensions,
        format,
    )?;
    era_runtime_save::wrap_compatible_save(
        payload,
        &artifact.manifest.compatibility,
        SaveCodecLimits::default(),
    )
}

pub(crate) fn encode_owned_era_save(
    state: &EraState,
    artifact: &BytecodeArtifact,
    description: String,
    opaque_extensions: Vec<OpaqueSaveExtension>,
    owned_state: &OwnedSaveStateV1,
    format: SaveFormat,
) -> Result<Vec<u8>, SaveCodecError> {
    if artifact.manifest.compatibility.profile != CompatibilityProfileId::EmueraSkiaSnake {
        return encode_era_save(state, artifact, description, opaque_extensions, format);
    }
    validate_owned_state(owned_state)?;
    let payload = encode_scoped_payload(
        state,
        artifact,
        SaveFileKind::Normal,
        description,
        opaque_extensions,
        format,
    )?;
    let owned_state = era_protocol::encode_canonical(owned_state)
        .map_err(|error| SaveCodecError::InvalidFormat(error.to_string()))?;
    era_runtime_save::wrap_compatible_save_with_state(
        payload,
        &owned_state,
        &artifact.manifest.compatibility,
        SaveCodecLimits::default(),
    )
}

pub(crate) fn encode_scoped_save_payload(
    state: &EraState,
    artifact: &BytecodeArtifact,
    kind: SaveFileKind,
    description: String,
    opaque_extensions: Vec<OpaqueSaveExtension>,
    format: SaveFormat,
) -> Result<Vec<u8>, SaveCodecError> {
    encode_scoped_payload(
        state,
        artifact,
        kind,
        description,
        opaque_extensions,
        format,
    )
}

fn encode_scoped_payload(
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

pub(crate) fn encode_scoped_save(
    state: &EraState,
    artifact: &BytecodeArtifact,
    kind: SaveFileKind,
    description: String,
    opaque_extensions: Vec<OpaqueSaveExtension>,
    format: SaveFormat,
) -> Result<Vec<u8>, SaveCodecError> {
    if kind == SaveFileKind::Normal
        && artifact.manifest.compatibility.profile == CompatibilityProfileId::EmueraSkiaSnake
    {
        return Err(SaveCodecError::InvalidFormat(
            "snake ordinary saves require canonical OwnedSaveStateV1".into(),
        ));
    }
    let payload = encode_scoped_payload(
        state,
        artifact,
        kind,
        description,
        opaque_extensions,
        format,
    )?;
    era_runtime_save::wrap_compatible_save(
        payload,
        &artifact.manifest.compatibility,
        SaveCodecLimits::default(),
    )
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
    #[allow(clippy::too_many_lines)]
    fn snake_scoped_saves_round_trip_and_reference_sessions_reject_them() {
        use erabasic_compat::{CompatibilityIdentity, CompatibilityProfileId};
        let mut artifact = BytecodeArtifact {
            manifest: ArtifactManifest::new(Digest::default()),
            call_compatibility: BytecodeCallCompatibility::default(),
            runtime_builtins: Vec::new(),
            runtime_native_authorizations: Vec::new(),
            runtime_host_authorizations: Vec::new(),
            runtime_staged_authorizations: Vec::new(),
            runtime_variables: Vec::new(),
            project_data: load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
                .data
                .unwrap(),
            globals: Vec::new(),
            native_imports: Vec::new(),
            host_imports: Vec::new(),
            functions: Vec::new(),
            event_groups: Vec::new(),
            source_map: SourceMap::default(),
        };
        let reference = artifact.clone();
        artifact.manifest.compatibility =
            CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake);
        let state = EraState {
            unique_code: 1,
            version: 2,
            variables: BTreeMap::new(),
            characters: Vec::new(),
        };
        assert!(
            encode_era_save(
                &state,
                &artifact,
                "missing-owned-state".into(),
                Vec::new(),
                SaveFormat::Binary1808,
            )
            .is_err()
        );
        assert!(
            encode_scoped_save(
                &state,
                &artifact,
                SaveFileKind::Normal,
                "missing-owned-state".into(),
                Vec::new(),
                SaveFormat::Binary1808,
            )
            .is_err()
        );
        for kind in [
            SaveFileKind::Global,
            SaveFileKind::Variable,
            SaveFileKind::Character,
        ] {
            for format in [SaveFormat::Binary1808, SaveFormat::Binary1808Gzip] {
                let encoded = encode_scoped_save(
                    &state,
                    &artifact,
                    kind,
                    "profile fixture".into(),
                    Vec::new(),
                    format,
                )
                .unwrap();
                let restored = decode_scoped_save(&encoded, &artifact, kind).unwrap();
                assert_eq!(restored.state.unique_code, 1);
                assert_eq!(restored.description, "profile fixture");
                assert!(decode_scoped_save(&encoded, &reference, kind).is_err());
            }
        }
        for format in [
            SaveFormat::Text1808,
            SaveFormat::Binary1808,
            SaveFormat::Binary1808Gzip,
        ] {
            let global_payload = encode_scoped_save_payload(
                &state,
                &artifact,
                SaveFileKind::Global,
                String::new(),
                Vec::new(),
                SaveFormat::Binary1808,
            )
            .unwrap();
            let owned = OwnedSaveStateV1 {
                format_version: OwnedSaveStateV1::FORMAT_VERSION,
                global_payload: global_payload.into(),
                sfmt_state: vec![0; 625],
                databases: Vec::new(),
            };
            let encoded = encode_owned_era_save(
                &state,
                &artifact,
                "ordinary".into(),
                Vec::new(),
                &owned,
                format,
            )
            .unwrap();
            let restored = decode_era_save(&encoded, &artifact).unwrap();
            assert_eq!(restored.description, "ordinary");
            assert_eq!(restored.owned_state.unwrap().sfmt_state, vec![0; 625]);
            if format == SaveFormat::Binary1808 {
                let mut invalid_sfmt = owned.clone();
                invalid_sfmt.sfmt_state.pop();
                assert!(
                    encode_owned_era_save(
                        &state,
                        &artifact,
                        String::new(),
                        Vec::new(),
                        &invalid_sfmt,
                        format,
                    )
                    .is_err()
                );

                let identity = SqlDatabaseIdentityV1 {
                    source: era_runtime_protocol::SqlDatabaseSourceV1::Memory,
                    sqlite_version: era_runtime_protocol::SQL_SQLITE_VERSION.into(),
                    format_version: era_runtime_protocol::SQL_DATABASE_FORMAT_VERSION,
                };
                let revision = SqlRevisionV1 {
                    sha256: era_protocol::ProtocolBytes::new(vec![7; 32]),
                };
                let mut unsorted = owned.clone();
                unsorted.databases = ["z", "a"]
                    .into_iter()
                    .map(|logical_name| OwnedDatabaseRevisionV1 {
                        logical_name: logical_name.into(),
                        identity: identity.clone(),
                        exact_durable_revision: revision.clone(),
                    })
                    .collect();
                assert!(
                    encode_owned_era_save(
                        &state,
                        &artifact,
                        String::new(),
                        Vec::new(),
                        &unsorted,
                        format,
                    )
                    .is_err()
                );
            }
            let bare = encode_era_save(&state, &reference, "reference".into(), Vec::new(), format)
                .unwrap();
            assert!(decode_era_save(&bare, &artifact).is_err());
            assert_eq!(
                decode_era_save(&bare, &reference).unwrap().description,
                "reference"
            );
        }

        let global_payload = encode_scoped_save_payload(
            &state,
            &artifact,
            SaveFileKind::Global,
            String::new(),
            Vec::new(),
            SaveFormat::Binary1808,
        )
        .unwrap();
        let owned = |marker: i64| OwnedSaveStateV1 {
            format_version: OwnedSaveStateV1::FORMAT_VERSION,
            global_payload: global_payload.clone().into(),
            sfmt_state: vec![marker; 625],
            databases: Vec::new(),
        };
        let encode_owned = |owned: &OwnedSaveStateV1| {
            encode_owned_era_save(
                &state,
                &artifact,
                "isolated".into(),
                Vec::new(),
                owned,
                SaveFormat::Binary1808Gzip,
            )
            .unwrap()
        };
        let save_a = encode_owned(&owned(1));
        let save_b = encode_owned(&owned(2));
        let save_a_again = encode_owned(&owned(1));
        assert_eq!(save_a, save_a_again);
        assert_ne!(save_a, save_b);
        assert_eq!(
            decode_era_save(&save_a, &artifact)
                .unwrap()
                .owned_state
                .unwrap()
                .sfmt_state[0],
            1
        );
        assert_eq!(
            decode_era_save(&save_b, &artifact)
                .unwrap()
                .owned_state
                .unwrap()
                .sfmt_state[0],
            2
        );

        let ordinary_payload = encode_scoped_save_payload(
            &state,
            &artifact,
            SaveFileKind::Normal,
            String::new(),
            Vec::new(),
            SaveFormat::Binary1808,
        )
        .unwrap();
        let mut noncanonical_state = era_protocol::encode_canonical(&owned(3)).unwrap();
        noncanonical_state.push(0);
        let noncanonical = era_runtime_save::wrap_compatible_save_with_state(
            ordinary_payload,
            &noncanonical_state,
            &artifact.manifest.compatibility,
            SaveCodecLimits::default(),
        )
        .unwrap();
        assert!(decode_era_save(&noncanonical, &artifact).is_err());

        let mut extended_state = era_protocol::encode_canonical(&owned(3)).unwrap();
        assert_eq!(extended_state[0], 0xa4);
        extended_state[0] = 0xa5;
        extended_state.extend_from_slice(&[0x04, 0xf6]);
        let extended = era_runtime_save::wrap_compatible_save_with_state(
            encode_scoped_save_payload(
                &state,
                &artifact,
                SaveFileKind::Normal,
                String::new(),
                Vec::new(),
                SaveFormat::Binary1808,
            )
            .unwrap(),
            &extended_state,
            &artifact.manifest.compatibility,
            SaveCodecLimits::default(),
        )
        .unwrap();
        assert!(decode_era_save(&extended, &artifact).is_err());

        let mut oversized_sfmt = minicbor::Encoder::new(Vec::new());
        oversized_sfmt
            .map(1)
            .unwrap()
            .u8(2)
            .unwrap()
            .array(626)
            .unwrap();
        for _ in 0..626 {
            oversized_sfmt.i64(0).unwrap();
        }
        assert!(matches!(
            decode_owned_state(&oversized_sfmt.into_writer(), &artifact),
            Err(SaveCodecError::LimitExceeded("SFMT state elements"))
        ));
        let mut oversized_databases = minicbor::Encoder::new(Vec::new());
        oversized_databases
            .map(1)
            .unwrap()
            .u8(3)
            .unwrap()
            .array(9)
            .unwrap();
        assert!(matches!(
            decode_owned_state(&oversized_databases.into_writer(), &artifact),
            Err(SaveCodecError::LimitExceeded("SQL connection count"))
        ));

        let mut foreign_global = state.clone();
        foreign_global.version += 1;
        let mut mismatched = owned(4);
        mismatched.global_payload = encode_scoped_save_payload(
            &foreign_global,
            &artifact,
            SaveFileKind::Global,
            String::new(),
            Vec::new(),
            SaveFormat::Binary1808,
        )
        .unwrap()
        .into();
        assert!(decode_era_save(&encode_owned(&mismatched), &artifact).is_err());
    }

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
            runtime_builtins: Vec::new(),
            runtime_native_authorizations: Vec::new(),
            runtime_host_authorizations: Vec::new(),
            runtime_staged_authorizations: Vec::new(),
            runtime_variables: Vec::new(),
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
            runtime_builtins: Vec::new(),
            runtime_native_authorizations: Vec::new(),
            runtime_host_authorizations: Vec::new(),
            runtime_staged_authorizations: Vec::new(),
            runtime_variables: definitions
                .iter()
                .map(|global| erabasic_bytecode::RuntimeVariableSymbol {
                    match_name_rejection: None,
                    character_disposal: erabasic_bytecode::CharacterArrayDisposal::Preserve,
                    key: global.key,
                    reference: false,
                    reference_semantics: erabasic_bytecode::RuntimeReferenceSemantics {
                        is_const: false,
                        can_restructure: false,
                    },
                })
                .collect(),
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
