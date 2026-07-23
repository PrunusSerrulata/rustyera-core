use erabasic_data::ProjectData;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::io::Write;

use crate::{
    BytecodeType, COMPILER_ABI_VERSION, CONTAINER_VERSION, Digest, FormatVersion, HOST_ABI_VERSION,
    HostImport, ISA_VERSION, NATIVE_ABI_VERSION, NativeImport, ProgramVersion, SourceMap,
    SymbolKey, VM_ABI_VERSION,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportKind {
    Function,
    Native,
    Host,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FunctionImport {
    pub kind: ImportKind,
    pub key: SymbolKey,
}

/// The storage class is explicit in bytecode so a VM never has to reconstruct
/// lifetime rules from source names or project CSV metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BytecodeStorage {
    Project,
    FunctionLocal,
    FunctionPersistent,
    FunctionStatic,
    Character,
    Constant,
    Calculated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BytecodePersistence {
    None,
    GameSave,
    GlobalSave,
    ExtendedSave,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum BytecodeConstant {
    Integer(i64),
    String(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BytecodeGlobal {
    pub key: SymbolKey,
    pub name: String,
    pub value_type: crate::BytecodeType,
    pub dimensions: Vec<u64>,
    pub mutable: bool,
    pub storage: BytecodeStorage,
    pub persistence: BytecodePersistence,
    pub initial_values: Vec<BytecodeConstant>,
    /// Function-local and function-static variables name their owning function.
    pub owner: Option<SymbolKey>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BytecodeParameter {
    pub key: SymbolKey,
    /// Constant destination subscripts from the function label. Character
    /// variables keep the optional character selector as the leading entry.
    pub indices: Vec<u64>,
    pub value_type: BytecodeType,
    pub by_reference: bool,
    pub default: Option<BytecodeConstant>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct BytecodeCallCompatibility {
    pub allow_event_as_normal: bool,
    pub allow_omitted_arguments: bool,
    pub auto_convert_integer_to_string: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BytecodeFunctionKind {
    Normal,
    Event,
    System,
    Method,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BytecodeLabel {
    pub name: String,
    pub instruction: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BytecodeFunction {
    pub key: SymbolKey,
    pub name: String,
    pub kind: BytecodeFunctionKind,
    pub parameters: Vec<BytecodeParameter>,
    pub result: Option<BytecodeType>,
    pub labels: Vec<BytecodeLabel>,
    pub imports: Vec<FunctionImport>,
    pub code: Vec<crate::EncodedInstruction>,
    pub max_stack: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BytecodeEventEntry {
    pub function: SymbolKey,
    pub single: bool,
}

/// Ordered reference dispatch groups for one case-insensitive event name.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BytecodeEventGroup {
    pub name: String,
    pub only: Vec<BytecodeEventEntry>,
    pub priority: Vec<BytecodeEventEntry>,
    pub normal: Vec<BytecodeEventEntry>,
    pub later: Vec<BytecodeEventEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactManifest {
    pub container_version: FormatVersion,
    pub isa_version: FormatVersion,
    pub compiler_abi: u32,
    pub native_abi: u32,
    pub program_version: ProgramVersion,
    pub artifact_id: Digest,
    pub compiler_options: Digest,
    pub required_features: Vec<String>,
}

impl ArtifactManifest {
    #[must_use]
    pub fn new(compiler_options: Digest) -> Self {
        Self {
            container_version: CONTAINER_VERSION,
            isa_version: ISA_VERSION,
            compiler_abi: COMPILER_ABI_VERSION,
            native_abi: NATIVE_ABI_VERSION,
            program_version: ProgramVersion {
                vm_abi: VM_ABI_VERSION,
                host_abi: HOST_ABI_VERSION,
                execution_id: Digest::default(),
            },
            artifact_id: Digest::default(),
            compiler_options,
            required_features: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BytecodeArtifact {
    pub manifest: ArtifactManifest,
    pub call_compatibility: BytecodeCallCompatibility,
    pub project_data: ProjectData,
    pub globals: Vec<BytecodeGlobal>,
    pub native_imports: Vec<NativeImport>,
    pub host_imports: Vec<HostImport>,
    pub functions: Vec<BytecodeFunction>,
    pub event_groups: Vec<BytecodeEventGroup>,
    pub source_map: SourceMap,
}

impl BytecodeArtifact {
    /// Canonical ordering is part of the compiler ABI and is applied before hashing.
    pub fn canonicalize(&mut self) {
        self.manifest.required_features.sort();
        self.manifest.required_features.dedup();
        self.globals.sort_by_key(|global| global.key);
        self.native_imports.sort_by_key(|import| import.import.key);
        self.host_imports.sort_by_key(|import| import.import.key);
        self.functions.sort_by_key(|function| function.key);
        for function in &mut self.functions {
            function
                .labels
                .sort_by_key(|label| (label.name.to_ascii_uppercase(), label.name.clone()));
        }
        self.event_groups
            .sort_by_key(|group| (group.name.to_ascii_uppercase(), group.name.clone()));
        self.source_map
            .entries
            .sort_by_key(|entry| (entry.function, entry.code_start, entry.code_end));
    }

    /// Recompute execution and artifact identities after canonical ordering.
    ///
    /// # Errors
    ///
    /// Returns an error if one of the canonical section values cannot be encoded.
    pub fn refresh_ids(&mut self) -> Result<(), serde_json::Error> {
        self.canonicalize();
        let versions = canonical_digest(
            "rustyera.bytecode.identity.versions.v2",
            &(
                self.manifest.isa_version,
                self.manifest.compiler_abi,
                self.manifest.native_abi,
                self.manifest.program_version.vm_abi,
                self.manifest.program_version.host_abi,
                &self.manifest.compiler_options,
                &self.manifest.required_features,
            ),
        )?;
        let project =
            canonical_digest("rustyera.bytecode.identity.project.v2", &self.project_data)?;
        let globals = canonical_digest("rustyera.bytecode.identity.globals.v2", &self.globals)?;
        let native =
            canonical_digest("rustyera.bytecode.identity.native.v2", &self.native_imports)?;
        let host = canonical_digest("rustyera.bytecode.identity.host.v2", &self.host_imports)?;
        let functions = parallel_binary_digest(
            "rustyera.bytecode.identity.functions.v4",
            "rustyera.bytecode.identity.function-chunk.v4",
            &self.functions,
            256,
            encode_function_chunk,
        );
        let events = canonical_digest("rustyera.bytecode.identity.events.v2", &self.event_groups)?;
        let call_compatibility = canonical_digest(
            "rustyera.bytecode.identity.call-compatibility.v2",
            &self.call_compatibility,
        )?;
        let execution_id = Digest::hash(
            "rustyera.bytecode.execution.v2",
            &[
                &versions.0,
                &project.0,
                &globals.0,
                &native.0,
                &host.0,
                &functions.0,
                &events.0,
                &call_compatibility.0,
            ],
        );
        self.manifest.program_version.execution_id = execution_id;

        let source_records = canonical_digest(
            "rustyera.bytecode.identity.source-records.v3",
            &self.source_map.sources,
        )?;
        let statement_fingerprints = binary_digest_sequence(
            "rustyera.bytecode.identity.statement-fingerprints.v4",
            &self.source_map.statement_fingerprints,
        );
        let source_entries = parallel_binary_digest(
            "rustyera.bytecode.identity.source-entries.v4",
            "rustyera.bytecode.identity.source-entry-chunk.v4",
            &self.source_map.entries,
            65_536,
            encode_source_entry_chunk,
        );
        let sources = Digest::hash(
            "rustyera.bytecode.identity.sources.v4",
            &[
                &source_records.0,
                &statement_fingerprints.0,
                &source_entries.0,
            ],
        );
        self.manifest.artifact_id = Digest::hash(
            "rustyera.bytecode.artifact.v2",
            &[&execution_id.0, &sources.0],
        );
        Ok(())
    }

    #[must_use]
    pub fn into_unvalidated(self) -> UnvalidatedArtifact {
        UnvalidatedArtifact(self)
    }
}

fn canonical_digest<T: Serialize + ?Sized>(
    domain: &str,
    value: &T,
) -> Result<Digest, serde_json::Error> {
    let mut writer = DigestWriter {
        hasher: blake3::Hasher::new_derive_key(domain),
    };
    serde_json::to_writer(&mut writer, value)?;
    Ok(Digest(*writer.hasher.finalize().as_bytes()))
}

fn parallel_binary_digest<T: Sync>(
    domain: &str,
    chunk_domain: &str,
    values: &[T],
    chunk_size: usize,
    encode_chunk: fn(&[T], &mut Vec<u8>),
) -> Digest {
    let chunks = values
        .par_chunks(chunk_size)
        .map(|chunk| {
            let mut encoded = Vec::new();
            encode_chunk(chunk, &mut encoded);
            Digest::hash(chunk_domain, &[&encoded])
        })
        .collect::<Vec<_>>();
    binary_digest_sequence(domain, &chunks)
}

fn binary_digest_sequence(domain: &str, values: &[Digest]) -> Digest {
    let mut encoded = Vec::with_capacity(8 + values.len().saturating_mul(32));
    append_length(&mut encoded, values.len());
    for value in values {
        encoded.extend_from_slice(&value.0);
    }
    Digest::hash(domain, &[&encoded])
}

/// Canonical binary identity encoding for the bytecode section.
///
/// Unlike the public JSON representation, this internal versioned encoding avoids converting
/// millions of numeric operand bytes to decimal text. Every variable-width value is length
/// prefixed, every enum has an explicit tag, and the identity domains are versioned alongside the
/// compiler ABI.
fn encode_function_chunk(functions: &[BytecodeFunction], output: &mut Vec<u8>) {
    append_length(output, functions.len());
    for function in functions {
        output.extend_from_slice(&function.key.0);
        append_string(output, &function.name);
        output.push(match function.kind {
            BytecodeFunctionKind::Normal => 0,
            BytecodeFunctionKind::Event => 1,
            BytecodeFunctionKind::System => 2,
            BytecodeFunctionKind::Method => 3,
        });
        append_length(output, function.parameters.len());
        for parameter in &function.parameters {
            output.extend_from_slice(&parameter.key.0);
            append_length(output, parameter.indices.len());
            for index in &parameter.indices {
                output.extend_from_slice(&index.to_le_bytes());
            }
            append_bytecode_type(output, parameter.value_type);
            output.push(u8::from(parameter.by_reference));
            append_constant(output, parameter.default.as_ref());
        }
        match function.result {
            Some(value_type) => {
                output.push(1);
                append_bytecode_type(output, value_type);
            }
            None => output.push(0),
        }
        append_length(output, function.labels.len());
        for label in &function.labels {
            append_string(output, &label.name);
            output.extend_from_slice(&label.instruction.to_le_bytes());
        }
        append_length(output, function.imports.len());
        for import in &function.imports {
            output.push(match import.kind {
                ImportKind::Function => 0,
                ImportKind::Native => 1,
                ImportKind::Host => 2,
            });
            output.extend_from_slice(&import.key.0);
        }
        append_length(output, function.code.len());
        for instruction in &function.code {
            output.extend_from_slice(&instruction.opcode.to_le_bytes());
            append_length(output, instruction.payload.len());
            output.extend_from_slice(instruction.payload.as_slice());
        }
        output.extend_from_slice(&function.max_stack.to_le_bytes());
    }
}

fn append_bytecode_type(output: &mut Vec<u8>, value_type: BytecodeType) {
    output.push(match value_type {
        BytecodeType::Integer => 0,
        BytecodeType::String => 1,
        BytecodeType::IntegerPlace => 2,
        BytecodeType::StringPlace => 3,
    });
}

fn append_constant(output: &mut Vec<u8>, constant: Option<&BytecodeConstant>) {
    match constant {
        None => output.push(0),
        Some(BytecodeConstant::Integer(value)) => {
            output.push(1);
            output.extend_from_slice(&value.to_le_bytes());
        }
        Some(BytecodeConstant::String(value)) => {
            output.push(2);
            append_string(output, value);
        }
    }
}

fn encode_source_entry_chunk(entries: &[crate::SourceMapEntry], output: &mut Vec<u8>) {
    append_length(output, entries.len());
    let group_count = entries
        .windows(2)
        .filter(|pair| pair[0].function != pair[1].function)
        .count()
        + usize::from(!entries.is_empty());
    append_length(output, group_count);
    let mut group_start = 0;
    while group_start < entries.len() {
        let function = entries[group_start].function;
        let group_length =
            entries[group_start..].partition_point(|entry| entry.function == function);
        output.extend_from_slice(&function.0);
        append_length(output, group_length);
        for entry in &entries[group_start..group_start + group_length] {
            append_varint(output, entry.code_start);
            append_varint(output, entry.code_end);
            append_varint(output, entry.byte_start);
            append_varint(output, entry.byte_end);
            append_varint(output, u64::from(entry.statement_fingerprint));
            match entry.origin_chain.as_deref() {
                None => output.push(0),
                Some(origins) => {
                    output.push(1);
                    append_length(output, origins.len());
                    for &(source_index, byte_start, byte_end) in origins {
                        append_varint(output, u64::from(source_index));
                        append_varint(output, byte_start);
                        append_varint(output, byte_end);
                    }
                }
            }
            append_varint(output, u64::from(entry.source_index));
        }
        group_start += group_length;
    }
}

fn append_string(output: &mut Vec<u8>, value: &str) {
    append_length(output, value.len());
    output.extend_from_slice(value.as_bytes());
}

fn append_length(output: &mut Vec<u8>, value: usize) {
    output.extend_from_slice(&(value as u64).to_le_bytes());
}

fn append_varint(output: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        output.push(u8::try_from(value & 0x7f).expect("masked varint byte fits in u8") | 0x80);
        value >>= 7;
    }
    output.push(u8::try_from(value).expect("final varint byte fits in u8"));
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnvalidatedArtifact(pub(crate) BytecodeArtifact);

impl UnvalidatedArtifact {
    #[must_use]
    pub fn artifact(&self) -> &BytecodeArtifact {
        &self.0
    }

    #[must_use]
    pub fn into_inner(self) -> BytecodeArtifact {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeLimits {
    pub maximum_bytes: u64,
    pub maximum_section_bytes: u64,
    pub maximum_sections: u32,
    pub maximum_functions: u32,
    pub maximum_instructions: u64,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            maximum_bytes: 1024 * 1024 * 1024,
            maximum_section_bytes: 512 * 1024 * 1024,
            maximum_sections: 64,
            maximum_functions: 1_000_000,
            maximum_instructions: 100_000_000,
        }
    }
}
