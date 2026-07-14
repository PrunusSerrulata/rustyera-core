//! Stable executable representation used between the compiler and VM.
//!
//! The persisted format is deliberately independent from Rust's memory layout.
//! Host-facing operations all use [`Opcode::CallHost`]; presentation or device
//! concepts never receive dedicated VM opcodes.

mod artifact;
mod codec;
mod host;
mod ids;
mod isa;
mod patch;
mod source_map;
mod version;

pub use artifact::{
    ArtifactManifest, BytecodeArtifact, BytecodeConstant, BytecodeFunction, BytecodeGlobal,
    BytecodeParameter, BytecodePersistence, BytecodeStorage, DecodeLimits, FunctionImport,
    ImportKind, UnvalidatedArtifact,
};
pub use codec::{DecodeError, EncodeError, decode_artifact, encode_artifact};
pub use host::{
    HostCapability, HostEffect, HostImport, HostSnapshotCapability, NativeImport, RuntimeImport,
    RuntimeImportKind,
};
pub use ids::{Digest, SymbolKey};
pub use isa::{BytecodeType, EncodedInstruction, Opcode, opcode};
pub use patch::{BytecodePatch, PatchError, apply_patch, create_patch};
pub use source_map::{ResolvedSourceLocation, SourceMap, SourceMapEntry, SourceRecord};
pub use version::{
    COMPILER_ABI_VERSION, CONTAINER_VERSION, FormatVersion, HOST_ABI_VERSION, ISA_VERSION,
    NATIVE_ABI_VERSION, ProgramVersion, VM_ABI_VERSION,
};

/// Eight-byte marker at the beginning of every `.erbc` file.
pub const BYTECODE_MAGIC: [u8; 8] = *b"RERABC\0\0";
