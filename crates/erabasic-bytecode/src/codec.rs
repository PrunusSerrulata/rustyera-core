use std::{collections::BTreeMap, fmt};

use serde::{Serialize, de::DeserializeOwned};

use crate::{
    ArtifactManifest, BYTECODE_MAGIC, BytecodeArtifact, BytecodeCallCompatibility,
    BytecodeEventGroup, BytecodeFunction, BytecodeGlobal, CONTAINER_VERSION, DecodeLimits,
    HostImport, NativeImport, SourceMap, UnvalidatedArtifact,
};

const MANIFEST: u16 = 1;
const PROJECT_DATA: u16 = 2;
const GLOBALS: u16 = 3;
const NATIVE_IMPORTS: u16 = 4;
const HOST_IMPORTS: u16 = 5;
const FUNCTIONS: u16 = 6;
const SOURCE_MAP: u16 = 7;
const EVENT_GROUPS: u16 = 8;
const CALL_COMPATIBILITY: u16 = 9;
const RUNTIME_BUILTINS: u16 = 10;

#[derive(Debug)]
pub enum EncodeError {
    Json(serde_json::Error),
    TooLarge(&'static str),
}

impl fmt::Display for EncodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(formatter, "cannot encode bytecode section: {error}"),
            Self::TooLarge(name) => write!(formatter, "bytecode {name} exceeds the format limit"),
        }
    }
}

impl std::error::Error for EncodeError {}

impl From<serde_json::Error> for EncodeError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecodeError {
    LimitExceeded(&'static str),
    Truncated,
    InvalidMagic,
    UnsupportedContainer { major: u16, minor: u16 },
    UnknownRequiredSection(u16),
    DuplicateSection(u16),
    MissingSection(u16),
    CorruptSection(u16),
    InvalidSection { kind: u16, message: String },
    IdentityMismatch,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for DecodeError {}

/// Encode an artifact using the canonical `.erbc` container.
///
/// # Errors
///
/// Returns an error if a section cannot be serialized or exceeds a format limit.
pub fn encode_artifact(artifact: &BytecodeArtifact) -> Result<Vec<u8>, EncodeError> {
    let mut artifact = artifact.clone();
    artifact.refresh_ids()?;
    let sections = vec![
        section(MANIFEST, &artifact.manifest)?,
        section(PROJECT_DATA, &artifact.project_data)?,
        section(GLOBALS, &artifact.globals)?,
        section(NATIVE_IMPORTS, &artifact.native_imports)?,
        section(HOST_IMPORTS, &artifact.host_imports)?,
        section(FUNCTIONS, &artifact.functions)?,
        section(EVENT_GROUPS, &artifact.event_groups)?,
        section(CALL_COMPATIBILITY, &artifact.call_compatibility)?,
        section(RUNTIME_BUILTINS, &artifact.runtime_builtins)?,
        section(SOURCE_MAP, &artifact.source_map)?,
    ];
    let mut output = Vec::new();
    output.extend_from_slice(&BYTECODE_MAGIC);
    output.extend_from_slice(&CONTAINER_VERSION.major.to_le_bytes());
    output.extend_from_slice(&CONTAINER_VERSION.minor.to_le_bytes());
    let section_count =
        u32::try_from(sections.len()).map_err(|_| EncodeError::TooLarge("section count"))?;
    output.extend_from_slice(&section_count.to_le_bytes());
    for (kind, payload) in sections {
        let length = u64::try_from(payload.len()).map_err(|_| EncodeError::TooLarge("section"))?;
        output.extend_from_slice(&kind.to_le_bytes());
        output.push(1); // Every v1 section is required.
        output.push(0);
        output.extend_from_slice(&length.to_le_bytes());
        output.extend_from_slice(blake3::hash(&payload).as_bytes());
        output.extend_from_slice(&payload);
    }
    Ok(output)
}

/// Decode an untrusted `.erbc` container without marking it executable.
///
/// # Errors
///
/// Returns an error for malformed, unsupported, corrupt, or over-limit input.
pub fn decode_artifact(
    bytes: &[u8],
    limits: &DecodeLimits,
) -> Result<UnvalidatedArtifact, DecodeError> {
    if bytes.len() as u64 > limits.maximum_bytes {
        return Err(DecodeError::LimitExceeded("artifact bytes"));
    }
    let mut reader = Reader::new(bytes);
    if reader.take(8)? != BYTECODE_MAGIC {
        return Err(DecodeError::InvalidMagic);
    }
    let major = reader.u16()?;
    let minor = reader.u16()?;
    if major != CONTAINER_VERSION.major || minor > CONTAINER_VERSION.minor {
        return Err(DecodeError::UnsupportedContainer { major, minor });
    }
    let section_count = reader.u32()?;
    if section_count > limits.maximum_sections {
        return Err(DecodeError::LimitExceeded("section count"));
    }
    let mut sections = BTreeMap::new();
    for _ in 0..section_count {
        let kind = reader.u16()?;
        let required = reader.u8()? != 0;
        let _reserved = reader.u8()?;
        let length = reader.u64()?;
        if length > limits.maximum_section_bytes {
            return Err(DecodeError::LimitExceeded("section bytes"));
        }
        let expected = reader.take(32)?;
        let payload = reader.take(
            usize::try_from(length).map_err(|_| DecodeError::LimitExceeded("section bytes"))?,
        )?;
        if blake3::hash(payload).as_bytes() != expected {
            return Err(DecodeError::CorruptSection(kind));
        }
        if !(MANIFEST..=RUNTIME_BUILTINS).contains(&kind) {
            if required {
                return Err(DecodeError::UnknownRequiredSection(kind));
            }
            continue;
        }
        if sections.insert(kind, payload).is_some() {
            return Err(DecodeError::DuplicateSection(kind));
        }
    }
    if !reader.is_empty() {
        return Err(DecodeError::InvalidSection {
            kind: 0,
            message: "trailing bytes after final section".into(),
        });
    }

    let manifest = parse::<ArtifactManifest>(&sections, MANIFEST)?;
    let mut artifact = BytecodeArtifact {
        manifest,
        call_compatibility: parse::<BytecodeCallCompatibility>(&sections, CALL_COMPATIBILITY)?,
        runtime_builtins: parse(&sections, RUNTIME_BUILTINS)?,
        project_data: parse(&sections, PROJECT_DATA)?,
        globals: parse::<Vec<BytecodeGlobal>>(&sections, GLOBALS)?,
        native_imports: parse::<Vec<NativeImport>>(&sections, NATIVE_IMPORTS)?,
        host_imports: parse::<Vec<HostImport>>(&sections, HOST_IMPORTS)?,
        functions: parse::<Vec<BytecodeFunction>>(&sections, FUNCTIONS)?,
        event_groups: parse::<Vec<BytecodeEventGroup>>(&sections, EVENT_GROUPS)?,
        source_map: parse::<SourceMap>(&sections, SOURCE_MAP)?,
    };
    if artifact.functions.len() > usize::try_from(limits.maximum_functions).unwrap_or(usize::MAX)
        || artifact
            .functions
            .iter()
            .map(|function| function.code.len() as u64)
            .sum::<u64>()
            > limits.maximum_instructions
    {
        return Err(DecodeError::LimitExceeded("function or instruction count"));
    }
    let encoded_execution = artifact.manifest.program_version.execution_id;
    let encoded_artifact = artifact.manifest.artifact_id;
    artifact
        .refresh_ids()
        .map_err(|error| DecodeError::InvalidSection {
            kind: MANIFEST,
            message: error.to_string(),
        })?;
    if artifact.manifest.program_version.execution_id != encoded_execution
        || artifact.manifest.artifact_id != encoded_artifact
    {
        return Err(DecodeError::IdentityMismatch);
    }
    Ok(UnvalidatedArtifact(artifact))
}

fn section<T: Serialize>(kind: u16, value: &T) -> Result<(u16, Vec<u8>), serde_json::Error> {
    Ok((kind, serde_json::to_vec(value)?))
}

fn parse<T: DeserializeOwned>(
    sections: &BTreeMap<u16, &[u8]>,
    kind: u16,
) -> Result<T, DecodeError> {
    let payload = sections
        .get(&kind)
        .ok_or(DecodeError::MissingSection(kind))?;
    serde_json::from_slice(payload).map_err(|error| DecodeError::InvalidSection {
        kind,
        message: error.to_string(),
    })
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], DecodeError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or(DecodeError::Truncated)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(DecodeError::Truncated)?;
        self.offset = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(
            self.take(2)?.try_into().expect("two bytes were requested"),
        ))
    }

    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("four bytes were requested"),
        ))
    }

    fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(
            self.take(8)?
                .try_into()
                .expect("eight bytes were requested"),
        ))
    }

    fn is_empty(&self) -> bool {
        self.offset == self.bytes.len()
    }
}
