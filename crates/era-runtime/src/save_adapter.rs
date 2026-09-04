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
    decode_scoped_save(bytes, artifact, SaveFileKind::Normal)
}

pub(crate) fn decode_scoped_save(
    bytes: &[u8],
    artifact: &BytecodeArtifact,
    kind: SaveFileKind,
) -> Result<DecodedEraSave, SaveCodecError> {
    decode_scoped_payload(bytes, artifact, kind)
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
            &text_layout(
                artifact,
                kind,
                Text1808Dialect::for_current_artifact(artifact),
            )?,
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
    // Structured extensions are decoded into the VM's ordinary or GLOBAL scope. Rebuild every
    // recognized record from that scope instead of retaining stale recognized records collected
    // from a previous payload.
    let mut output = opaque
        .iter()
        .filter(|extension| !matches!(extension.type_tag, 0x20..=0x22))
        .cloned()
        .collect::<Vec<_>>();
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
    encode_scoped_payload(
        state,
        artifact,
        SaveFileKind::Normal,
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
    let dialect = Text1808Dialect::for_current_artifact(artifact);
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
            &text_layout(artifact, kind, dialect)?,
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
    encode_scoped_payload(
        state,
        artifact,
        kind,
        description,
        opaque_extensions,
        format,
    )
}

fn text_layout(
    artifact: &BytecodeArtifact,
    kind: SaveFileKind,
    dialect: Text1808Dialect,
) -> Result<Text1808Layout, SaveCodecError> {
    let mut base_variables = Vec::new();
    let mut base_character_variables = Vec::new();
    // Ordinary saves begin with eight built-in String/Integer dictionaries. Version 1808 then
    // appends user arrays for ranks one through three. Snake inserts one Float dictionary after
    // each user String/Integer pair; the empty placeholders keep later supported groups aligned.
    let extended_count = dialect.shared_group_count(kind);
    let character_count = dialect.character_group_count();
    let mut extended_groups = vec![Vec::new(); extended_count];
    let mut extended_character_groups = vec![Vec::new(); character_count];
    let mut unsupported_extended_groups = vec![false; extended_count];
    let mut unsupported_extended_character_groups = vec![false; character_count];
    if dialect == Text1808Dialect::Snake1808 {
        for index in dialect.float_shared_groups(kind) {
            unsupported_extended_groups[*index] = true;
        }
        for index in dialect.float_character_groups() {
            unsupported_extended_character_groups[*index] = true;
        }
    }
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
            } else if let Some(index) = dialect.global_group(&variable) {
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
                if !user_defined {
                    let character_index = if dialect == Text1808Dialect::Snake1808 {
                        dialect.character_group(&variable)
                    } else {
                        Some(index)
                    };
                    if let Some(index) = character_index
                        && index < extended_character_groups.len()
                    {
                        extended_character_groups[index].push(variable);
                    }
                }
            } else if user_defined && !variable.dimensions.is_empty() {
                if let Some(user_index) = dialect.user_shared_group(&variable) {
                    extended_groups[user_index].push(variable);
                }
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
        unsupported_extended_groups,
        unsupported_extended_character_groups,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Text1808Dialect {
    Reference1808,
    Snake1808,
}

impl Text1808Dialect {
    fn for_current_artifact(artifact: &BytecodeArtifact) -> Self {
        if artifact.manifest.compatibility.profile == CompatibilityProfileId::EmueraSkiaSnake {
            Self::Snake1808
        } else {
            Self::Reference1808
        }
    }

    const fn shared_group_count(self, kind: SaveFileKind) -> usize {
        match (self, kind) {
            (Self::Snake1808, SaveFileKind::Global) => 9,
            (Self::Snake1808, _) => 17,
            (Self::Reference1808, SaveFileKind::Global) => 6,
            (Self::Reference1808, _) => 14,
        }
    }

    const fn character_group_count(self) -> usize {
        match self {
            Self::Snake1808 => 9,
            Self::Reference1808 => 6,
        }
    }

    const fn float_shared_groups(self, kind: SaveFileKind) -> &'static [usize] {
        match (self, kind) {
            (Self::Snake1808, SaveFileKind::Global) => &[2, 5, 8],
            (Self::Snake1808, _) => &[10, 13, 16],
            _ => &[],
        }
    }

    const fn float_character_groups(self) -> &'static [usize] {
        match self {
            Self::Snake1808 => &[2, 5, 8],
            Self::Reference1808 => &[],
        }
    }

    fn global_group(self, variable: &Text1808Variable) -> Option<usize> {
        let rank = variable.dimensions.len();
        let width = if self == Self::Snake1808 { 3 } else { 2 };
        (1..=3).contains(&rank).then_some(
            (rank - 1) * width + usize::from(variable.value_type == Text1808ValueType::Integer),
        )
    }

    fn user_shared_group(self, variable: &Text1808Variable) -> Option<usize> {
        let rank = variable.dimensions.len();
        if !(1..=3).contains(&rank) {
            return None;
        }
        let width = if self == Self::Snake1808 { 3 } else { 2 };
        Some(
            8 + (rank - 1) * width + usize::from(variable.value_type == Text1808ValueType::Integer),
        )
    }

    fn character_group(self, variable: &Text1808Variable) -> Option<usize> {
        if self != Self::Snake1808 {
            return None;
        }
        let rank = variable.dimensions.len();
        (rank <= 2)
            .then_some(rank * 3 + usize::from(variable.value_type == Text1808ValueType::Integer))
    }
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

    fn text_variable_for_mapping(value_type: Text1808ValueType, rank: usize) -> Text1808Variable {
        Text1808Variable {
            name: "MAPPED".into(),
            value_type,
            dimensions: vec![2; rank],
        }
    }

    #[test]
    fn text_dialects_freeze_reference_and_snake_group_mappings() {
        let reference = Text1808Dialect::Reference1808;
        let snake = Text1808Dialect::Snake1808;
        assert_eq!(reference.shared_group_count(SaveFileKind::Normal), 14);
        assert_eq!(reference.shared_group_count(SaveFileKind::Global), 6);
        assert_eq!(snake.shared_group_count(SaveFileKind::Normal), 17);
        assert_eq!(snake.shared_group_count(SaveFileKind::Global), 9);
        assert_eq!(reference.character_group_count(), 6);
        assert_eq!(snake.character_group_count(), 9);

        for (rank, reference_pair, snake_pair) in [
            (1, [8, 9], [8, 9]),
            (2, [10, 11], [11, 12]),
            (3, [12, 13], [14, 15]),
        ] {
            for (value_type, offset) in [
                (Text1808ValueType::String, 0),
                (Text1808ValueType::Integer, 1),
            ] {
                let variable = text_variable_for_mapping(value_type, rank);
                assert_eq!(
                    reference.user_shared_group(&variable),
                    Some(reference_pair[offset])
                );
                assert_eq!(snake.user_shared_group(&variable), Some(snake_pair[offset]));
                assert_eq!(
                    reference.global_group(&variable),
                    Some((rank - 1) * 2 + offset)
                );
                assert_eq!(snake.global_group(&variable), Some((rank - 1) * 3 + offset));
            }
        }
        for rank in 0..=2 {
            for (value_type, offset) in [
                (Text1808ValueType::String, 0),
                (Text1808ValueType::Integer, 1),
            ] {
                let variable = text_variable_for_mapping(value_type, rank);
                assert_eq!(snake.character_group(&variable), Some(rank * 3 + offset));
            }
        }
        assert_eq!(
            snake.float_shared_groups(SaveFileKind::Normal),
            &[10, 13, 16]
        );
        assert_eq!(snake.float_shared_groups(SaveFileKind::Global), &[2, 5, 8]);
        assert_eq!(snake.float_character_groups(), &[2, 5, 8]);
        assert!(
            reference
                .float_shared_groups(SaveFileKind::Normal)
                .is_empty()
        );
        assert!(reference.float_character_groups().is_empty());
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn snake_scoped_saves_use_standard_emuera_1808_only() {
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
        for kind in [
            SaveFileKind::Normal,
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
                assert!(matches!(
                    era_runtime_save::inspect_metadata(&encoded, true, SaveCodecLimits::default()),
                    Ok(era_runtime_save::SaveMetadataInspection::Complete { .. })
                ));
                let restored = decode_scoped_save(&encoded, &artifact, kind).unwrap();
                assert_eq!(restored.state.unique_code, 1);
                assert_eq!(restored.description, "profile fixture");
                assert!(decode_scoped_save(&encoded, &reference, kind).is_ok());
            }
        }
        for (kind, expected_groups) in [(SaveFileKind::Normal, 17), (SaveFileKind::Global, 9)] {
            let encoded = encode_scoped_save(
                &state,
                &artifact,
                kind,
                "text topology".into(),
                Vec::new(),
                SaveFormat::Text1808,
            )
            .unwrap();
            let source = std::str::from_utf8(&encoded).unwrap();
            assert_eq!(source.matches("__EMU_SEPARATOR__").count(), expected_groups);
            decode_scoped_save(&encoded, &artifact, kind).unwrap();
        }
        for format in [
            SaveFormat::Text1808,
            SaveFormat::Binary1808,
            SaveFormat::Binary1808Gzip,
        ] {
            let bare = encode_era_save(&state, &reference, "reference".into(), Vec::new(), format)
                .unwrap();
            if format == SaveFormat::Text1808 {
                assert!(decode_era_save(&bare, &artifact).is_err());
            } else {
                assert!(decode_era_save(&bare, &artifact).is_ok());
            }
            assert_eq!(
                decode_era_save(&bare, &reference).unwrap().description,
                "reference"
            );
        }
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
    fn structured_merge_drops_recognized_records_absent_from_the_current_scope() {
        let ordinary = encode_save_extension(
            &SaveExtension::DataTable {
                key: "ordinary".into(),
                schema: "ordinary-schema".into(),
                data: "ordinary-data".into(),
            },
            SaveCodecLimits::default(),
        )
        .unwrap();
        let global = StructuredExtension::DataTable {
            key: "global".into(),
            schema: "global-schema".into(),
            data: "global-data".into(),
        };

        let merged = merge_structured_extensions(&[ordinary], vec![global]).unwrap();

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].key, "global");
        assert_eq!(merged[0].type_tag, 0x22);
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
