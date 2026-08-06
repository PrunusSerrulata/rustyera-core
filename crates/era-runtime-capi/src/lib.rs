//! Audited unsafe adapter for the caller-pumped runtime C ABI.
//!
//! All game state remains in safe [`era_runtime`] code. Raw pointers are copied or
//! validated at this boundary and no borrowed frontend memory survives a call.

#![deny(unsafe_op_in_unsafe_fn)]

use std::collections::BTreeMap;
use std::ffi::{c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Mutex, OnceLock};

use era_protocol::SessionId;
use era_runtime::{
    ProjectProgressReporter, ProjectProgressStage, RuntimeDriveBudget, RuntimeDriveState,
    RuntimeOptions, RuntimeSession,
};
use era_runtime_ffi::{
    ERA_DEBUG_SCOPE_ALL, ERA_RUNTIME_ABI_VERSION, EraAbiVersion, EraByteSlice, EraCallHeader,
    EraCreateOptions, EraDriveOptions, EraDriveResult, EraDriveState, EraOwnedBuffer,
    EraProjectProgress, EraProjectProgressStage, EraRuntimeApi, EraSessionHandle, EraStatus,
};

static IMPLEMENTATION_NAME: &[u8] = b"RustyEra runtime\0";

struct SessionRecord {
    runtime: RuntimeSession,
    last_error: String,
}

struct Registry {
    next_handle: u64,
    next_buffer: u64,
    sessions: BTreeMap<u64, SessionRecord>,
    buffers: BTreeMap<u64, Box<[u8]>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            next_handle: 1,
            next_buffer: 1,
            sessions: BTreeMap::new(),
            buffers: BTreeMap::new(),
        }
    }
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}

/// Return the ABI v3 function table. The table itself contains no Rust layout.
#[unsafe(no_mangle)]
/// # Safety
///
/// `out_api` must point to writable storage for one complete [`EraRuntimeApi`].
pub unsafe extern "C" fn era_runtime_get_api(
    requested: EraAbiVersion,
    out_api: *mut EraRuntimeApi,
) -> EraStatus {
    ffi_status(|| {
        if requested.major != ERA_RUNTIME_ABI_VERSION.major || out_api.is_null() {
            return EraStatus::AbiMismatch;
        }
        let mut reserved = [std::ptr::null_mut(); 8];
        reserved[0] = session_set_project_progress as *const () as *mut c_void;
        reserved[1] = session_decode_project_file as *const () as *mut c_void;
        reserved[2] = session_decode_project_file_frontend as *const () as *mut c_void;
        let api = EraRuntimeApi {
            struct_size: u32::try_from(std::mem::size_of::<EraRuntimeApi>()).unwrap_or(u32::MAX),
            abi_version: ERA_RUNTIME_ABI_VERSION,
            implementation_name: IMPLEMENTATION_NAME.as_ptr().cast::<c_char>(),
            implementation_context: std::ptr::null_mut(),
            session_create: session_create as *const () as *const c_void,
            session_submit: session_submit as *const () as *const c_void,
            session_drive: session_drive as *const () as *const c_void,
            session_poll: session_poll as *const () as *const c_void,
            session_destroy: session_destroy as *const () as *const c_void,
            release_buffer: release_buffer as *const () as *const c_void,
            last_error: last_error as *const () as *const c_void,
            reserved,
        };
        // SAFETY: null was rejected and the caller contract requires writable storage
        // for one complete EraRuntimeApi value.
        unsafe { out_api.write(api) };
        EraStatus::Ok
    })
}

extern "C" fn session_decode_project_file(
    header: EraCallHeader,
    handle: EraSessionHandle,
    input: EraByteSlice,
    out_buffer: *mut EraOwnedBuffer,
) -> EraStatus {
    decode_project_file_buffer(header, handle, input, out_buffer, false)
}

extern "C" fn session_decode_project_file_frontend(
    header: EraCallHeader,
    handle: EraSessionHandle,
    input: EraByteSlice,
    out_buffer: *mut EraOwnedBuffer,
) -> EraStatus {
    decode_project_file_buffer(header, handle, input, out_buffer, true)
}

fn decode_project_file_buffer(
    header: EraCallHeader,
    handle: EraSessionHandle,
    input: EraByteSlice,
    out_buffer: *mut EraOwnedBuffer,
    compact: bool,
) -> EraStatus {
    ffi_status(|| {
        if !valid_header::<EraOwnedBuffer>(header)
            || out_buffer.is_null()
            || (input.data.is_null() && input.len != 0)
        {
            return EraStatus::InvalidArgument;
        }
        let bytes = if input.len == 0 {
            &[]
        } else {
            // SAFETY: the non-empty pointer/length pair was validated and is borrowed only
            // for this call.
            unsafe { std::slice::from_raw_parts(input.data, input.len) }
        };
        {
            let registry = lock_registry();
            if !registry.sessions.contains_key(&handle.value) {
                return EraStatus::InvalidHandle;
            }
        }
        let decoded = if compact {
            era_runtime::decode_project_file_frontend_manifest(bytes, input.len)
        } else {
            era_runtime::decode_project_file(bytes, input.len)
        };
        let manifest = match decoded {
            Ok(decoded) => decoded.manifest,
            Err(error) => {
                let mut registry = lock_registry();
                let Some(record) = registry.sessions.get_mut(&handle.value) else {
                    return EraStatus::InvalidHandle;
                };
                record.last_error = error.to_string();
                return EraStatus::InvalidArgument;
            }
        };
        let encoded = match minicbor::to_vec(manifest) {
            Ok(encoded) => encoded,
            Err(error) => {
                let mut registry = lock_registry();
                let Some(record) = registry.sessions.get_mut(&handle.value) else {
                    return EraStatus::InvalidHandle;
                };
                record.last_error = error.to_string();
                return EraStatus::InternalError;
            }
        };
        let mut registry = lock_registry();
        if !registry.sessions.contains_key(&handle.value) {
            return EraStatus::InvalidHandle;
        }
        write_owned_buffer(&mut registry, encoded, out_buffer)
    })
}

type ProjectProgressCallback = extern "C" fn(*mut c_void, EraProjectProgress);

extern "C" fn session_set_project_progress(
    header: EraCallHeader,
    handle: EraSessionHandle,
    callback: Option<ProjectProgressCallback>,
    context: *mut c_void,
) -> EraStatus {
    ffi_status(|| {
        if !valid_header::<EraCallHeader>(header) {
            return EraStatus::AbiMismatch;
        }
        let mut registry = lock_registry();
        let Some(record) = registry.sessions.get_mut(&handle.value) else {
            return EraStatus::InvalidHandle;
        };
        let reporter = callback.map(|callback| {
            let context = context as usize;
            ProjectProgressReporter::new(move |progress| {
                callback(
                    context as *mut c_void,
                    EraProjectProgress {
                        header: EraCallHeader::for_type::<EraProjectProgress>(),
                        stage: project_progress_stage(progress.stage),
                        completed: progress.completed,
                        total: progress.total,
                    },
                );
            })
        });
        record.runtime.set_project_progress_reporter(reporter);
        EraStatus::Ok
    })
}

const fn project_progress_stage(stage: ProjectProgressStage) -> EraProjectProgressStage {
    match stage {
        ProjectProgressStage::Scanning => EraProjectProgressStage::Scanning,
        ProjectProgressStage::Normalizing => EraProjectProgressStage::Normalizing,
        ProjectProgressStage::LoadingData => EraProjectProgressStage::LoadingData,
        ProjectProgressStage::Parsing => EraProjectProgressStage::Parsing,
        ProjectProgressStage::Analyzing => EraProjectProgressStage::Analyzing,
        ProjectProgressStage::Compiling => EraProjectProgressStage::Compiling,
        ProjectProgressStage::Validating => EraProjectProgressStage::Validating,
        ProjectProgressStage::Finalizing => EraProjectProgressStage::Finalizing,
        ProjectProgressStage::Preparing => EraProjectProgressStage::Preparing,
    }
}

extern "C" fn session_create(
    header: EraCallHeader,
    options: *const EraCreateOptions,
    out_handle: *mut EraSessionHandle,
) -> EraStatus {
    ffi_status(|| {
        if !valid_header::<EraCreateOptions>(header) || options.is_null() || out_handle.is_null() {
            return EraStatus::InvalidArgument;
        }
        // SAFETY: pointers were checked and the ABI requires both pointees to remain
        // valid for the duration of this call only.
        let options = unsafe { &*options };
        if !valid_header::<EraCreateOptions>(options.header) {
            return EraStatus::AbiMismatch;
        }
        if options.debug_scope_mask & !ERA_DEBUG_SCOPE_ALL != 0 {
            return EraStatus::InvalidArgument;
        }
        let mut registry = lock_registry();
        let handle = registry.next_handle;
        registry.next_handle = registry.next_handle.saturating_add(1);
        let runtime = RuntimeSession::new(RuntimeOptions {
            session_id: SessionId {
                high: 0x5255_5354_5945_5241,
                low: handle,
            },
            debug_scope_mask: options.debug_scope_mask,
            ..RuntimeOptions::default()
        });
        registry.sessions.insert(
            handle,
            SessionRecord {
                runtime,
                last_error: String::new(),
            },
        );
        // SAFETY: out_handle was checked and is exclusively owned by the caller.
        unsafe { out_handle.write(EraSessionHandle { value: handle }) };
        EraStatus::Ok
    })
}

extern "C" fn session_submit(
    header: EraCallHeader,
    handle: EraSessionHandle,
    input: EraByteSlice,
) -> EraStatus {
    ffi_status(|| {
        if !valid_header::<EraCallHeader>(header) || (input.data.is_null() && input.len != 0) {
            return EraStatus::InvalidArgument;
        }
        let mut registry = lock_registry();
        let Some(record) = registry.sessions.get_mut(&handle.value) else {
            return EraStatus::InvalidHandle;
        };
        // SAFETY: the null/length pair was validated. The bytes are copied by the
        // runtime decoder before this function returns.
        let bytes = if input.len == 0 {
            &[]
        } else {
            // SAFETY: a non-empty slice has a non-null pointer by validation above.
            unsafe { std::slice::from_raw_parts(input.data, input.len) }
        };
        match record.runtime.submit_envelope(bytes) {
            Ok(()) => EraStatus::Ok,
            Err(error) => {
                record.last_error = error.to_string();
                EraStatus::InvalidArgument
            }
        }
    })
}

extern "C" fn session_drive(
    header: EraCallHeader,
    handle: EraSessionHandle,
    options: *const EraDriveOptions,
    out_result: *mut EraDriveResult,
) -> EraStatus {
    ffi_status(|| {
        if !valid_header::<EraDriveOptions>(header) || options.is_null() || out_result.is_null() {
            return EraStatus::InvalidArgument;
        }
        // SAFETY: both pointers were checked and are only accessed during this call.
        let options = unsafe { &*options };
        if !valid_header::<EraDriveOptions>(options.header) {
            return EraStatus::AbiMismatch;
        }
        let mut registry = lock_registry();
        let Some(record) = registry.sessions.get_mut(&handle.value) else {
            return EraStatus::InvalidHandle;
        };
        match record.runtime.drive(RuntimeDriveBudget {
            maximum_vm_instructions: options.maximum_vm_instructions,
            maximum_runtime_transitions: options.maximum_runtime_transitions,
        }) {
            Ok(report) => {
                // SAFETY: out_result was checked and the caller provided writable storage.
                unsafe {
                    out_result.write(EraDriveResult {
                        header: EraCallHeader::for_type::<EraDriveResult>(),
                        state: drive_state(report.state),
                        vm_instructions: report.vm_instructions,
                        runtime_transitions: report.runtime_transitions,
                        queued_envelopes: report.queued_envelopes,
                    });
                }
                EraStatus::Ok
            }
            Err(error) => {
                record.last_error = error.to_string();
                EraStatus::InternalError
            }
        }
    })
}

extern "C" fn session_poll(
    header: EraCallHeader,
    handle: EraSessionHandle,
    out_buffer: *mut EraOwnedBuffer,
) -> EraStatus {
    ffi_status(|| {
        if !valid_header::<EraOwnedBuffer>(header) || out_buffer.is_null() {
            return EraStatus::InvalidArgument;
        }
        let mut registry = lock_registry();
        let Some(record) = registry.sessions.get_mut(&handle.value) else {
            return EraStatus::InvalidHandle;
        };
        let Some(bytes) = record.runtime.poll_envelope() else {
            return EraStatus::Empty;
        };
        write_owned_buffer(&mut registry, bytes, out_buffer)
    })
}

extern "C" fn session_destroy(header: EraCallHeader, handle: EraSessionHandle) -> EraStatus {
    ffi_status(|| {
        if !valid_header::<EraCallHeader>(header) {
            return EraStatus::AbiMismatch;
        }
        // Runtime destruction may cancel/join background cache work and release a large VM.
        // Never retain the process-wide registry lock while running those destructors, or an
        // unrelated session create/drive call would be serialized behind cleanup.
        let removed = {
            let mut registry = lock_registry();
            registry.sessions.remove(&handle.value)
        };
        if removed.is_some() {
            EraStatus::Ok
        } else {
            EraStatus::InvalidHandle
        }
    })
}

extern "C" fn release_buffer(header: EraCallHeader, buffer: EraOwnedBuffer) -> EraStatus {
    ffi_status(|| {
        if !valid_header::<EraOwnedBuffer>(header) {
            return EraStatus::AbiMismatch;
        }
        if lock_registry().buffers.remove(&buffer.token).is_some() {
            EraStatus::Ok
        } else {
            EraStatus::InvalidArgument
        }
    })
}

extern "C" fn last_error(
    header: EraCallHeader,
    handle: EraSessionHandle,
    out_buffer: *mut EraOwnedBuffer,
) -> EraStatus {
    ffi_status(|| {
        if !valid_header::<EraOwnedBuffer>(header) || out_buffer.is_null() {
            return EraStatus::InvalidArgument;
        }
        let mut registry = lock_registry();
        let Some(record) = registry.sessions.get(&handle.value) else {
            return EraStatus::InvalidHandle;
        };
        let bytes = record.last_error.as_bytes().to_vec();
        write_owned_buffer(&mut registry, bytes, out_buffer)
    })
}

fn write_owned_buffer(
    registry: &mut Registry,
    bytes: Vec<u8>,
    out_buffer: *mut EraOwnedBuffer,
) -> EraStatus {
    let token = registry.next_buffer;
    registry.next_buffer = registry.next_buffer.saturating_add(1);
    let mut bytes = bytes.into_boxed_slice();
    let output = EraOwnedBuffer {
        data: bytes.as_mut_ptr(),
        len: bytes.len(),
        token,
    };
    registry.buffers.insert(token, bytes);
    // SAFETY: callers reach this helper only after validating out_buffer.
    unsafe { out_buffer.write(output) };
    EraStatus::Ok
}

fn valid_header<T>(header: EraCallHeader) -> bool {
    header.abi_version.major == ERA_RUNTIME_ABI_VERSION.major
        && header.struct_size >= u32::try_from(std::mem::size_of::<T>()).unwrap_or(u32::MAX)
}

fn lock_registry() -> std::sync::MutexGuard<'static, Registry> {
    registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn ffi_status(operation: impl FnOnce() -> EraStatus) -> EraStatus {
    catch_unwind(AssertUnwindSafe(operation)).unwrap_or(EraStatus::InternalError)
}

const fn drive_state(state: RuntimeDriveState) -> EraDriveState {
    match state {
        RuntimeDriveState::Idle => EraDriveState::Idle,
        RuntimeDriveState::MoreWork => EraDriveState::MoreWork,
        RuntimeDriveState::OutputReady => EraDriveState::OutputReady,
        RuntimeDriveState::Stopped => EraDriveState::Stopped,
        RuntimeDriveState::Faulted => EraDriveState::Faulted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_a_complete_v3_function_table() {
        let mut api = std::mem::MaybeUninit::<EraRuntimeApi>::uninit();
        assert_eq!(
            // SAFETY: MaybeUninit provides writable storage for the complete table.
            unsafe { era_runtime_get_api(ERA_RUNTIME_ABI_VERSION, api.as_mut_ptr()) },
            EraStatus::Ok
        );
        // SAFETY: a successful call initialized the complete table.
        let api = unsafe { api.assume_init() };
        assert_eq!(api.abi_version, ERA_RUNTIME_ABI_VERSION);
        assert!(!api.session_create.is_null());
        assert!(!api.session_submit.is_null());
        assert!(!api.session_drive.is_null());
        assert!(!api.session_poll.is_null());
        assert!(!api.session_destroy.is_null());
        assert!(!api.release_buffer.is_null());
        assert!(!api.reserved[0].is_null());
        assert!(!api.reserved[1].is_null());
        assert!(!api.reserved[2].is_null());
    }

    #[test]
    fn c_boundary_creates_drives_and_destroys_an_isolated_session() {
        let options = EraCreateOptions {
            debug_scope_mask: era_runtime_ffi::ERA_DEBUG_SCOPE_EXECUTION_READ,
            ..EraCreateOptions::default()
        };
        let mut handle = EraSessionHandle::default();
        assert_eq!(
            session_create(options.header, &raw const options, &raw mut handle),
            EraStatus::Ok
        );
        assert_ne!(handle.value, 0);

        let drive_options = EraDriveOptions::default();
        let mut result = EraDriveResult::default();
        assert_eq!(
            session_drive(
                drive_options.header,
                handle,
                &raw const drive_options,
                &raw mut result,
            ),
            EraStatus::Ok
        );
        assert_eq!(result.state, EraDriveState::Idle);
        assert_eq!(
            session_destroy(EraCallHeader::for_type::<EraCallHeader>(), handle),
            EraStatus::Ok
        );
    }

    #[test]
    fn c_boundary_rejects_unknown_debug_scope_bits() {
        let options = EraCreateOptions {
            debug_scope_mask: 1 << 63,
            ..EraCreateOptions::default()
        };
        let mut handle = EraSessionHandle::default();
        assert_eq!(
            session_create(options.header, &raw const options, &raw mut handle),
            EraStatus::InvalidArgument
        );
    }
}
