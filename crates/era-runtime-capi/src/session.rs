use super::*;
pub(super) extern "C" fn session_create(
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

pub(super) extern "C" fn session_submit(
    header: EraCallHeader,
    handle: EraSessionHandle,
    input: EraByteSlice,
) -> EraStatus {
    ffi_status(|| {
        if !valid_header::<EraCallHeader>(header) || invalid_byte_slice(input) {
            return EraStatus::InvalidArgument;
        }
        let mut registry = lock_registry();
        let Some(record) = registry.sessions.get_mut(&handle.value) else {
            return EraStatus::InvalidHandle;
        };
        // SAFETY: the pointer/length pair was validated and the runtime decoder does not
        // retain the call-scoped borrow.
        let bytes = unsafe { borrow_byte_slice(&input) };
        match record.runtime.submit_envelope(bytes) {
            Ok(()) => EraStatus::Ok,
            Err(error) => {
                record.last_error = error.to_string();
                EraStatus::InvalidArgument
            }
        }
    })
}

pub(super) extern "C" fn session_drive(
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

pub(super) extern "C" fn session_poll(
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

pub(super) extern "C" fn session_destroy(
    header: EraCallHeader,
    handle: EraSessionHandle,
) -> EraStatus {
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
