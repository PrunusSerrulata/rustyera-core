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
mod tests;
