use super::*;
pub(super) extern "C" fn session_stage_project_manifest(
    header: EraCallHeader,
    handle: EraSessionHandle,
    input: EraByteSlice,
) -> EraStatus {
    ffi_status(|| {
        if !valid_header::<EraCallHeader>(header) || invalid_byte_slice(input) {
            return EraStatus::InvalidArgument;
        }
        let maximum_bytes = {
            let registry = lock_registry();
            let Some(record) = registry.sessions.get(&handle.value) else {
                return EraStatus::InvalidHandle;
            };
            usize::try_from(record.runtime.maximum_transfer_bytes()).unwrap_or(usize::MAX)
        };
        if input.len > maximum_bytes {
            if let Some(record) = lock_registry().sessions.get_mut(&handle.value) {
                record.last_error = "project manifest exceeds the negotiated transfer limit".into();
            }
            return EraStatus::ResourceLimit;
        }
        let bytes = if input.len == 0 {
            &[]
        } else {
            // SAFETY: the pointer/length pair was validated and is borrowed only while this
            // synchronous entry point decodes the manifest.
            unsafe { std::slice::from_raw_parts(input.data, input.len) }
        };
        let manifest = match decode_canonical(bytes) {
            Ok(manifest) => manifest,
            Err(error) => {
                if let Some(record) = lock_registry().sessions.get_mut(&handle.value) {
                    record.last_error = error.to_string();
                    return EraStatus::InvalidArgument;
                }
                return EraStatus::InvalidHandle;
            }
        };
        let mut registry = lock_registry();
        let Some(record) = registry.sessions.get_mut(&handle.value) else {
            return EraStatus::InvalidHandle;
        };
        match record.runtime.stage_project_manifest(manifest) {
            Ok(()) => EraStatus::Ok,
            Err(era_runtime::RuntimeError::Busy(message)) => {
                record.last_error = message.into();
                EraStatus::Busy
            }
            Err(error) => {
                record.last_error = error.to_string();
                EraStatus::InternalError
            }
        }
    })
}

pub(super) extern "C" fn session_stage_compiled_cache(
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

pub(super) extern "C" fn session_allocate_compiled_cache(
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

pub(super) extern "C" fn session_commit_compiled_cache(
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

pub(super) extern "C" fn session_decode_project_file(
    header: EraCallHeader,
    handle: EraSessionHandle,
    input: EraByteSlice,
    out_buffer: *mut EraOwnedBuffer,
) -> EraStatus {
    decode_project_file_buffer(header, handle, input, out_buffer, false)
}

pub(super) extern "C" fn session_decode_project_file_frontend(
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

pub(super) extern "C" fn prepare_project_configuration_update(
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

pub(super) extern "C" fn session_set_project_progress(
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
        ProjectProgressStage::CacheParsing => EraProjectProgressStage::CacheParsing,
        ProjectProgressStage::CacheDecoding => EraProjectProgressStage::CacheDecoding,
        ProjectProgressStage::CacheValidating => EraProjectProgressStage::CacheValidating,
    }
}
