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
    assert!(!api.reserved[7].is_null());
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
fn c_boundary_stages_one_cbor_project_manifest_without_an_envelope() {
    let options = EraCreateOptions::default();
    let mut handle = EraSessionHandle::default();
    assert_eq!(
        session_create(options.header, &raw const options, &raw mut handle),
        EraStatus::Ok
    );
    let manifest = encoded_project_manifest();
    let input = EraByteSlice {
        data: manifest.as_ptr(),
        len: manifest.len(),
    };
    assert_eq!(
        session_stage_project_manifest(EraCallHeader::for_type::<EraCallHeader>(), handle, input,),
        EraStatus::Ok
    );
    assert_eq!(
        session_stage_project_manifest(EraCallHeader::for_type::<EraCallHeader>(), handle, input,),
        EraStatus::Busy
    );
    assert_eq!(
        session_destroy(EraCallHeader::for_type::<EraCallHeader>(), handle),
        EraStatus::Ok
    );
}

#[test]
fn project_manifest_staging_validates_handle_and_limit_before_reading_input() {
    let dangling = EraByteSlice {
        data: std::ptr::dangling(),
        len: usize::MAX,
    };
    assert_eq!(
        session_stage_project_manifest(
            EraCallHeader::for_type::<EraCallHeader>(),
            EraSessionHandle { value: u64::MAX },
            dangling,
        ),
        EraStatus::InvalidHandle
    );
    let options = EraCreateOptions::default();
    let mut handle = EraSessionHandle::default();
    assert_eq!(
        session_create(options.header, &raw const options, &raw mut handle),
        EraStatus::Ok
    );
    assert_eq!(
        session_stage_project_manifest(
            EraCallHeader::for_type::<EraCallHeader>(),
            handle,
            dangling,
        ),
        EraStatus::ResourceLimit
    );
    assert_eq!(
        session_destroy(EraCallHeader::for_type::<EraCallHeader>(), handle),
        EraStatus::Ok
    );
}

#[test]
fn project_manifest_staging_rejects_noncanonical_or_malformed_cbor() {
    let options = EraCreateOptions::default();
    let mut handle = EraSessionHandle::default();
    assert_eq!(
        session_create(options.header, &raw const options, &raw mut handle),
        EraStatus::Ok
    );
    let canonical = encoded_project_manifest();
    let mut trailing = canonical.clone();
    trailing.push(0);
    let mut nonminimal = canonical.clone();
    nonminimal.splice(2..3, [0x18, 0x01]);
    let mut descending = vec![0xa3, 0x01, 0x80, 0x00, 0x01];
    descending.extend_from_slice(&canonical[5..]);
    let invalid = [
        trailing,
        nonminimal,
        descending,
        // Truncated files array.
        vec![0xa3, 0x00, 0x01, 0x01, 0x81],
        // Protocol 35 manifests without an explicit compatibility identity are not accepted.
        vec![0xa2, 0x00, 0x01, 0x01, 0x80],
    ];
    for bytes in invalid {
        assert_eq!(
            session_stage_project_manifest(
                EraCallHeader::for_type::<EraCallHeader>(),
                handle,
                EraByteSlice {
                    data: bytes.as_ptr(),
                    len: bytes.len(),
                },
            ),
            EraStatus::InvalidArgument
        );
    }
    let manifest = encoded_project_manifest();
    assert_eq!(
        session_stage_project_manifest(
            EraCallHeader::for_type::<EraCallHeader>(),
            handle,
            EraByteSlice {
                data: manifest.as_ptr(),
                len: manifest.len(),
            },
        ),
        EraStatus::Ok
    );
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

fn encoded_project_manifest() -> Vec<u8> {
    minicbor::to_vec(era_runtime_protocol::ProjectManifest {
        project_revision: 1,
        files: Vec::new(),
        compatibility: Default::default(),
    })
    .unwrap()
}
