//! C ABI declarations shared by the runtime dynamic library and its frontends.
//!
//! This safe crate fixes ownership, versioning and caller-driven pumping. The audited
//! raw-pointer implementation lives in `era-runtime-capi`.

use std::ffi::{c_char, c_void};

pub const ERA_RUNTIME_ABI_VERSION: EraAbiVersion = EraAbiVersion { major: 3, minor: 9 };
pub const ERA_RUNTIME_GET_API_SYMBOL: &str = "era_runtime_get_api";

pub const ERA_DEBUG_SCOPE_VARIABLES_READ: u64 = 1 << 0;
pub const ERA_DEBUG_SCOPE_VARIABLES_WRITE: u64 = 1 << 1;
pub const ERA_DEBUG_SCOPE_GAME_FIELDS_READ: u64 = 1 << 2;
pub const ERA_DEBUG_SCOPE_GAME_FIELDS_WRITE: u64 = 1 << 3;
pub const ERA_DEBUG_SCOPE_EXECUTION_READ: u64 = 1 << 4;
pub const ERA_DEBUG_SCOPE_EXECUTION_CONTROL: u64 = 1 << 5;
pub const ERA_DEBUG_SCOPE_CONSOLE_EVALUATE: u64 = 1 << 6;
pub const ERA_DEBUG_SCOPE_CONSOLE_EXECUTE: u64 = 1 << 7;
pub const ERA_DEBUG_SCOPE_BREAKPOINTS_MANAGE: u64 = 1 << 8;
pub const ERA_DEBUG_SCOPE_ALL: u64 = (1 << 10) - 1;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EraAbiVersion {
    pub major: u16,
    pub minor: u16,
}

/// Every C argument structure begins with this header. `struct_size` permits a newer
/// library to accept a shorter structure from an older frontend.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EraCallHeader {
    pub struct_size: u32,
    pub abi_version: EraAbiVersion,
}

impl EraCallHeader {
    #[must_use]
    pub fn for_type<T>() -> Self {
        Self {
            struct_size: size_u32::<T>(),
            abi_version: ERA_RUNTIME_ABI_VERSION,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EraSessionHandle {
    pub value: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EraByteSlice {
    pub data: *const u8,
    pub len: usize,
}

#[repr(C)]
#[derive(Debug)]
pub struct EraOwnedBuffer {
    pub data: *mut u8,
    pub len: usize,
    /// Opaque allocator identity returned unchanged to `release_buffer`.
    pub token: u64,
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EraStatus {
    Ok = 0,
    Empty = 1,
    Busy = 2,
    InvalidArgument = 3,
    AbiMismatch = 4,
    InvalidHandle = 5,
    ResourceLimit = 6,
    InternalError = 7,
}

/// Creator-only policy. A debug client may request a subset of `debug_scope_mask`
/// through the independent debug protocol but can never widen this mask.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EraCreateOptions {
    pub header: EraCallHeader,
    pub debug_scope_mask: u64,
    pub reserved: [u64; 4],
}

impl Default for EraCreateOptions {
    fn default() -> Self {
        Self {
            header: EraCallHeader::for_type::<Self>(),
            debug_scope_mask: 0,
            reserved: [0; 4],
        }
    }
}

/// Drive work by deterministic counters only. Wall-clock and deadline events always
/// arrive in versioned frontend messages instead of being sampled by the library.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EraDriveOptions {
    pub header: EraCallHeader,
    pub maximum_vm_instructions: u64,
    pub maximum_runtime_transitions: u32,
    pub reserved: u32,
}

impl Default for EraDriveOptions {
    fn default() -> Self {
        Self {
            header: EraCallHeader::for_type::<Self>(),
            maximum_vm_instructions: 100_000,
            maximum_runtime_transitions: 1024,
            reserved: 0,
        }
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EraDriveState {
    Idle = 0,
    MoreWork = 1,
    OutputReady = 2,
    Stopped = 3,
    Faulted = 4,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EraDriveResult {
    pub header: EraCallHeader,
    pub state: EraDriveState,
    pub vm_instructions: u64,
    pub runtime_transitions: u32,
    pub queued_envelopes: u32,
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EraProjectProgressStage {
    Scanning = 0,
    Normalizing = 1,
    LoadingData = 2,
    Parsing = 3,
    Analyzing = 4,
    Compiling = 5,
    Validating = 6,
    Finalizing = 7,
    Preparing = 8,
    Packaging = 9,
    CacheParsing = 10,
    CacheDecoding = 11,
    CacheValidating = 12,
    InitializingMemory = 13,
    IndexingProgram = 14,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EraProjectProgress {
    pub header: EraCallHeader,
    pub stage: EraProjectProgressStage,
    pub completed: u64,
    pub total: u64,
}

impl Default for EraDriveResult {
    fn default() -> Self {
        Self {
            header: EraCallHeader::for_type::<Self>(),
            state: EraDriveState::Idle,
            vm_instructions: 0,
            runtime_transitions: 0,
            queued_envelopes: 0,
        }
    }
}

/// Opaque slot for a C function pointer. The checked C header supplies the typed
/// signatures. Rust callers must not receive a safe function type accepting arbitrary
/// raw pointers; `era-runtime-capi` exposes the audited unsafe bindings.
pub type EraFunctionPointer = *const c_void;

/// Versioned function table returned by the `era_runtime_get_api` symbol.
/// `reserved[0]` in ABI 3.1 is an optional `EraSessionSetProjectProgressFn` extension.
/// `reserved[1]` in ABI 3.2 is an optional `EraSessionDecodeProjectFileFn` extension.
/// `reserved[2]` in ABI 3.3 is an optional compact frontend-manifest decoder with the same
/// function signature.
/// `reserved[3]` in ABI 3.4 is an optional owned compiled-cache staging fast path. It only stages
/// bytes; the authoritative project-load request and all observable results continue to use
/// submit/poll.
/// `reserved[4]` and `reserved[5]` in ABI 3.5 optionally allocate and commit a runtime-owned
/// writable compiled-cache buffer. A frontend may fill that buffer directly, then either commit
/// it once or return it unchanged to `release_buffer`. The caller must fill all bytes and must not
/// access the allocation concurrently with commit/release. A rejected header, handle, shape, or
/// session purpose leaves ownership with the caller; after those checks pass, commit consumes the
/// allocation even when staging returns busy, resource-limit, or internal-error status. Only a
/// successful commit writes the transfer ID.
/// `reserved[6]` in ABI 3.6 prepares an append-only project-configuration journal record. Its
/// owned output starts with the little-endian `u64` truncation offset followed by bytes to append.
/// `reserved[7]` in ABI 3.8 stages one CBOR-encoded project manifest for the next source-only
/// project-load command. The implementation copies and decodes the input before returning.
#[repr(C)]
pub struct EraRuntimeApi {
    pub struct_size: u32,
    pub abi_version: EraAbiVersion,
    pub implementation_name: *const c_char,
    pub implementation_context: *mut c_void,
    pub session_create: EraFunctionPointer,
    pub session_submit: EraFunctionPointer,
    pub session_drive: EraFunctionPointer,
    pub session_poll: EraFunctionPointer,
    pub session_destroy: EraFunctionPointer,
    pub release_buffer: EraFunctionPointer,
    pub last_error: EraFunctionPointer,
    pub reserved: [*mut c_void; 8],
}

fn size_u32<T>() -> u32 {
    u32::try_from(std::mem::size_of::<T>()).expect("FFI structures always fit in a u32")
}
