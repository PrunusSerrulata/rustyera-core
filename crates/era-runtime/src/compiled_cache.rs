use std::collections::BTreeSet;
use std::fmt;
use std::io::Write as _;
use std::ops::Range;
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};

use era_protocol::ProtocolBytes;
use era_runtime_protocol::{
    ConfigurationClientProfile, ExtensionDeclaration, ExternalResource, FileCategory, FilePayload,
    ProjectIdentity, ProjectManifest, ProtocolDiagnostic, SubmittedFile, validate_relative_path,
};
use erabasic_bytecode::{
    ArtifactManifest, BytecodeArtifact, BytecodeCallCompatibility, BytecodeEventGroup,
    BytecodeFunction, BytecodeGlobal, Digest, HostImport, NativeImport, SourceMap, SourceMapEntry,
    SourceRecord, SymbolKey,
};
use erabasic_compiler::IncrementalState;
use erabasic_validator::{ValidatedArtifact, validate_bytecode};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::project::{NormalizedProjectSnapshot, NormalizedResourceIdentity};
use crate::resource::ResourceGraph;

mod configuration_update;

use configuration_update::{
    ConfigurationJournal, StreamingConfigurationJournal, apply_journal, configuration_digest,
    encode_record, parse_journal, replace_configuration,
};

include!("compiled_cache/metadata.rs");
include!("compiled_cache/planning.rs");

mod buffer;
pub(crate) use buffer::ContainerBytes;
mod cooperative;
mod decode;
mod identity;
mod native;
mod sections;
mod stream;

use cooperative::ManifestSectionEncoder;
#[cfg(test)]
pub(crate) use decode::decode;
pub use decode::{
    decode_project_file, decode_project_file_frontend_manifest,
    prepare_project_configuration_update,
};
pub(crate) use decode::{decode_project_file_cache_with_progress, decode_with_progress};
#[cfg(test)]
pub(crate) use identity::{encode_compiled_cache_for_test, encode_full_project_for_test};
pub(crate) use identity::{
    project_identity, project_key, validate_full_project_manifest, validate_full_project_sources,
};
#[cfg(any(target_arch = "wasm32", test))]
use native::encode_project_file_header;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::{
    ProjectContainerControl, encode_cancellable, encode_full_project_cancellable,
};
#[allow(clippy::wildcard_imports)]
use sections::*;
pub use stream::{DecodedProjectFileStream, ProjectFileStreamDecoder};

#[cfg(test)]
use decode::{
    CacheDecodeDelays, compact_frontend_manifest, decode_cache_parts,
    decode_cache_parts_with_delays, parse_cache_sections,
};
#[cfg(test)]
use native::encode_manifest_section;

mod io;
#[cfg(test)]
mod tests;

use self::io::{
    HashWriter, decode_raw_section, decode_section, encode_raw_section, equal_ranges, read_section,
    read_stream_varint, read_u32, read_u64, read_varint, write_varint,
};
