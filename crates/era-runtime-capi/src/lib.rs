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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BufferPurpose {
    Detached,
    CompiledCache(EraSessionHandle),
}

struct RegisteredBuffer {
    bytes: Vec<u8>,
    purpose: BufferPurpose,
}

struct Registry {
    next_handle: u64,
    next_buffer: u64,
    sessions: BTreeMap<u64, SessionRecord>,
    buffers: BTreeMap<u64, RegisteredBuffer>,
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
        reserved[3] = session_stage_compiled_cache as *const () as *mut c_void;
        reserved[4] = session_allocate_compiled_cache as *const () as *mut c_void;
        reserved[5] = session_commit_compiled_cache as *const () as *mut c_void;
        reserved[6] = prepare_project_configuration_update as *const () as *mut c_void;
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

extern "C" fn session_stage_compiled_cache(
    header: EraCallHeader,
    handle: EraSessionHandle,
    input: EraByteSlice,
    out_transfer_id: *mut u64,
) -> EraStatus {
    ffi_status(|| {
        if !valid_header::<u64>(header)
            || out_transfer_id.is_null()
            || (input.data.is_null() && input.len != 0)
        {
            return EraStatus::InvalidArgument;
        }
        let mut registry = lock_registry();
        let Some(record) = registry.sessions.get_mut(&handle.value) else {
            return EraStatus::InvalidHandle;
        };
        if u64::try_from(input.len).unwrap_or(u64::MAX) > record.runtime.maximum_transfer_bytes() {
            record.last_error =
                "compiled project cache exceeds the negotiated transfer limit".into();
            return EraStatus::ResourceLimit;
        }
        let mut bytes = Vec::new();
        if bytes.try_reserve_exact(input.len).is_err() {
            record.last_error = "cannot allocate the compiled project cache staging buffer".into();
            return EraStatus::ResourceLimit;
        }
        if input.len != 0 {
            // SAFETY: the non-empty pointer/length pair was validated and is copied into the
            // reserved owned buffer before the call returns. No frontend memory is retained.
            bytes.extend_from_slice(unsafe { std::slice::from_raw_parts(input.data, input.len) });
        }
        stage_owned_compiled_cache(record, bytes, out_transfer_id)
    })
}

extern "C" fn session_allocate_compiled_cache(
    header: EraCallHeader,
    handle: EraSessionHandle,
    len: usize,
    out_buffer: *mut EraOwnedBuffer,
) -> EraStatus {
    ffi_status(|| {
        if !valid_header::<EraOwnedBuffer>(header) || out_buffer.is_null() {
            return EraStatus::InvalidArgument;
        }
        let maximum = {
            let registry = lock_registry();
            let Some(record) = registry.sessions.get(&handle.value) else {
                return EraStatus::InvalidHandle;
            };
            record.runtime.maximum_transfer_bytes()
        };
        if u64::try_from(len).unwrap_or(u64::MAX) > maximum {
            if let Some(record) = lock_registry().sessions.get_mut(&handle.value) {
                record.last_error =
                    "compiled project cache exceeds the negotiated transfer limit".into();
            }
            return EraStatus::ResourceLimit;
        }
        let mut bytes = Vec::new();
        if bytes.try_reserve_exact(len).is_err() {
            if let Some(record) = lock_registry().sessions.get_mut(&handle.value) {
                record.last_error =
                    "cannot allocate the compiled project cache staging buffer".into();
            }
            return EraStatus::ResourceLimit;
        }
        bytes.resize(len, 0);
        let mut registry = lock_registry();
        if !registry.sessions.contains_key(&handle.value) {
            return EraStatus::InvalidHandle;
        }
        write_registered_buffer(
            &mut registry,
            bytes,
            BufferPurpose::CompiledCache(handle),
            out_buffer,
        )
    })
}

extern "C" fn session_commit_compiled_cache(
    header: EraCallHeader,
    handle: EraSessionHandle,
    buffer: EraOwnedBuffer,
    out_transfer_id: *mut u64,
) -> EraStatus {
    ffi_status(|| {
        if !valid_header::<u64>(header) || out_transfer_id.is_null() {
            return EraStatus::InvalidArgument;
        }
        let mut registry = lock_registry();
        if !registry.sessions.contains_key(&handle.value) {
            return EraStatus::InvalidHandle;
        }
        let Some(registered) = registry.buffers.get(&buffer.token) else {
            return EraStatus::InvalidArgument;
        };
        if registered.purpose != BufferPurpose::CompiledCache(handle)
            || !registered_buffer_matches(registered, &buffer)
        {
            return EraStatus::InvalidArgument;
        }
        // A structurally valid commit consumes the writable allocation even if Runtime rejects
        // it. This makes ownership unambiguous across the FFI boundary and prevents a caller from
        // mutating bytes after Runtime has inspected them.
        let registered = registry
            .buffers
            .remove(&buffer.token)
            .expect("registered buffer was just validated");
        let bytes = registered.bytes;
        let record = registry
            .sessions
            .get_mut(&handle.value)
            .expect("session was just validated");
        stage_owned_compiled_cache(record, bytes, out_transfer_id)
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

extern "C" fn prepare_project_configuration_update(
    header: EraCallHeader,
    handle: EraSessionHandle,
    project_file: EraByteSlice,
    expected_digest: EraByteSlice,
    contents: EraByteSlice,
    out_buffer: *mut EraOwnedBuffer,
) -> EraStatus {
    ffi_status(|| {
        if !valid_header::<EraOwnedBuffer>(header)
            || out_buffer.is_null()
            || invalid_byte_slice(project_file)
            || invalid_byte_slice(expected_digest)
            || invalid_byte_slice(contents)
        {
            return EraStatus::InvalidArgument;
        }
        let maximum_bytes = {
            let registry = lock_registry();
            let Some(record) = registry.sessions.get(&handle.value) else {
                return EraStatus::InvalidHandle;
            };
            usize::try_from(record.runtime.maximum_transfer_bytes()).unwrap_or(usize::MAX)
        };
        if project_file.len > maximum_bytes
            || expected_digest.len > maximum_bytes
            || contents.len > maximum_bytes
        {
            return EraStatus::ResourceLimit;
        }
        // SAFETY: all pointer/length pairs were validated and each borrow remains scoped to this
        // synchronous ABI call.
        let project_file = if project_file.len == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(project_file.data, project_file.len) }
        };
        // SAFETY: same argument validation and call-scoped lifetime as above.
        let expected_digest = if expected_digest.len == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(expected_digest.data, expected_digest.len) }
        };
        // SAFETY: same argument validation and call-scoped lifetime as above.
        let contents = if contents.len == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(contents.data, contents.len) }
        };
        let contents = match std::str::from_utf8(contents) {
            Ok(contents) => contents,
            Err(_) => return EraStatus::InvalidArgument,
        };
        let update = match era_runtime::prepare_project_configuration_update(
            project_file,
            maximum_bytes,
            expected_digest,
            contents,
        ) {
            Ok(update) => update,
            Err(error) => {
                let mut registry = lock_registry();
                let Some(record) = registry.sessions.get_mut(&handle.value) else {
                    return EraStatus::InvalidHandle;
                };
                record.last_error = error.to_string();
                return EraStatus::InvalidArgument;
            }
        };
        let mut encoded = Vec::with_capacity(8 + update.append.len());
        encoded.extend_from_slice(&update.truncate_to.to_le_bytes());
        encoded.extend_from_slice(&update.append);
        let mut registry = lock_registry();
        if !registry.sessions.contains_key(&handle.value) {
            return EraStatus::InvalidHandle;
        }
        write_owned_buffer(&mut registry, encoded, out_buffer)
    })
}

const fn invalid_byte_slice(value: EraByteSlice) -> bool {
    value.data.is_null() && value.len != 0
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
        ProjectProgressStage::Packaging => EraProjectProgressStage::Packaging,
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
        let mut registry = lock_registry();
        let Some(registered) = registry.buffers.get(&buffer.token) else {
            return EraStatus::InvalidArgument;
        };
        if !registered_buffer_matches(registered, &buffer) {
            return EraStatus::InvalidArgument;
        }
        registry.buffers.remove(&buffer.token);
        EraStatus::Ok
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
    write_registered_buffer(registry, bytes, BufferPurpose::Detached, out_buffer)
}

fn write_registered_buffer(
    registry: &mut Registry,
    bytes: Vec<u8>,
    purpose: BufferPurpose,
    out_buffer: *mut EraOwnedBuffer,
) -> EraStatus {
    let token = registry.next_buffer;
    registry.next_buffer = registry.next_buffer.saturating_add(1);
    let mut bytes = bytes;
    let output = EraOwnedBuffer {
        data: bytes.as_mut_ptr(),
        len: bytes.len(),
        token,
    };
    registry
        .buffers
        .insert(token, RegisteredBuffer { bytes, purpose });
    // SAFETY: callers reach this helper only after validating out_buffer.
    unsafe { out_buffer.write(output) };
    EraStatus::Ok
}

fn registered_buffer_matches(registered: &RegisteredBuffer, buffer: &EraOwnedBuffer) -> bool {
    registered.bytes.as_ptr().cast_mut() == buffer.data && registered.bytes.len() == buffer.len
}

fn stage_owned_compiled_cache(
    record: &mut SessionRecord,
    bytes: Vec<u8>,
    out_transfer_id: *mut u64,
) -> EraStatus {
    match record.runtime.stage_compiled_project_cache(bytes) {
        Ok(transfer_id) => {
            // SAFETY: both ABI entry points reject null and require writable u64 storage.
            unsafe { out_transfer_id.write(transfer_id) };
            EraStatus::Ok
        }
        Err(era_runtime::RuntimeError::ResourceLimit(message)) => {
            record.last_error = message.into();
            EraStatus::ResourceLimit
        }
        Err(era_runtime::RuntimeError::Busy(message)) => {
            record.last_error = message.into();
            EraStatus::Busy
        }
        Err(error) => {
            record.last_error = error.to_string();
            EraStatus::InternalError
        }
    }
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
        assert!(!api.reserved[3].is_null());
        assert!(!api.reserved[4].is_null());
        assert!(!api.reserved[5].is_null());
        assert!(!api.reserved[6].is_null());
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

    #[test]
    fn project_configuration_update_boundary_rejects_invalid_inputs_and_reports_planner_errors() {
        let options = EraCreateOptions::default();
        let mut handle = EraSessionHandle::default();
        assert_eq!(
            session_create(options.header, &raw const options, &raw mut handle),
            EraStatus::Ok
        );
        let header = EraCallHeader::for_type::<EraOwnedBuffer>();
        let empty = EraByteSlice {
            data: std::ptr::null(),
            len: 0,
        };
        let invalid_utf8 = [0xff];
        let invalid_project = b"not a project file";
        let valid_contents = b"[audio]\nvolume = 42\n";
        let mut output = EraOwnedBuffer {
            data: std::ptr::dangling_mut(),
            len: 73,
            token: 74,
        };

        assert_eq!(
            prepare_project_configuration_update(
                header,
                handle,
                EraByteSlice {
                    data: std::ptr::null(),
                    len: 1,
                },
                empty,
                empty,
                &raw mut output,
            ),
            EraStatus::InvalidArgument
        );
        assert_eq!(
            prepare_project_configuration_update(
                header,
                EraSessionHandle { value: u64::MAX },
                EraByteSlice {
                    data: std::ptr::dangling(),
                    len: 1,
                },
                empty,
                empty,
                &raw mut output,
            ),
            EraStatus::InvalidHandle
        );
        assert_eq!(
            prepare_project_configuration_update(
                header,
                handle,
                EraByteSlice {
                    data: invalid_project.as_ptr(),
                    len: invalid_project.len(),
                },
                empty,
                EraByteSlice {
                    data: invalid_utf8.as_ptr(),
                    len: invalid_utf8.len(),
                },
                &raw mut output,
            ),
            EraStatus::InvalidArgument
        );
        assert_eq!(
            prepare_project_configuration_update(
                header,
                handle,
                EraByteSlice {
                    data: invalid_project.as_ptr(),
                    len: invalid_project.len(),
                },
                empty,
                EraByteSlice {
                    data: valid_contents.as_ptr(),
                    len: valid_contents.len(),
                },
                &raw mut output,
            ),
            EraStatus::InvalidArgument
        );
        assert_eq!(output.len, 73);
        assert_eq!(output.token, 74);

        let mut error = EraOwnedBuffer {
            data: std::ptr::null_mut(),
            len: 0,
            token: 0,
        };
        assert_eq!(last_error(header, handle, &raw mut error), EraStatus::Ok);
        // SAFETY: `last_error` returned one live owned buffer.
        let message = unsafe { std::slice::from_raw_parts(error.data, error.len) };
        assert!(String::from_utf8_lossy(message).contains("invalid header"));
        let released = EraOwnedBuffer {
            data: error.data,
            len: error.len,
            token: error.token,
        };
        assert_eq!(release_buffer(header, error), EraStatus::Ok);
        assert_eq!(release_buffer(header, released), EraStatus::InvalidArgument);
        assert_eq!(
            session_destroy(EraCallHeader::for_type::<EraCallHeader>(), handle),
            EraStatus::Ok
        );
    }

    #[test]
    fn c_boundary_stages_a_compiled_cache_without_protocol_chunking() {
        let options = EraCreateOptions::default();
        let mut handle = EraSessionHandle::default();
        assert_eq!(
            session_create(options.header, &raw const options, &raw mut handle),
            EraStatus::Ok
        );
        let bytes = [1_u8, 2, 3, 4];
        let mut transfer_id = 0;

        assert_eq!(
            session_stage_compiled_cache(
                EraCallHeader::for_type::<u64>(),
                handle,
                EraByteSlice {
                    data: bytes.as_ptr(),
                    len: bytes.len(),
                },
                &raw mut transfer_id,
            ),
            EraStatus::Ok
        );
        assert_ne!(transfer_id, 0);
        assert_eq!(
            session_destroy(EraCallHeader::for_type::<EraCallHeader>(), handle),
            EraStatus::Ok
        );
    }

    #[test]
    fn compiled_cache_staging_validates_the_handle_before_reading_input() {
        let mut transfer_id = 73;

        assert_eq!(
            session_stage_compiled_cache(
                EraCallHeader::for_type::<u64>(),
                EraSessionHandle { value: u64::MAX },
                EraByteSlice {
                    data: std::ptr::dangling(),
                    len: 1,
                },
                &raw mut transfer_id,
            ),
            EraStatus::InvalidHandle
        );
        assert_eq!(transfer_id, 73);
    }

    #[test]
    fn compiled_cache_staging_rejects_an_oversized_slice_without_reading_it() {
        let options = EraCreateOptions::default();
        let mut handle = EraSessionHandle::default();
        assert_eq!(
            session_create(options.header, &raw const options, &raw mut handle),
            EraStatus::Ok
        );
        let mut transfer_id = 91;

        assert_eq!(
            session_stage_compiled_cache(
                EraCallHeader::for_type::<u64>(),
                handle,
                EraByteSlice {
                    data: std::ptr::dangling(),
                    len: usize::MAX,
                },
                &raw mut transfer_id,
            ),
            EraStatus::ResourceLimit
        );
        assert_eq!(transfer_id, 91);
        assert_eq!(
            session_destroy(EraCallHeader::for_type::<EraCallHeader>(), handle),
            EraStatus::Ok
        );
    }

    #[test]
    fn compiled_cache_staging_owns_the_input_and_preserves_an_active_transfer() {
        let options = EraCreateOptions::default();
        let mut handle = EraSessionHandle::default();
        assert_eq!(
            session_create(options.header, &raw const options, &raw mut handle),
            EraStatus::Ok
        );
        let mut caller_bytes = [1_u8, 2, 3, 4];
        let mut first_transfer_id = 0;
        assert_eq!(
            session_stage_compiled_cache(
                EraCallHeader::for_type::<u64>(),
                handle,
                EraByteSlice {
                    data: caller_bytes.as_ptr(),
                    len: caller_bytes.len(),
                },
                &raw mut first_transfer_id,
            ),
            EraStatus::Ok
        );
        caller_bytes.fill(9);
        let mut second_transfer_id = 117;
        assert_eq!(
            session_stage_compiled_cache(
                EraCallHeader::for_type::<u64>(),
                handle,
                EraByteSlice {
                    data: caller_bytes.as_ptr(),
                    len: caller_bytes.len(),
                },
                &raw mut second_transfer_id,
            ),
            EraStatus::Busy
        );
        assert_ne!(first_transfer_id, 0);
        assert_eq!(second_transfer_id, 117);
        assert_eq!(
            session_destroy(EraCallHeader::for_type::<EraCallHeader>(), handle),
            EraStatus::Ok
        );
    }

    #[test]
    fn writable_compiled_cache_buffer_commits_without_an_input_copy() {
        let options = EraCreateOptions::default();
        let mut handle = EraSessionHandle::default();
        assert_eq!(
            session_create(options.header, &raw const options, &raw mut handle),
            EraStatus::Ok
        );
        let mut buffer = EraOwnedBuffer {
            data: std::ptr::null_mut(),
            len: 0,
            token: 0,
        };
        assert_eq!(
            session_allocate_compiled_cache(
                EraCallHeader::for_type::<EraOwnedBuffer>(),
                handle,
                4,
                &raw mut buffer,
            ),
            EraStatus::Ok
        );
        // SAFETY: the allocation extension returned four caller-writable bytes.
        unsafe { std::slice::from_raw_parts_mut(buffer.data, buffer.len) }
            .copy_from_slice(&[1, 2, 3, 4]);
        let consumed_buffer = EraOwnedBuffer {
            data: buffer.data,
            len: buffer.len,
            token: buffer.token,
        };
        let mut transfer_id = 0;
        assert_eq!(
            session_commit_compiled_cache(
                EraCallHeader::for_type::<u64>(),
                handle,
                buffer,
                &raw mut transfer_id,
            ),
            EraStatus::Ok
        );
        assert_ne!(transfer_id, 0);
        assert_eq!(
            release_buffer(EraCallHeader::for_type::<EraOwnedBuffer>(), consumed_buffer,),
            EraStatus::InvalidArgument
        );
        assert_eq!(
            session_destroy(EraCallHeader::for_type::<EraCallHeader>(), handle),
            EraStatus::Ok
        );
    }

    #[test]
    fn writable_compiled_cache_buffer_can_be_released_or_rejects_a_forged_shape() {
        let options = EraCreateOptions::default();
        let mut handle = EraSessionHandle::default();
        assert_eq!(
            session_create(options.header, &raw const options, &raw mut handle),
            EraStatus::Ok
        );
        let mut buffer = EraOwnedBuffer {
            data: std::ptr::null_mut(),
            len: 0,
            token: 0,
        };
        assert_eq!(
            session_allocate_compiled_cache(
                EraCallHeader::for_type::<EraOwnedBuffer>(),
                handle,
                4,
                &raw mut buffer,
            ),
            EraStatus::Ok
        );
        let forged_pointer = EraOwnedBuffer {
            data: std::ptr::dangling_mut(),
            len: buffer.len,
            token: buffer.token,
        };
        assert_eq!(
            release_buffer(EraCallHeader::for_type::<EraOwnedBuffer>(), forged_pointer,),
            EraStatus::InvalidArgument
        );
        let forged = EraOwnedBuffer {
            data: buffer.data,
            len: buffer.len + 1,
            token: buffer.token,
        };
        assert_eq!(
            release_buffer(EraCallHeader::for_type::<EraOwnedBuffer>(), forged),
            EraStatus::InvalidArgument
        );
        assert_eq!(
            release_buffer(EraCallHeader::for_type::<EraOwnedBuffer>(), buffer),
            EraStatus::Ok
        );
        assert_eq!(
            session_destroy(EraCallHeader::for_type::<EraCallHeader>(), handle),
            EraStatus::Ok
        );
    }

    #[test]
    fn writable_compiled_cache_allocation_validates_handle_and_limit_first() {
        let mut untouched = EraOwnedBuffer {
            data: std::ptr::dangling_mut(),
            len: 73,
            token: 74,
        };
        assert_eq!(
            session_allocate_compiled_cache(
                EraCallHeader::for_type::<EraOwnedBuffer>(),
                EraSessionHandle { value: u64::MAX },
                usize::MAX,
                &raw mut untouched,
            ),
            EraStatus::InvalidHandle
        );
        assert_eq!(untouched.len, 73);
        assert_eq!(untouched.token, 74);

        let options = EraCreateOptions::default();
        let mut handle = EraSessionHandle::default();
        assert_eq!(
            session_create(options.header, &raw const options, &raw mut handle),
            EraStatus::Ok
        );
        assert_eq!(
            session_allocate_compiled_cache(
                EraCallHeader::for_type::<EraOwnedBuffer>(),
                handle,
                usize::MAX,
                &raw mut untouched,
            ),
            EraStatus::ResourceLimit
        );
        assert_eq!(untouched.len, 73);
        assert_eq!(untouched.token, 74);
        assert_eq!(
            session_destroy(EraCallHeader::for_type::<EraCallHeader>(), handle),
            EraStatus::Ok
        );
    }

    #[test]
    fn writable_compiled_cache_cannot_cross_sessions_and_valid_commit_consumes_it() {
        let options = EraCreateOptions::default();
        let mut first = EraSessionHandle::default();
        let mut second = EraSessionHandle::default();
        assert_eq!(
            session_create(options.header, &raw const options, &raw mut first),
            EraStatus::Ok
        );
        assert_eq!(
            session_create(options.header, &raw const options, &raw mut second),
            EraStatus::Ok
        );
        let mut buffer = EraOwnedBuffer {
            data: std::ptr::null_mut(),
            len: 0,
            token: 0,
        };
        assert_eq!(
            session_allocate_compiled_cache(
                EraCallHeader::for_type::<EraOwnedBuffer>(),
                first,
                4,
                &raw mut buffer,
            ),
            EraStatus::Ok
        );
        let mut transfer_id = 81;
        assert_eq!(
            session_commit_compiled_cache(
                EraCallHeader::for_type::<u64>(),
                second,
                EraOwnedBuffer {
                    data: buffer.data,
                    len: buffer.len,
                    token: buffer.token,
                },
                &raw mut transfer_id,
            ),
            EraStatus::InvalidArgument
        );
        assert_eq!(transfer_id, 81);
        assert_eq!(
            session_commit_compiled_cache(
                EraCallHeader::for_type::<u64>(),
                first,
                buffer,
                &raw mut transfer_id,
            ),
            EraStatus::Ok
        );
        assert_ne!(transfer_id, 81);

        let mut busy_buffer = EraOwnedBuffer {
            data: std::ptr::null_mut(),
            len: 0,
            token: 0,
        };
        assert_eq!(
            session_allocate_compiled_cache(
                EraCallHeader::for_type::<EraOwnedBuffer>(),
                first,
                4,
                &raw mut busy_buffer,
            ),
            EraStatus::Ok
        );
        let consumed_busy_buffer = EraOwnedBuffer {
            data: busy_buffer.data,
            len: busy_buffer.len,
            token: busy_buffer.token,
        };
        let mut busy_transfer_id = 82;
        assert_eq!(
            session_commit_compiled_cache(
                EraCallHeader::for_type::<u64>(),
                first,
                busy_buffer,
                &raw mut busy_transfer_id,
            ),
            EraStatus::Busy
        );
        assert_eq!(busy_transfer_id, 82);
        assert_eq!(
            release_buffer(
                EraCallHeader::for_type::<EraOwnedBuffer>(),
                consumed_busy_buffer,
            ),
            EraStatus::InvalidArgument
        );
        assert_eq!(
            session_destroy(EraCallHeader::for_type::<EraCallHeader>(), first),
            EraStatus::Ok
        );
        assert_eq!(
            session_destroy(EraCallHeader::for_type::<EraCallHeader>(), second),
            EraStatus::Ok
        );
    }
}
