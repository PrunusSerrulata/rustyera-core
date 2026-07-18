use erabasic_data::ProjectData;
use serde::{Deserialize, Serialize};

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
        let versions = serde_json::to_vec(&(
            self.manifest.isa_version,
            self.manifest.compiler_abi,
            self.manifest.native_abi,
            self.manifest.program_version.vm_abi,
            self.manifest.program_version.host_abi,
            &self.manifest.compiler_options,
            &self.manifest.required_features,
        ))?;
        let project = serde_json::to_vec(&self.project_data)?;
        let globals = serde_json::to_vec(&self.globals)?;
        let native = serde_json::to_vec(&self.native_imports)?;
        let host = serde_json::to_vec(&self.host_imports)?;
        let functions = serde_json::to_vec(&self.functions)?;
        let events = serde_json::to_vec(&self.event_groups)?;
        let call_compatibility = serde_json::to_vec(&self.call_compatibility)?;
        let execution_id = Digest::hash(
            "rustyera.bytecode.execution.v1",
            &[
                &versions,
                &project,
                &globals,
                &native,
                &host,
                &functions,
                &events,
                &call_compatibility,
            ],
        );
        self.manifest.program_version.execution_id = execution_id;

        let sources = serde_json::to_vec(&self.source_map)?;
        self.manifest.artifact_id = Digest::hash(
            "rustyera.bytecode.artifact.v1",
            &[&execution_id.0, &sources],
        );
        Ok(())
    }

    #[must_use]
    pub fn into_unvalidated(self) -> UnvalidatedArtifact {
        UnvalidatedArtifact(self)
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
