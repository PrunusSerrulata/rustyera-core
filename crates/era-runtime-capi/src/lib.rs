//! Audited unsafe adapter for the caller-pumped runtime C ABI.
//!
//! All game state remains in safe [`era_runtime`] code. Raw pointers are copied or
//! validated at this boundary and no borrowed frontend memory survives a call.

#![deny(unsafe_op_in_unsafe_fn)]

use std::collections::BTreeMap;
use std::ffi::{c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Mutex, OnceLock};

use era_protocol::{SessionId, decode_canonical};
use era_runtime::{
    ProjectProgressReporter, ProjectProgressStage, RuntimeDriveBudget, RuntimeDriveState,
    RuntimeOptions, RuntimeSession,
};
use era_runtime_ffi::{
    ERA_DEBUG_SCOPE_ALL, ERA_RUNTIME_ABI_VERSION, EraAbiVersion, EraByteSlice, EraCallHeader,
    EraCreateOptions, EraDriveOptions, EraDriveResult, EraDriveState, EraOwnedBuffer,
    EraProjectProgress, EraProjectProgressStage, EraRuntimeApi, EraSessionHandle, EraStatus,
};

mod project;
mod registry;
mod session;

use project::{
    prepare_project_configuration_update, session_allocate_compiled_cache,
    session_commit_compiled_cache, session_decode_project_file,
    session_decode_project_file_frontend, session_set_project_progress,
    session_stage_compiled_cache, session_stage_project_manifest,
};
use registry::*;
use session::{session_create, session_destroy, session_drive, session_poll, session_submit};

static IMPLEMENTATION_NAME: &[u8] = b"RustyEra runtime\0";
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
        let reserved = [
            session_set_project_progress as *const () as *mut c_void,
            session_decode_project_file as *const () as *mut c_void,
            session_decode_project_file_frontend as *const () as *mut c_void,
            session_stage_compiled_cache as *const () as *mut c_void,
            session_allocate_compiled_cache as *const () as *mut c_void,
            session_commit_compiled_cache as *const () as *mut c_void,
            prepare_project_configuration_update as *const () as *mut c_void,
            session_stage_project_manifest as *const () as *mut c_void,
        ];
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

const fn invalid_byte_slice(value: EraByteSlice) -> bool {
    value.data.is_null() && value.len != 0
}

/// Borrow a byte slice after [`invalid_byte_slice`] has rejected a non-empty null pointer.
///
/// # Safety
///
/// For a non-empty value, `data` must remain readable for `len` bytes without concurrent
/// mutation for the lifetime of the returned slice.
unsafe fn borrow_byte_slice(value: &EraByteSlice) -> &[u8] {
    if value.len == 0 {
        &[]
    } else {
        // SAFETY: the caller establishes the pointer validity documented above.
        unsafe { std::slice::from_raw_parts(value.data, value.len) }
    }
}

#[cfg(test)]
mod tests;
