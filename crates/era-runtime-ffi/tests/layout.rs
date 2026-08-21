use std::mem::{align_of, offset_of, size_of};

use era_runtime_ffi::{
    ERA_RUNTIME_ABI_VERSION, ERA_RUNTIME_GET_API_SYMBOL, EraAbiVersion, EraByteSlice,
    EraCallHeader, EraCreateOptions, EraDriveOptions, EraDriveResult, EraDriveState,
    EraOwnedBuffer, EraProjectProgress, EraProjectProgressStage, EraRuntimeApi, EraSessionHandle,
    EraStatus,
};

#[test]
fn abi_types_have_exact_size_alignment_and_offsets() {
    assert_layout::<EraAbiVersion>(4, 2);
    assert_eq!(offset_of!(EraAbiVersion, major), 0);
    assert_eq!(offset_of!(EraAbiVersion, minor), 2);

    assert_layout::<EraCallHeader>(8, 4);
    assert_eq!(offset_of!(EraCallHeader, struct_size), 0);
    assert_eq!(offset_of!(EraCallHeader, abi_version), 4);
    assert_layout::<EraSessionHandle>(8, 8);

    #[cfg(target_pointer_width = "64")]
    {
        assert_layout::<EraByteSlice>(16, 8);
        assert_eq!(offset_of!(EraByteSlice, data), 0);
        assert_eq!(offset_of!(EraByteSlice, len), 8);
        assert_layout::<EraOwnedBuffer>(24, 8);
        assert_eq!(offset_of!(EraOwnedBuffer, data), 0);
        assert_eq!(offset_of!(EraOwnedBuffer, len), 8);
        assert_eq!(offset_of!(EraOwnedBuffer, token), 16);

        assert_layout::<EraCreateOptions>(48, 8);
        assert_eq!(offset_of!(EraCreateOptions, header), 0);
        assert_eq!(offset_of!(EraCreateOptions, debug_scope_mask), 8);
        assert_eq!(offset_of!(EraCreateOptions, reserved), 16);

        assert_layout::<EraDriveOptions>(24, 8);
        assert_eq!(offset_of!(EraDriveOptions, header), 0);
        assert_eq!(offset_of!(EraDriveOptions, maximum_vm_instructions), 8);
        assert_eq!(offset_of!(EraDriveOptions, maximum_runtime_transitions), 16);
        assert_eq!(offset_of!(EraDriveOptions, reserved), 20);

        assert_layout::<EraDriveResult>(32, 8);
        assert_eq!(offset_of!(EraDriveResult, header), 0);
        assert_eq!(offset_of!(EraDriveResult, state), 8);
        assert_eq!(offset_of!(EraDriveResult, vm_instructions), 16);
        assert_eq!(offset_of!(EraDriveResult, runtime_transitions), 24);
        assert_eq!(offset_of!(EraDriveResult, queued_envelopes), 28);

        assert_layout::<EraProjectProgress>(32, 8);
        assert_eq!(offset_of!(EraProjectProgress, header), 0);
        assert_eq!(offset_of!(EraProjectProgress, stage), 8);
        assert_eq!(offset_of!(EraProjectProgress, completed), 16);
        assert_eq!(offset_of!(EraProjectProgress, total), 24);

        assert_layout::<EraRuntimeApi>(144, 8);
        assert_eq!(offset_of!(EraRuntimeApi, struct_size), 0);
        assert_eq!(offset_of!(EraRuntimeApi, abi_version), 4);
        assert_eq!(offset_of!(EraRuntimeApi, implementation_name), 8);
        assert_eq!(offset_of!(EraRuntimeApi, implementation_context), 16);
        assert_eq!(offset_of!(EraRuntimeApi, session_create), 24);
        assert_eq!(offset_of!(EraRuntimeApi, session_submit), 32);
        assert_eq!(offset_of!(EraRuntimeApi, session_drive), 40);
        assert_eq!(offset_of!(EraRuntimeApi, session_poll), 48);
        assert_eq!(offset_of!(EraRuntimeApi, session_destroy), 56);
        assert_eq!(offset_of!(EraRuntimeApi, release_buffer), 64);
        assert_eq!(offset_of!(EraRuntimeApi, last_error), 72);
        assert_eq!(offset_of!(EraRuntimeApi, reserved), 80);
    }

    assert_eq!(
        EraCreateOptions::default().header.struct_size as usize,
        size_of::<EraCreateOptions>()
    );
    assert_eq!(
        EraDriveOptions::default().header.struct_size as usize,
        size_of::<EraDriveOptions>()
    );
}

#[test]
fn every_c_enum_discriminant_is_fixed() {
    assert_eq!(
        [
            EraStatus::Ok as u32,
            EraStatus::Empty as u32,
            EraStatus::Busy as u32,
            EraStatus::InvalidArgument as u32,
            EraStatus::AbiMismatch as u32,
            EraStatus::InvalidHandle as u32,
            EraStatus::ResourceLimit as u32,
            EraStatus::InternalError as u32,
        ],
        [0, 1, 2, 3, 4, 5, 6, 7]
    );
    assert_eq!(
        [
            EraDriveState::Idle as u32,
            EraDriveState::MoreWork as u32,
            EraDriveState::OutputReady as u32,
            EraDriveState::Stopped as u32,
            EraDriveState::Faulted as u32,
        ],
        [0, 1, 2, 3, 4]
    );
    assert_eq!(
        [
            EraProjectProgressStage::Scanning as u32,
            EraProjectProgressStage::Normalizing as u32,
            EraProjectProgressStage::LoadingData as u32,
            EraProjectProgressStage::Parsing as u32,
            EraProjectProgressStage::Analyzing as u32,
            EraProjectProgressStage::Compiling as u32,
            EraProjectProgressStage::Validating as u32,
            EraProjectProgressStage::Finalizing as u32,
            EraProjectProgressStage::Preparing as u32,
            EraProjectProgressStage::Packaging as u32,
            EraProjectProgressStage::CacheParsing as u32,
            EraProjectProgressStage::CacheDecoding as u32,
            EraProjectProgressStage::CacheValidating as u32,
            EraProjectProgressStage::InitializingMemory as u32,
            EraProjectProgressStage::IndexingProgram as u32,
        ],
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]
    );
}

#[test]
fn checked_header_matches_version_function_table_and_reserved_slots() {
    let header = include_str!("../include/era_runtime.h");
    assert!(header.contains("#define ERA_RUNTIME_ABI_MAJOR 3u"));
    assert!(header.contains("#define ERA_RUNTIME_ABI_MINOR 9u"));
    assert!(header.contains(ERA_RUNTIME_GET_API_SYMBOL));
    assert_eq!(
        ERA_RUNTIME_ABI_VERSION,
        EraAbiVersion { major: 3, minor: 9 }
    );

    for field in [
        "session_create",
        "session_submit",
        "session_drive",
        "session_poll",
        "session_destroy",
        "release_buffer",
        "last_error",
    ] {
        assert!(header.contains(field), "C function table omitted {field}");
    }
    assert!(header.contains("void *reserved[8]"));
    for slot in 0..8 {
        assert!(
            header.contains(&format!("reserved[{slot}]")),
            "C header omitted the ABI contract for reserved[{slot}]"
        );
    }
    assert_eq!(
        size_of::<[*mut std::ffi::c_void; 8]>(),
        usize::BITS as usize
    );
}

fn assert_layout<T>(size: usize, alignment: usize) {
    assert_eq!(size_of::<T>(), size);
    assert_eq!(align_of::<T>(), alignment);
}
