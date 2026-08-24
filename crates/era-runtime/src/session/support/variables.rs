#[allow(clippy::wildcard_imports)]
use super::super::*;

pub(in super::super) fn runtime_variable_key(
    vm: &RuntimeVm,
    name: &str,
) -> Result<erabasic_bytecode::SymbolKey, RuntimeError> {
    vm.vm()
        .global_by_name(name)
        .map(|global| global.key)
        .ok_or_else(|| RuntimeError::Internal(format!("system variable {name} is missing")))
}

pub(in super::super) fn runtime_variable_length(vm: &RuntimeVm, name: &str) -> usize {
    vm.vm()
        .global_by_name(name)
        .and_then(|global| global.dimensions.first())
        .and_then(|length| usize::try_from(*length).ok())
        .unwrap_or(0)
}

pub(in super::super) fn table_names(
    vm: &RuntimeVm,
    kind: erabasic_data::NameTableKind,
) -> Vec<Option<String>> {
    vm.vm()
        .artifact()
        .project_data
        .static_data
        .name_tables
        .get(&kind)
        .map_or_else(Vec::new, |table| table.names.clone())
}

pub(in super::super) fn format_named_character_values(
    vm: &RuntimeVm,
    variable: &str,
    table: erabasic_data::NameTableKind,
    character: u64,
    format: u8,
) -> Result<String, RuntimeError> {
    let names = table_names(vm, table);
    let length = names.len().min(runtime_variable_length(vm, variable));
    let mut output = String::new();
    for (index, name) in names.into_iter().take(length).enumerate() {
        let Some(name) = name.filter(|name| !name.is_empty()) else {
            continue;
        };
        let value = read_runtime_integer(
            vm,
            variable,
            &[u64::try_from(index).unwrap_or(u64::MAX)],
            Some(character),
        )?;
        if value == 0 {
            continue;
        }
        match format {
            1 => write!(output, "[{name}]").expect("writing to String cannot fail"),
            2 => write!(output, "{name}{value} ").expect("writing to String cannot fail"),
            _ => write!(output, "{name}LV{value} ").expect("writing to String cannot fail"),
        }
    }
    Ok(output)
}

pub(in super::super) fn format_having_items(vm: &RuntimeVm) -> Result<String, RuntimeError> {
    let names = table_names(vm, erabasic_data::NameTableKind::Item);
    let length = names.len().min(runtime_variable_length(vm, "ITEM"));
    let mut items = String::new();
    for (index, name) in names.into_iter().take(length).enumerate() {
        let Some(name) = name.filter(|name| !name.is_empty()) else {
            continue;
        };
        let count = read_runtime_integer(
            vm,
            "ITEM",
            &[u64::try_from(index).unwrap_or(u64::MAX)],
            None,
        )?;
        if count != 0 {
            write!(items, "{name}({count}) ").expect("writing to String cannot fail");
        }
    }
    if items.is_empty() {
        items.push_str("なし");
    }
    Ok(format!("所持アイテム：{items}"))
}

pub(in super::super) fn format_character_palam(
    vm: &RuntimeVm,
    character: u64,
) -> Result<Vec<String>, RuntimeError> {
    let names = table_names(vm, erabasic_data::NameTableKind::Palam);
    let length = names
        .len()
        .min(runtime_variable_length(vm, "PALAM"))
        .min(100);
    let borders = (1_u64..=4)
        .map(|index| read_runtime_integer(vm, "PALAMLV", &[index], None))
        .collect::<Result<Vec<_>, _>>()?;
    let mut output = Vec::new();
    for (index, name) in names.into_iter().take(length).enumerate() {
        let value = read_runtime_integer(
            vm,
            "PALAM",
            &[u64::try_from(index).unwrap_or(u64::MAX)],
            Some(character),
        )?;
        if value == 0 && name.as_deref().is_none_or(str::is_empty) {
            continue;
        }
        let (mark, border) = if value >= borders[2] {
            ('*', borders[3])
        } else if value >= borders[1] {
            ('>', borders[2])
        } else if value >= borders[0] {
            ('=', borders[1])
        } else {
            ('-', borders[0])
        };
        let filled = if border <= 0 || value >= border {
            10
        } else if value <= 0 {
            0
        } else {
            usize::try_from(value.saturating_mul(10) / border)
                .unwrap_or_default()
                .min(10)
        };
        let bar = format!(
            "{}{}",
            mark.to_string().repeat(filled),
            ".".repeat(10 - filled)
        );
        output.push(format!("{}[{bar}]{value:>6}", name.unwrap_or_default()));
    }
    Ok(output)
}

pub(in super::super) fn format_shop_items(
    vm: &RuntimeVm,
    project: &NormalizedProjectSnapshot,
) -> Result<Vec<(String, i64)>, RuntimeError> {
    let names = table_names(vm, erabasic_data::NameTableKind::Item);
    let prices = &vm.vm().artifact().project_data.static_data.item_prices;
    let length = names
        .len()
        .min(prices.len())
        .min(runtime_variable_length(vm, "ITEMSALES"));
    let mut output = Vec::new();
    for (index, name) in names.into_iter().take(length).enumerate() {
        if read_runtime_integer(
            vm,
            "ITEMSALES",
            &[u64::try_from(index).unwrap_or(u64::MAX)],
            None,
        )? == 0
        {
            continue;
        }
        let price = prices[index];
        let price = if project.money_first {
            format!("{}{price}", project.money_label)
        } else {
            format!("{price}{}", project.money_label)
        };
        output.push((
            format!("[{index}] {}({price})", name.unwrap_or_default()),
            i64::try_from(index).unwrap_or(i64::MAX),
        ));
    }
    Ok(output)
}

pub(in super::super) fn clear_upcheck_arrays(
    vm: &mut RuntimeVm,
    character_scoped: bool,
    character: Option<u64>,
) -> Result<(), RuntimeError> {
    let variables = if character_scoped {
        ["CUP", "CDOWN"]
    } else {
        ["UP", "DOWN"]
    };
    let mut writes = Vec::new();
    for variable in variables {
        let key = runtime_variable_key(vm, variable)?;
        for index in 0..runtime_variable_length(vm, variable) {
            writes.push(VmRuntimeWrite {
                variable: key,
                indices: vec![u64::try_from(index).unwrap_or(u64::MAX)],
                character,
                value: VmValue::Integer(0),
            });
        }
    }
    let prepared = vm
        .prepare_runtime_state(VmRuntimeStateTransaction::Mutate {
            writes,
            fills: Vec::new(),
            clear_characters: false,
            add_characters_from_csv: Vec::new(),
        })
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    vm.commit_runtime_state(prepared)
        .map_err(|error| RuntimeError::Internal(error.to_string()))
}

pub(in super::super) fn apply_upcheck(
    vm: &mut RuntimeVm,
    character: u64,
    character_scoped: bool,
) -> Result<Vec<String>, RuntimeError> {
    let (up_name, down_name, delta_character) = if character_scoped {
        ("CUP", "CDOWN", Some(character))
    } else {
        ("UP", "DOWN", None)
    };
    let length = runtime_variable_length(vm, "PALAM")
        .min(runtime_variable_length(vm, up_name))
        .min(runtime_variable_length(vm, down_name));
    let names = table_names(vm, erabasic_data::NameTableKind::Palam);
    let mut writes = Vec::with_capacity(length.saturating_mul(3));
    let mut lines = Vec::new();
    for index in 0..length {
        let coordinate = [u64::try_from(index).unwrap_or(u64::MAX)];
        let current = match read_runtime_integer(vm, "PALAM", &coordinate, Some(character)) {
            Ok(value) => value,
            Err(_error) if character_scoped => return Ok(Vec::new()),
            Err(_) => {
                clear_upcheck_arrays(vm, false, None)?;
                return Ok(Vec::new());
            }
        };
        let up = read_runtime_integer(vm, up_name, &coordinate, delta_character)?;
        let down = read_runtime_integer(vm, down_name, &coordinate, delta_character)?;
        if up > 0 || down > 0 {
            let updated = current.wrapping_add(up).wrapping_sub(down);
            let mut text = format!(
                "{} {current}",
                names.get(index).and_then(Clone::clone).unwrap_or_default()
            );
            if up > 0 {
                write!(text, "+{up}").expect("writing to String cannot fail");
            }
            if down > 0 {
                write!(text, "-{down}").expect("writing to String cannot fail");
            }
            write!(text, "={updated}").expect("writing to String cannot fail");
            lines.push(text);
            writes.push(VmRuntimeWrite {
                variable: runtime_variable_key(vm, "PALAM")?,
                indices: coordinate.to_vec(),
                character: Some(character),
                value: VmValue::Integer(updated),
            });
        }
        for variable in [up_name, down_name] {
            writes.push(VmRuntimeWrite {
                variable: runtime_variable_key(vm, variable)?,
                indices: coordinate.to_vec(),
                character: delta_character,
                value: VmValue::Integer(0),
            });
        }
    }
    let prepared = vm
        .prepare_runtime_state(VmRuntimeStateTransaction::Mutate {
            writes,
            fills: Vec::new(),
            clear_characters: false,
            add_characters_from_csv: Vec::new(),
        })
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    vm.commit_runtime_state(prepared)
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    Ok(lines)
}

pub(in super::super) fn integer_argument_value(
    arguments: &[VmValue],
    index: usize,
) -> Result<i64, RuntimeError> {
    match arguments.get(index) {
        Some(VmValue::Integer(value)) => Ok(*value),
        _ => Err(RuntimeError::Internal(format!(
            "host argument {} must be integer",
            index + 1
        ))),
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
    arguments: &[VmValue],
    index: usize,
) -> Result<i32, RuntimeError> {
    i32::try_from(integer_argument_value(arguments, index)?).map_err(|_| {
        RuntimeError::Internal(format!(
            "host argument {} must fit a signed 32-bit drawing coordinate",
            index + 1
        ))
    })
}

pub(in super::super) fn checked_argb(value: i64) -> Result<i64, RuntimeError> {
    if (0..=i64::from(u32::MAX)).contains(&value) {
        Ok(value)
    } else {
        Err(RuntimeError::Internal(
            "graphics ARGB value must fit an unsigned 32-bit value".into(),
        ))
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
    let row = place.indices.len() - 2;
    let column = place.indices.len() - 1;
    let base_row = place.indices[row];
    let base_column = place.indices[column];
    let mut matrix = Vec::with_capacity(25);
    for y in 0..5 {
        for x in 0..5 {
            place.indices[row] = base_row.saturating_add(y);
            place.indices[column] = base_column.saturating_add(x);
            let VmValue::Integer(value) = vm
                .read_host_place(fiber, &place)
                .map_err(|error| RuntimeError::Internal(error.to_string()))?
            else {
                return Err(RuntimeError::Internal(
                    "graphics color matrix contains a non-integer value".into(),
                ));
            };
            matrix.push(value);
        }
    }
    Ok(matrix)
}

pub(in super::super) fn integer_value_or_zero(value: &VmValue) -> i64 {
    match value {
        VmValue::Integer(value) => *value,
        _ => 0,
    }
}

pub(in super::super) fn string_argument_value<'a>(
    arguments: &'a [VmValue],
    index: usize,
    command: &str,
) -> Result<&'a str, RuntimeError> {
    match arguments.get(index) {
        Some(VmValue::String(value)) => Ok(value),
        _ => Err(RuntimeError::Internal(format!(
            "{command} argument {} must be string",
            index + 1
        ))),
    }
}

pub(in super::super) fn save_slot_argument(
    arguments: &[VmValue],
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

pub(in super::super) fn read_runtime_integer(
    vm: &RuntimeVm,
    name: &str,
    indices: &[u64],
    character: Option<u64>,
) -> Result<i64, RuntimeError> {
    let values = vm
        .read_runtime_state(&[erabasic_vm::VmRuntimeRead {
            variable: runtime_variable_key(vm, name)?,
            indices: indices.to_vec(),
            character,
        }])
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    match values.as_slice() {
        [VmValue::Integer(value)] => Ok(*value),
        _ => Err(RuntimeError::Internal(format!(
            "system variable {name} is not integer"
        ))),
    }
}

pub(in super::super) fn read_runtime_string(
    vm: &RuntimeVm,
    name: &str,
) -> Result<String, RuntimeError> {
    let values = vm
        .read_runtime_state(&[erabasic_vm::VmRuntimeRead {
            variable: runtime_variable_key(vm, name)?,
            indices: Vec::new(),
            character: None,
        }])
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    match values.as_slice() {
        [VmValue::String(value)] => Ok(value.clone()),
        _ => Err(RuntimeError::Internal(format!(
            "system variable {name} is not string"
        ))),
    }
}

pub(in super::super) fn write_runtime_integer(
    vm: &mut RuntimeVm,
    name: &str,
    indices: &[u64],
    character: Option<u64>,
    value: i64,
) -> Result<(), RuntimeError> {
    let variable = runtime_variable_key(vm, name)?;
    vm.vm_mut()
        .write_variable(variable, indices, character, VmValue::Integer(value))
        .map_err(|error| RuntimeError::Internal(error.to_string()))
}

pub(in super::super) fn synchronize_line_count(
    presentation: &mut PresentationModel,
    vm: &mut RuntimeVm,
) -> Result<(), RuntimeError> {
    if !presentation.line_count_is_dirty() {
        return Ok(());
    }
    // The analyzer only materializes calculated globals that a project references.
    // A project without LINECOUNT therefore needs no VM cell synchronization.
    if vm.vm().global_by_name("LINECOUNT").is_some() {
        write_runtime_integer(
            vm,
            "LINECOUNT",
            &[],
            None,
            presentation.logical_line_count(),
        )?;
    }
    presentation.mark_line_count_synchronized();
    Ok(())
}

pub(in super::super) fn write_runtime_string(
    vm: &mut RuntimeVm,
    name: &str,
    value: String,
) -> Result<(), RuntimeError> {
    let variable = runtime_variable_key(vm, name)?;
    vm.vm_mut()
        .write_variable(variable, &[], None, VmValue::String(value))
        .map_err(|error| RuntimeError::Internal(error.to_string()))
}

pub(in super::super) fn fill_runtime_variable(
    vm: &mut RuntimeVm,
    name: &str,
    value: VmValue,
    all_characters: bool,
) -> Result<(), RuntimeError> {
    let variable = runtime_variable_key(vm, name)?;
    vm.vm_mut()
        .fill_runtime_variables(&[VmRuntimeFill {
            variable,
            value,
            all_characters,
        }])
        .map_err(|error| RuntimeError::Internal(error.to_string()))
}

pub(in super::super) fn reset_training_state(vm: &mut RuntimeVm) -> Result<(), RuntimeError> {
    let key = |name: &str| runtime_variable_key(vm, name);
    let mut writes = vec![
        VmRuntimeWrite {
            variable: key("ASSIPLAY")?,
            indices: Vec::new(),
            character: None,
            value: VmValue::Integer(0),
        },
        VmRuntimeWrite {
            variable: key("PREVCOM")?,
            indices: Vec::new(),
            character: None,
            value: VmValue::Integer(-1),
        },
        VmRuntimeWrite {
            variable: key("NEXTCOM")?,
            indices: Vec::new(),
            character: None,
            value: VmValue::Integer(-1),
        },
    ];
    let fills = [
        "TFLAG", "TSTR", "GOTJUEL", "TEQUIP", "EX", "PALAM", "SOURCE", "TCVAR",
    ]
    .into_iter()
    .map(|name| {
        Ok(VmRuntimeFill {
            variable: key(name)?,
            value: if name == "TSTR" {
                VmValue::String(String::new())
            } else {
                VmValue::Integer(0)
            },
            all_characters: matches!(
                name,
                "GOTJUEL" | "TEQUIP" | "EX" | "PALAM" | "SOURCE" | "TCVAR"
            ),
        })
    })
    .collect::<Result<Vec<_>, RuntimeError>>()?;
    let character_count = vm.vm().export_era_state().characters.len();
    let stain = key("STAIN")?;
    let stain_defaults = vm
        .vm()
        .artifact()
        .project_data
        .static_data
        .replace
        .stain_default
        .clone();
    for character in 0..character_count {
        for (index, value) in stain_defaults.iter().copied().enumerate() {
            writes.push(VmRuntimeWrite {
                variable: stain,
                indices: vec![u64::try_from(index).unwrap_or(u64::MAX)],
                character: Some(u64::try_from(character).unwrap_or(u64::MAX)),
                value: VmValue::Integer(value),
            });
        }
    }
    let prepared = vm
        .prepare_runtime_state(VmRuntimeStateTransaction::Mutate {
            writes,
            fills,
            clear_characters: false,
            add_characters_from_csv: Vec::new(),
        })
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    vm.commit_runtime_state(prepared)
        .map_err(|error| RuntimeError::Internal(error.to_string()))
}

pub(in super::super) fn reset_after_show_user(vm: &mut RuntimeVm) -> Result<(), RuntimeError> {
    let mut fills = Vec::new();
    for name in ["UP", "DOWN", "LOSEBASE", "DOWNBASE", "CUP", "CDOWN"] {
        fills.push(VmRuntimeFill {
            variable: runtime_variable_key(vm, name)?,
            value: VmValue::Integer(0),
            all_characters: matches!(name, "DOWNBASE" | "CUP" | "CDOWN"),
        });
    }
    vm.vm_mut()
        .fill_runtime_variables(&fills)
        .map_err(|error| RuntimeError::Internal(error.to_string()))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) enum PurchaseResult {
    Purchased,
    OutOfStock,
    NotEnoughMoney,
}

#[allow(clippy::too_many_lines)]
pub(in super::super) fn purchase_item(
    vm: &mut RuntimeVm,
    item: usize,
    maximum_shop_items: u32,
) -> Result<PurchaseResult, RuntimeError> {
    if item >= usize::try_from(maximum_shop_items).unwrap_or(usize::MAX) {
        return Ok(PurchaseResult::OutOfStock);
    }
    let artifact = vm.vm().artifact();
    let item_names = artifact
        .project_data
        .static_data
        .name_tables
        .get(&erabasic_data::NameTableKind::Item);
    let price = artifact
        .project_data
        .static_data
        .item_prices
        .get(item)
        .copied();
    if price.is_none()
        || item_names
            .and_then(|table| table.names.get(item))
            .and_then(Option::as_ref)
            .is_none()
    {
        return Ok(PurchaseResult::OutOfStock);
    }
    let find = |name: &str| runtime_variable_key(vm, name);
    let sales = find("ITEMSALES")?;
    let money = find("MONEY")?;
    let items = find("ITEM")?;
    let bought = find("BOUGHT")?;
    let values = vm
        .read_runtime_state(&[
            erabasic_vm::VmRuntimeRead {
                variable: sales,
                indices: vec![u64::try_from(item).unwrap_or(u64::MAX)],
                character: None,
            },
            erabasic_vm::VmRuntimeRead {
                variable: money,
                indices: Vec::new(),
                character: None,
            },
            erabasic_vm::VmRuntimeRead {
                variable: items,
                indices: vec![u64::try_from(item).unwrap_or(u64::MAX)],
                character: None,
            },
        ])
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    let [
        VmValue::Integer(for_sale),
        VmValue::Integer(current_money),
        VmValue::Integer(owned),
    ] = values.as_slice()
    else {
        return Err(RuntimeError::Internal(
            "shop variables have incompatible types".into(),
        ));
    };
    if *for_sale == 0 {
        return Ok(PurchaseResult::OutOfStock);
    }
    let price = price.expect("checked above");
    if *current_money < price {
        return Ok(PurchaseResult::NotEnoughMoney);
    }
    let prepared = vm
        .prepare_runtime_state(VmRuntimeStateTransaction::Mutate {
            writes: vec![
                VmRuntimeWrite {
                    variable: money,
                    indices: Vec::new(),
                    character: None,
                    value: VmValue::Integer(current_money - price),
                },
                VmRuntimeWrite {
                    variable: items,
                    indices: vec![u64::try_from(item).unwrap_or(u64::MAX)],
                    character: None,
                    value: VmValue::Integer(owned.saturating_add(1)),
                },
                VmRuntimeWrite {
                    variable: bought,
                    indices: Vec::new(),
                    character: None,
                    value: VmValue::Integer(i64::try_from(item).unwrap_or(i64::MAX)),
                },
            ],
            fills: Vec::new(),
            clear_characters: false,
            add_characters_from_csv: Vec::new(),
        })
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    vm.commit_runtime_state(prepared)
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    Ok(PurchaseResult::Purchased)
}
