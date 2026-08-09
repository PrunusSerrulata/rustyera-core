use std::mem::{align_of, size_of};

use era_runtime_ffi::{
    ERA_RUNTIME_ABI_VERSION, ERA_RUNTIME_GET_API_SYMBOL, EraAbiVersion, EraCallHeader,
    EraCreateOptions, EraDriveOptions, EraOwnedBuffer, EraProjectProgress, EraProjectProgressStage,
    EraRuntimeApi, EraSessionHandle,
};

#[test]
fn abi_headers_and_handles_have_fixed_layouts() {
    assert_eq!(size_of::<EraAbiVersion>(), 4);
    assert_eq!(size_of::<EraCallHeader>(), 8);
    assert_eq!(size_of::<EraSessionHandle>(), 8);
    assert_eq!(align_of::<EraOwnedBuffer>(), align_of::<usize>());
    assert_eq!(size_of::<EraProjectProgress>(), 32);
    assert_eq!(EraProjectProgressStage::Finalizing as u32, 7);
    assert_eq!(EraProjectProgressStage::Preparing as u32, 8);
    assert_eq!(
        EraCreateOptions::default().header.struct_size as usize,
        size_of::<EraCreateOptions>()
    );
    assert_eq!(
        EraDriveOptions::default().header.struct_size as usize,
        size_of::<EraDriveOptions>()
    );
    assert!(size_of::<EraRuntimeApi>() >= size_of::<usize>() * 10);
}

#[test]
fn checked_header_tracks_the_rust_abi_version() {
    let header = include_str!("../include/era_runtime.h");
    assert!(header.contains("#define ERA_RUNTIME_ABI_MAJOR 3u"));
    assert!(header.contains("#define ERA_RUNTIME_ABI_MINOR 6u"));
    assert!(header.contains(ERA_RUNTIME_GET_API_SYMBOL));
    assert_eq!(ERA_RUNTIME_ABI_VERSION.major, 3);
    assert_eq!(ERA_RUNTIME_ABI_VERSION.minor, 6);
}
