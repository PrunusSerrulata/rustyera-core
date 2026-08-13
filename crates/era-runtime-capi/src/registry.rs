use super::*;

pub(super) struct SessionRecord {
    pub(super) runtime: RuntimeSession,
    pub(super) last_error: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BufferPurpose {
    Detached,
    CompiledCache(EraSessionHandle),
}

pub(super) struct RegisteredBuffer {
    pub(super) bytes: Vec<u8>,
    pub(super) purpose: BufferPurpose,
}

pub(super) struct Registry {
    pub(super) next_handle: u64,
    pub(super) next_buffer: u64,
    pub(super) sessions: BTreeMap<u64, SessionRecord>,
    pub(super) buffers: BTreeMap<u64, RegisteredBuffer>,
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

pub(super) fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}
pub(super) extern "C" fn release_buffer(
    header: EraCallHeader,
    buffer: EraOwnedBuffer,
) -> EraStatus {
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

pub(super) extern "C" fn last_error(
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

pub(super) fn write_owned_buffer(
    registry: &mut Registry,
    bytes: Vec<u8>,
    out_buffer: *mut EraOwnedBuffer,
) -> EraStatus {
    write_registered_buffer(registry, bytes, BufferPurpose::Detached, out_buffer)
}

pub(super) fn write_registered_buffer(
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

pub(super) fn registered_buffer_matches(
    registered: &RegisteredBuffer,
    buffer: &EraOwnedBuffer,
) -> bool {
    registered.bytes.as_ptr().cast_mut() == buffer.data && registered.bytes.len() == buffer.len
}

pub(super) fn stage_owned_compiled_cache(
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

pub(super) fn valid_header<T>(header: EraCallHeader) -> bool {
    header.abi_version.major == ERA_RUNTIME_ABI_VERSION.major
        && header.struct_size >= u32::try_from(std::mem::size_of::<T>()).unwrap_or(u32::MAX)
}

pub(super) fn lock_registry() -> std::sync::MutexGuard<'static, Registry> {
    registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(super) fn ffi_status(operation: impl FnOnce() -> EraStatus) -> EraStatus {
    catch_unwind(AssertUnwindSafe(operation)).unwrap_or(EraStatus::InternalError)
}

pub(super) const fn drive_state(state: RuntimeDriveState) -> EraDriveState {
    match state {
        RuntimeDriveState::Idle => EraDriveState::Idle,
        RuntimeDriveState::MoreWork => EraDriveState::MoreWork,
        RuntimeDriveState::OutputReady => EraDriveState::OutputReady,
        RuntimeDriveState::Stopped => EraDriveState::Stopped,
        RuntimeDriveState::Faulted => EraDriveState::Faulted,
    }
}
