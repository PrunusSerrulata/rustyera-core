/// Access to trusted source argument presence. Slice-only static callers keep their
/// existing behavior; dynamic requests distinguish omission from a real Integer MIN.
pub(in super::super) trait HostArgumentValues {
    fn argument(&self, index: usize) -> Option<&VmValue>;
    fn source_omitted(&self, _index: usize) -> bool {
        false
    }
}
impl HostArgumentValues for [VmValue] {
    fn argument(&self, index: usize) -> Option<&VmValue> {
        self.get(index)
    }
}
impl HostArgumentValues for Vec<VmValue> {
    fn argument(&self, index: usize) -> Option<&VmValue> {
        self.get(index)
    }
}
impl<const N: usize> HostArgumentValues for [VmValue; N] {
    fn argument(&self, index: usize) -> Option<&VmValue> {
        self.get(index)
    }
}
impl HostArgumentValues for VmHostRequest {
    fn argument(&self, index: usize) -> Option<&VmValue> {
        VmHostRequest::argument(self, index)
    }
    fn source_omitted(&self, index: usize) -> bool {
        self.omitted_arguments.binary_search(&index).is_ok()
    }
}
fn missing_host_argument(
    arguments: &(impl HostArgumentValues + ?Sized),
    index: usize,
    message: String,
) -> RuntimeError {
    if arguments.source_omitted(index) {
        // Classified at the actual required getter; optional consumers use argument()
        // and their operation-specific defaults instead of this failure constructor.
        RuntimeError::Script {
            kind: erabasic_vm::ScriptFaultKind::Operation,
            message: format!(
                "source argument {} is null at a required Host getter",
                index + 1
            ),
        }
    } else {
        RuntimeError::Internal(message)
    }
}

pub(in super::super) fn integer_argument_value(
    arguments: &(impl HostArgumentValues + ?Sized),
    index: usize,
) -> Result<i64, RuntimeError> {
    match arguments.argument(index) {
        Some(VmValue::Integer(value)) => Ok(*value),
        _ => Err(missing_host_argument(
            arguments,
            index,
            format!("host argument {} must be integer", index + 1),
        )),
    }
}

pub(in super::super) fn color_argument_value(arguments: &[VmValue]) -> Result<i64, &'static str> {
    match arguments {
        [VmValue::Integer(rgb)] => Ok(rgb & 0xff_ffff),
        [
            VmValue::Integer(red),
            VmValue::Integer(green),
            VmValue::Integer(blue),
        ] => {
            if !(0..=255).contains(red) || !(0..=255).contains(green) || !(0..=255).contains(blue) {
                return Err("color channels must be between 0 and 255");
            }
            Ok((red << 16) | (green << 8) | blue)
        }
        _ => Err("color requires one packed RGB value or three R,G,B values"),
    }
}

pub(in super::super) fn vm_place(value: &VmValue) -> Option<PlaceDescriptor> {
    match value {
        VmValue::IntegerPlace(place) | VmValue::StringPlace(place) => Some(place.as_ref().clone()),
        VmValue::Integer(_) | VmValue::String(_) => None,
    }
}

pub(in super::super) fn i32_argument_value(
    arguments: &(impl HostArgumentValues + ?Sized),
    index: usize,
) -> Result<i32, RuntimeError> {
    i32::try_from(integer_argument_value(arguments, index)?).map_err(|_| RuntimeError::Script {
        kind: erabasic_vm::ScriptFaultKind::Bounds,
        message: format!(
            "host argument {} must fit a signed 32-bit drawing coordinate",
            index + 1
        ),
    })
}

pub(in super::super) fn checked_argb(value: i64) -> Result<i64, RuntimeError> {
    if (0..=i64::from(u32::MAX)).contains(&value) {
        Ok(value)
    } else {
        Err(RuntimeError::Script {
            kind: erabasic_vm::ScriptFaultKind::Argument,
            message: "graphics ARGB value must fit an unsigned 32-bit value".into(),
        })
    }
}

// Preserve only a VM failure whose source category was already established by
// a trusted read. Implicit storage/type/state failures cannot choose Script by text/code.
pub(in super::super) fn runtime_script_read_error(error: erabasic_vm::VmError) -> RuntimeError {
    match error {
        erabasic_vm::VmError::ScriptFailure(failure) => match failure.category {
            erabasic_vm::FaultCategory::Script(kind) => RuntimeError::Script {
                kind,
                message: failure.message,
            },
            _ => RuntimeError::Internal(failure.to_string()),
        },
        error => RuntimeError::Internal(error.to_string()),
    }
}

pub(in super::super) fn read_color_matrix(
    vm: &RuntimeVm,
    fiber: erabasic_vm::FiberId,
    value: &VmValue,
) -> Result<Vec<i64>, RuntimeError> {
    let Some(mut place) = vm_place(value) else {
        return Err(RuntimeError::Internal(
            "graphics color matrix must be an integer array place".into(),
        ));
    };
    if place.indices.len() < 2 {
        return Err(RuntimeError::Internal(
            "graphics color matrix must have at least two dimensions".into(),
        ));
    }
    read_color_matrix_place(vm, fiber, &mut place).map(|matrix| matrix.to_vec())
}

fn read_color_matrix_place(
    vm: &RuntimeVm,
    fiber: erabasic_vm::FiberId,
    place: &mut PlaceDescriptor,
) -> Result<[i64; 25], RuntimeError> {
    let row = place.indices.len() - 2;
    let column = place.indices.len() - 1;
    let base_row = place.indices[row];
    let base_column = place.indices[column];
    let mut matrix = [0_i64; 25];
    for y in 0..5 {
        for x in 0..5 {
            place.indices[row] = base_row.checked_add(y).ok_or_else(|| {
                RuntimeError::Internal("graphics color matrix row index overflowed".into())
            })?;
            place.indices[column] = base_column.checked_add(x).ok_or_else(|| {
                RuntimeError::Internal("graphics color matrix column index overflowed".into())
            })?;
            let VmValue::Integer(value) = vm
                .read_host_place(fiber, place)
                .map_err(runtime_script_read_error)?
            else {
                return Err(RuntimeError::Internal(
                    "graphics color matrix contains a non-integer value".into(),
                ));
            };
            matrix[usize::try_from(y * 5 + x).expect("5x5 matrix index fits usize")] = value;
        }
    }
    Ok(matrix)
}

pub(in super::super) fn read_named_color_matrix(
    vm: &RuntimeVm,
    fiber: erabasic_vm::FiberId,
    name: &str,
    origin: [u64; 3],
) -> Option<Box<[i64; 25]>> {
    let global = vm.vm().global_by_name(name)?;
    if global.value_type != erabasic_bytecode::BytecodeType::Integer {
        return None;
    }
    let (character, indices) = match (global.storage, global.dimensions.len()) {
        (erabasic_bytecode::BytecodeStorage::Character, 2) => {
            (Some(origin[0]), vec![origin[1], origin[2]])
        }
        (erabasic_bytecode::BytecodeStorage::Character, _) => return None,
        (_, 2) => (None, vec![origin[0], origin[1]]),
        (_, 3) => (None, origin.to_vec()),
        _ => return None,
    };
    let mut place = PlaceDescriptor {
        variable: global.key,
        backing: None,
        indices,
        character,
        fiber: None,
        frame: None,
    };
    read_color_matrix_place(vm, fiber, &mut place)
        .ok()
        .map(Box::new)
}

pub(in super::super) fn integer_value_or_zero(value: &VmValue) -> i64 {
    match value {
        VmValue::Integer(value) => *value,
        _ => 0,
    }
}

pub(in super::super) fn string_argument_value<'a>(
    arguments: &'a (impl HostArgumentValues + ?Sized),
    index: usize,
    command: &str,
) -> Result<&'a str, RuntimeError> {
    match arguments.argument(index) {
        Some(VmValue::String(value)) => Ok(value),
        _ => Err(missing_host_argument(
            arguments,
            index,
            format!("{command} argument {} must be string", index + 1),
        )),
    }
}

pub(in super::super) fn save_slot_argument(
    arguments: &(impl HostArgumentValues + ?Sized),
    index: usize,
    command: &str,
) -> Result<u32, RuntimeError> {
    let value = integer_argument_value(arguments, index)?;
    u32::try_from(value)
        .ok()
        .filter(|value| *value <= i32::MAX.cast_unsigned())
        .ok_or_else(|| {
            RuntimeError::Internal(format!(
                "{command} argument {} must be between 0 and {}",
                index + 1,
                i32::MAX
            ))
        })
}

pub(in super::super) fn save_slot_path(slot: u32) -> String {
    format!("save{slot:02}.sav")
}

pub(in super::super) fn parse_save_slot(path: &str) -> Option<u32> {
    path.strip_prefix("save")?
        .strip_suffix(".sav")?
        .parse()
        .ok()
}

pub(in super::super) fn dat_filename(value: &str) -> Result<&str, RuntimeError> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains(['/', '\\', '\0'])
        || value.chars().any(char::is_control)
    {
        return Err(RuntimeError::Internal(
            "DAT name must be one safe relative filename component".into(),
        ));
    }
    Ok(value)
}

pub(in super::super) fn protocol_execution_origin(
    origin: erabasic_vm::VmExecutionOrigin,
) -> era_runtime_protocol::ExecutionOrigin {
    era_runtime_protocol::ExecutionOrigin {
        command: origin.command,
        function: origin.function_name,
        generation: origin.generation.0,
        instruction: origin.instruction,
        source: origin
            .source
            .map(|source| era_runtime_protocol::SourceLocation {
                relative_path: source.relative_path,
                byte_start: source.byte_start,
                byte_end: source.byte_end,
                line: Some(source.line),
                byte_column: Some(source.byte_column),
            }),
    }
}

pub(in super::super) const fn protocol_diagnostic_notification(
    notification: VmDiagnosticNotification,
) -> DiagnosticNotification {
    match notification {
        VmDiagnosticNotification::Default => DiagnosticNotification::Default,
        VmDiagnosticNotification::LogOnly => DiagnosticNotification::LogOnly,
    }
}

pub(in super::super) fn safe_relative_path(value: &str) -> Result<String, RuntimeError> {
    era_runtime_protocol::validate_relative_path(value)
        .map_err(|error| RuntimeError::Internal(error.message))
}

pub(in super::super) fn safe_relative_directory(value: &str) -> Result<String, RuntimeError> {
    if value.is_empty() || value == "." {
        Ok(String::new())
    } else {
        safe_relative_path(value)
    }
}

pub(in super::super) fn text_storage_target(
    value: &VmValue,
) -> Result<(StorageNamespace, String), RuntimeError> {
    match value {
        VmValue::Integer(value) => {
            let index = u32::try_from(*value)
                .ok()
                .filter(|value| *value <= i32::MAX.cast_unsigned())
                .ok_or_else(|| {
                    RuntimeError::Internal(
                        "text file number must be between 0 and 2147483647".into(),
                    )
                })?;
            Ok((StorageNamespace::Save, format!("txt{index:02}.txt")))
        }
        VmValue::String(value) => {
            let mut path = safe_relative_path(value)?;
            if !path
                .rsplit('/')
                .next()
                .is_some_and(|name| name.contains('.'))
            {
                path.push_str(".txt");
            }
            Ok((StorageNamespace::Data, path))
        }
        VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => Err(RuntimeError::Internal(
            "text file target must be an integer or string".into(),
        )),
    }
}

pub(in super::super) fn decode_load_text(bytes: &[u8]) -> Option<String> {
    // LOADTEXT operates on project/user text assets rather than submitted EraBasic
    // sources. Match the reference runtime's BOM-aware Unicode decoding without
    // introducing locale-dependent legacy code pages.
    let text = if let Some(bytes) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        std::str::from_utf8(bytes).ok().map(ToOwned::to_owned)
    } else if let Some(bytes) = bytes.strip_prefix(&[0xff, 0xfe]) {
        decode_utf16_bytes(bytes, u16::from_le_bytes)
    } else if let Some(bytes) = bytes.strip_prefix(&[0xfe, 0xff]) {
        decode_utf16_bytes(bytes, u16::from_be_bytes)
    } else {
        std::str::from_utf8(bytes).ok().map(ToOwned::to_owned)
    }?;
    Some(text.replace('\r', ""))
}

fn decode_utf16_bytes(bytes: &[u8], decode: fn([u8; 2]) -> u16) -> Option<String> {
    let mut chunks = bytes.chunks_exact(2);
    let units = chunks
        .by_ref()
        .map(|chunk| decode([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    chunks
        .remainder()
        .is_empty()
        .then(|| String::from_utf16(&units).ok())?
}
