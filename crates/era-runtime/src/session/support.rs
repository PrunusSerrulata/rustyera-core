//! Private helpers shared by the runtime session protocol and VM-driving paths.
//!
//! Keeping these stateless operations separate makes the state machine in the
//! parent module easier to review without exposing new public API.

use super::*;

pub(super) fn runtime_variable_key(
    vm: &RuntimeVm,
    name: &str,
) -> Result<erabasic_bytecode::SymbolKey, RuntimeError> {
    vm.vm()
        .global_by_name(name)
        .map(|global| global.key)
        .ok_or_else(|| RuntimeError::Internal(format!("system variable {name} is missing")))
}

pub(super) fn runtime_variable_length(vm: &RuntimeVm, name: &str) -> usize {
    vm.vm()
        .global_by_name(name)
        .and_then(|global| global.dimensions.first())
        .and_then(|length| usize::try_from(*length).ok())
        .unwrap_or(0)
}

pub(super) fn table_names(
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

pub(super) fn format_named_character_values(
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

pub(super) fn format_having_items(vm: &RuntimeVm) -> Result<String, RuntimeError> {
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

pub(super) fn format_character_palam(
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

pub(super) fn format_shop_items(
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

pub(super) fn clear_upcheck_arrays(
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

pub(super) fn apply_upcheck(
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

pub(super) fn integer_argument_value(
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

pub(super) fn color_argument_value(arguments: &[VmValue]) -> Result<i64, &'static str> {
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

pub(super) fn vm_place(value: &VmValue) -> Option<PlaceDescriptor> {
    match value {
        VmValue::IntegerPlace(place) | VmValue::StringPlace(place) => Some(place.as_ref().clone()),
        VmValue::Integer(_) | VmValue::String(_) => None,
    }
}

pub(super) fn i32_argument_value(arguments: &[VmValue], index: usize) -> Result<i32, RuntimeError> {
    i32::try_from(integer_argument_value(arguments, index)?).map_err(|_| {
        RuntimeError::Internal(format!(
            "host argument {} must fit a signed 32-bit drawing coordinate",
            index + 1
        ))
    })
}

pub(super) fn checked_argb(value: i64) -> Result<i64, RuntimeError> {
    if (0..=i64::from(u32::MAX)).contains(&value) {
        Ok(value)
    } else {
        Err(RuntimeError::Internal(
            "graphics ARGB value must fit an unsigned 32-bit value".into(),
        ))
    }
}

pub(super) fn read_color_matrix(
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

pub(super) fn integer_value_or_zero(value: &VmValue) -> i64 {
    match value {
        VmValue::Integer(value) => *value,
        _ => 0,
    }
}

pub(super) fn string_argument_value<'a>(
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

pub(super) fn save_slot_argument(
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

pub(super) fn save_slot_path(slot: u32) -> String {
    format!("save{slot:02}.sav")
}

pub(super) fn parse_save_slot(path: &str) -> Option<u32> {
    path.strip_prefix("save")?
        .strip_suffix(".sav")?
        .parse()
        .ok()
}

pub(super) fn dat_filename(value: &str) -> Result<&str, RuntimeError> {
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

pub(super) fn protocol_execution_origin(
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

pub(super) fn safe_relative_path(value: &str) -> Result<String, RuntimeError> {
    era_runtime_protocol::validate_relative_path(value)
        .map_err(|error| RuntimeError::Internal(error.message))
}

pub(super) fn safe_relative_directory(value: &str) -> Result<String, RuntimeError> {
    if value.is_empty() || value == "." {
        Ok(String::new())
    } else {
        safe_relative_path(value)
    }
}

pub(super) fn text_storage_target(
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

pub(super) fn decode_load_text(bytes: &[u8]) -> Option<String> {
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

pub(super) fn read_runtime_integer(
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

pub(super) fn read_runtime_string(vm: &RuntimeVm, name: &str) -> Result<String, RuntimeError> {
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

pub(super) fn write_runtime_integer(
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

pub(super) fn synchronize_line_count(
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

pub(super) fn write_runtime_string(
    vm: &mut RuntimeVm,
    name: &str,
    value: String,
) -> Result<(), RuntimeError> {
    let variable = runtime_variable_key(vm, name)?;
    vm.vm_mut()
        .write_variable(variable, &[], None, VmValue::String(value))
        .map_err(|error| RuntimeError::Internal(error.to_string()))
}

pub(super) fn fill_runtime_variable(
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

pub(super) fn reset_training_state(vm: &mut RuntimeVm) -> Result<(), RuntimeError> {
    let artifact = vm.vm().artifact();
    let key = |name: &str| {
        artifact
            .globals
            .iter()
            .find(|global| global.name.eq_ignore_ascii_case(name))
            .map(|global| global.key)
            .ok_or_else(|| RuntimeError::Internal(format!("system variable {name} is missing")))
    };
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
    let stain_defaults = artifact
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

pub(super) fn reset_after_show_user(vm: &mut RuntimeVm) -> Result<(), RuntimeError> {
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
pub(super) enum PurchaseResult {
    Purchased,
    OutOfStock,
    NotEnoughMoney,
}

#[allow(clippy::too_many_lines)]
pub(super) fn purchase_item(
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
    let find = |name: &str| {
        artifact
            .globals
            .iter()
            .find(|global| global.name.eq_ignore_ascii_case(name))
            .map(|global| global.key)
            .ok_or_else(|| RuntimeError::Internal(format!("system variable {name} is missing")))
    };
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

pub(super) fn commit_completion(
    vm: &mut RuntimeVm,
    request: erabasic_vm::HostRequestId,
    completion: VmHostCompletion,
) -> Result<(), RuntimeError> {
    let prepared = vm
        .validate_host_completion(request, completion)
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    vm.commit_host_completion(prepared)
        .map(|_| ())
        .map_err(|error| RuntimeError::Internal(error.to_string()))
}

pub(super) fn commit_integer_result(
    vm: &mut RuntimeVm,
    request: erabasic_vm::HostRequestId,
    value: i64,
) -> Result<(), RuntimeError> {
    commit_completion(
        vm,
        request,
        VmHostCompletion::Ready(HostReady {
            value: Some(VmValue::Integer(value)),
            writes: Vec::new(),
        }),
    )
}

pub(super) fn commit_host_result_write(
    vm: &mut RuntimeVm,
    request: erabasic_vm::HostRequestId,
    value: i64,
) -> Result<(), RuntimeError> {
    let writes = global_place(vm, "RESULT")
        .map(|target| {
            vec![HostWrite {
                target,
                value: VmValue::Integer(value),
            }]
        })
        .unwrap_or_default();
    commit_completion(
        vm,
        request,
        VmHostCompletion::Ready(HostReady {
            value: None,
            writes,
        }),
    )
}

pub(super) fn global_place(vm: &RuntimeVm, name: &str) -> Option<PlaceDescriptor> {
    vm.vm()
        .artifact()
        .globals
        .iter()
        .find(|global| global.name.eq_ignore_ascii_case(name))
        .map(|global| PlaceDescriptor {
            variable: global.key,
            indices: vec![0; global.dimensions.len()],
            character: None,
            fiber: None,
            frame: None,
        })
}

pub(super) fn global_place_at(vm: &RuntimeVm, name: &str, index: usize) -> Option<PlaceDescriptor> {
    let mut place = global_place(vm, name)?;
    let first = place.indices.first_mut()?;
    *first = u64::try_from(index).ok()?;
    Some(place)
}

pub(super) fn enum_name_matches(operation: &str, candidate: &str, query: &str) -> bool {
    let candidate = candidate.to_uppercase();
    let query = query.to_uppercase();
    if operation.ends_with("BEGINSWITH") {
        candidate.starts_with(&query)
    } else if operation.ends_with("ENDSWITH") {
        candidate.ends_with(&query)
    } else {
        candidate.contains(&query)
    }
}

pub(super) fn make_bar(
    value: i64,
    maximum: i64,
    length: i64,
    full: char,
    empty: char,
) -> Result<String, &'static str> {
    if maximum <= 0 {
        return Err("BAR maximum must be positive");
    }
    if !(1..100).contains(&length) {
        return Err("BAR length must be between 1 and 99");
    }
    let filled = (value.wrapping_mul(length) / maximum).clamp(0, length);
    let mut result = String::from("[");
    result.push_str(
        &full
            .to_string()
            .repeat(usize::try_from(filled).unwrap_or(0)),
    );
    result.push_str(
        &empty
            .to_string()
            .repeat(usize::try_from(length - filled).unwrap_or(0)),
    );
    result.push(']');
    Ok(result)
}

pub(super) fn named_color(name: &str) -> Option<i64> {
    erabasic_html::named_color(name).map(i64::from)
}

pub(super) fn string_array_writes(
    vm: &RuntimeVm,
    target: Option<PlaceDescriptor>,
    values: &[String],
) -> Vec<HostWrite> {
    let Some(base) = target.or_else(|| global_place_at(vm, "RESULTS", 0)) else {
        return Vec::new();
    };
    let maximum = vm
        .vm()
        .artifact()
        .globals
        .iter()
        .find(|definition| definition.key == base.variable)
        .and_then(|definition| definition.dimensions.first())
        .and_then(|value| usize::try_from(*value).ok())
        .unwrap_or(0);
    values
        .iter()
        .take(maximum)
        .enumerate()
        .map(|(index, value)| {
            let mut target = base.clone();
            if let Some(last) = target.indices.last_mut() {
                *last = u64::try_from(index).unwrap_or(u64::MAX);
            } else {
                target
                    .indices
                    .push(u64::try_from(index).unwrap_or(u64::MAX));
            }
            HostWrite {
                target,
                value: VmValue::String(value.clone()),
            }
        })
        .collect()
}

pub(super) fn is_print(name: &str) -> bool {
    name.starts_with("PRINT") || name == "REUSELASTLINE"
}

pub(super) fn print_uses_kana_conversion(name: &str) -> bool {
    name.starts_with("PRINT") && name.contains('K')
}

pub(super) fn print_uses_default_color(name: &str) -> bool {
    name.starts_with("PRINT") && name.contains('D') && !name.starts_with("PRINTDATA")
}

pub(super) fn is_input_command(name: &str) -> bool {
    matches!(
        name,
        "INPUT"
            | "INPUTS"
            | "ONEINPUT"
            | "ONEINPUTS"
            | "TINPUT"
            | "TINPUTS"
            | "TONEINPUT"
            | "TONEINPUTS"
            | "INPUTANY"
            | "BINPUT"
            | "BINPUTS"
            | "ONEBINPUT"
            | "ONEBINPUTS"
            | "INPUTMOUSEKEY"
    )
}

pub(super) fn is_runtime_print_command(name: &str) -> bool {
    is_print(name)
        || is_input_command(name)
        || matches!(
            name,
            "WAIT"
                | "WAITANYKEY"
                | "FORCEWAIT"
                | "TWAIT"
                | "DRAWLINE"
                | "CLEARLINE"
                | "HTML_PRINT"
                | "HTML_PRINT_ISLAND"
                | "HTML_PRINT_ISLAND_CLEAR"
                | "PRINT_IMG"
                | "PRINT_RECT"
                | "PRINT_SPACE"
                | "CUSTOMDRAWLINE"
                | "DRAWLINEFORM"
        )
}

pub(super) fn column_print_alignment(name: &str) -> Option<CellAlignment> {
    match name {
        "PRINTC" | "PRINTCK" | "PRINTCD" | "PRINTFORMC" | "PRINTFORMCK" | "PRINTFORMCD" => {
            Some(CellAlignment::Right)
        }
        "PRINTLC" | "PRINTLCK" | "PRINTLCD" | "PRINTFORMLC" | "PRINTFORMLCK" | "PRINTFORMLCD" => {
            Some(CellAlignment::Left)
        }
        _ => None,
    }
}

pub(super) fn print_commits_line(name: &str) -> bool {
    name.ends_with('L') || name.ends_with('W')
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum InputSubmission {
    Value(VmValue),
    Primitive(PrimitiveResult),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PrimitiveResult {
    pub(super) fields: [i32; 5],
    pub(super) selection: Option<VmValue>,
}

pub(super) fn input_value(
    pending: &PendingInput,
    token: InteractionToken,
    intent: InputIntent,
    allow_long_activation: bool,
) -> Option<InputSubmission> {
    if let InputIntent::Activate(activated) = intent {
        if token != pending.wait.submission_token {
            return None;
        }
        let value = pending.choices.get(&activated)?;
        if pending.wait.kind == WaitKind::PrimitiveMouseKey {
            return Some(InputSubmission::Value(value.clone()));
        }
        let text = match value {
            VmValue::Integer(value) => value.to_string(),
            VmValue::String(value) => value.clone(),
            VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => return None,
        };
        return submitted_text_value(pending, text, allow_long_activation)
            .map(InputSubmission::Value);
    }
    if token != pending.wait.submission_token {
        return None;
    }
    match (pending.wait.kind, intent) {
        (WaitKind::EnterKey | WaitKind::AnyKey, InputIntent::Continue)
        | (WaitKind::EnterKey, InputIntent::Enter)
        | (WaitKind::AnyKey, InputIntent::AnyKey(_)) => {
            Some(InputSubmission::Value(VmValue::Integer(0)))
        }
        (
            WaitKind::IntegerValue
            | WaitKind::StringValue
            | WaitKind::IntegerButton
            | WaitKind::StringButton,
            InputIntent::CommitText(value),
        ) => submitted_text_value(pending, value, false).map(InputSubmission::Value),
        (WaitKind::AnyValue, InputIntent::CommitText(value)) => Some(InputSubmission::Value(
            value
                .parse()
                .map_or_else(|_| VmValue::String(value), VmValue::Integer),
        )),
        (WaitKind::PrimitiveMouseKey, InputIntent::Primitive(value))
            if matches!(value.input_type, 1..=3) =>
        {
            let selection = match value.selection_token {
                Some(token) => Some(pending.choices.get(&token)?.clone()),
                None => None,
            };
            Some(InputSubmission::Primitive(PrimitiveResult {
                fields: [
                    value.input_type,
                    value.result_1,
                    value.result_2,
                    value.result_3,
                    value.result_4,
                ],
                selection,
            }))
        }
        _ => None,
    }
}

fn submitted_text_value(
    pending: &PendingInput,
    mut text: String,
    allow_long_activation: bool,
) -> Option<VmValue> {
    let use_default = text.is_empty() && pending.wait.deadline_ns.is_none();
    let value = if use_default {
        pending.wait.default_value.as_ref().map(protocol_to_vm)
    } else {
        if pending.wait.one_input && !allow_long_activation {
            text.truncate(text.chars().next().map_or(0, char::len_utf8));
        }
        match pending.wait.kind {
            WaitKind::IntegerValue | WaitKind::IntegerButton => {
                text.parse().ok().map(VmValue::Integer)
            }
            WaitKind::StringValue | WaitKind::StringButton => Some(VmValue::String(text)),
            _ => None,
        }
    }?;
    submission_matches_wait(pending, value)
}

fn submission_matches_wait(pending: &PendingInput, value: VmValue) -> Option<VmValue> {
    match (&pending.wait.kind, &value) {
        (WaitKind::IntegerValue, VmValue::Integer(_))
        | (WaitKind::StringValue, VmValue::String(_)) => Some(value),
        (WaitKind::IntegerButton, VmValue::Integer(candidate)) => pending
            .choices
            .values()
            .any(|choice| matches!(choice, VmValue::Integer(value) if value == candidate))
            .then_some(value),
        (WaitKind::StringButton, VmValue::String(candidate)) => pending
            .choices
            .values()
            .any(|choice| match choice {
                VmValue::Integer(value) => value.to_string() == *candidate,
                VmValue::String(value) => value == candidate,
                VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => false,
            })
            .then_some(value),
        _ => None,
    }
}

pub(super) fn selected_capabilities(client: &ClientCapabilities) -> ClientCapabilities {
    let services = selected_service_capabilities(&client.services);
    let font_metrics = client.font_metrics
        && services.iter().any(|capability| {
            capability.kind == ServiceKind::FontMetrics
                && capability.operation == GGET_TEXT_SIZE_OPERATION
        });
    ClientCapabilities {
        input_modalities: client.input_modalities.clone(),
        rich_text: client.rich_text,
        html: client.html,
        graphics: client.graphics,
        audio: client.audio,
        // Video still requires a typed playback contract.
        video: false,
        font_metrics,
        column_cells: client.column_cells,
        separators: client.separators,
        available_fonts: {
            let mut fonts = client.available_fonts.clone();
            fonts.sort_by_key(|name| name.to_lowercase());
            fonts.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
            fonts
        },
        services,
        storage: client.storage,
    }
}

pub(super) fn selected_service_capabilities(
    client: &[ServiceCapability],
) -> Vec<ServiceCapability> {
    let mut selected = client
        .iter()
        .filter_map(|capability| {
            let supported = match (capability.kind, capability.operation.as_str()) {
                (ServiceKind::Clock, LOCAL_DATE_TIME_OPERATION) => {
                    LOCAL_DATE_TIME_OPERATION_VERSION
                }
                (ServiceKind::Entropy, RANDOM_SEED_OPERATION) => RANDOM_SEED_OPERATION_VERSION,
                (ServiceKind::InputState, GET_KEY_STATE_OPERATION) => {
                    GET_KEY_STATE_OPERATION_VERSION
                }
                (ServiceKind::Image, IMAGE_METADATA_OPERATION) => IMAGE_METADATA_OPERATION_VERSION,
                (ServiceKind::Image, IMAGE_PIXEL_OPERATION) => IMAGE_PIXEL_OPERATION_VERSION,
                (ServiceKind::Network, UPDATE_CHECK_OPERATION) => UPDATE_CHECK_OPERATION_VERSION,
                (ServiceKind::OpenUrl, OPEN_URL_OPERATION) => OPEN_URL_OPERATION_VERSION,
                (ServiceKind::PresentationQuery, GET_DISPLAY_LINE_OPERATION) => {
                    GET_DISPLAY_LINE_OPERATION_VERSION
                }
                (ServiceKind::PresentationQuery, HTML_GET_PRINTED_STR_OPERATION) => {
                    HTML_GET_PRINTED_STR_OPERATION_VERSION
                }
                (ServiceKind::PresentationQuery, HTML_STRING_LEN_OPERATION) => {
                    HTML_STRING_LEN_OPERATION_VERSION
                }
                (ServiceKind::PresentationQuery, HTML_SUBSTRING_OPERATION) => {
                    HTML_SUBSTRING_OPERATION_VERSION
                }
                (ServiceKind::PresentationQuery, HTML_STRING_LINES_OPERATION) => {
                    HTML_STRING_LINES_OPERATION_VERSION
                }
                (ServiceKind::PresentationQuery, SERIALIZE_PHYSICAL_HISTORY_OPERATION) => {
                    SERIALIZE_PHYSICAL_HISTORY_OPERATION_VERSION
                }
                (ServiceKind::FontMetrics, GGET_TEXT_SIZE_OPERATION) => {
                    GGET_TEXT_SIZE_OPERATION_VERSION
                }
                (ServiceKind::Canvas, SAMPLE_CANVAS_PIXEL_OPERATION) => {
                    SAMPLE_CANVAS_PIXEL_OPERATION_VERSION
                }
                (ServiceKind::Canvas, DECODE_CANVAS_IMAGE_OPERATION) => {
                    DECODE_CANVAS_IMAGE_OPERATION_VERSION
                }
                (ServiceKind::Canvas, ENCODE_CANVAS_PNG_OPERATION) => {
                    ENCODE_CANVAS_PNG_OPERATION_VERSION
                }
                // Extension operations are application-defined. Select the client's
                // maximum now; a later registry declaration must bind that exact version.
                (ServiceKind::Extension, _) => capability.versions.maximum,
                _ => return None,
            };
            negotiate_version(capability.versions, VersionRange::exact(supported)).map(|version| {
                ServiceCapability {
                    kind: capability.kind,
                    operation: if capability.kind == ServiceKind::Extension {
                        capability.operation.to_ascii_lowercase()
                    } else {
                        capability.operation.clone()
                    },
                    versions: VersionRange::exact(version),
                }
            })
        })
        .collect::<Vec<_>>();
    selected.sort_by(|left, right| {
        (left.kind, left.operation.as_str()).cmp(&(right.kind, right.operation.as_str()))
    });
    selected.dedup_by(|left, right| left.kind == right.kind && left.operation == right.operation);
    selected
}

pub(super) fn select_locale(preferred: &[String]) -> &'static str {
    for locale in preferred {
        let locale = locale.to_ascii_lowercase();
        if locale == "zh-hans" || locale.starts_with("zh-cn") || locale.starts_with("zh-sg") {
            return "zh-Hans";
        }
        if locale == "en" || locale.starts_with("en-") {
            return "en";
        }
        if locale == "ja" || locale.starts_with("ja-") {
            return "ja";
        }
    }
    "ja"
}

pub(super) fn localized_system_text(locale: &str, key: SystemTextKey) -> String {
    let value = match (locale, key) {
        ("zh-Hans", SystemTextKey::InvalidValue) => "输入无效",
        ("zh-Hans", SystemTextKey::SaveQuestion) => "请选择保存位置",
        ("zh-Hans", SystemTextKey::LoadQuestion) => "请选择要读取的存档",
        ("zh-Hans", SystemTextKey::OverwriteQuestion) => "要覆盖这个存档吗？",
        ("zh-Hans", SystemTextKey::NotEnoughMoney) => "金钱不足",
        ("zh-Hans", SystemTextKey::OutOfStock) => "无法购买",
        ("zh-Hans", SystemTextKey::AutoSaveFailed) => "自动保存失败",
        ("zh-Hans", SystemTextKey::AutoSaveSkipped) => "已跳过自动保存",
        ("zh-Hans", SystemTextKey::ContinuousTrainProgress) => "＜连续执行：第 {0}/{1} 个命令＞",
        ("zh-Hans", SystemTextKey::ContinuousTrainCommandFailed) => "无法执行命令",
        ("zh-Hans", SystemTextKey::PressAnyKey) => "请按任意键",
        ("zh-Hans", SystemTextKey::SaveSlot) => "存档",
        ("zh-Hans", SystemTextKey::Back) => "返回",
        ("zh-Hans", SystemTextKey::NewGame) => "开始新游戏",
        ("zh-Hans", SystemTextKey::LoadGame) => "读取存档",
        ("en", SystemTextKey::InvalidValue) => "Invalid value",
        ("en", SystemTextKey::SaveQuestion) => "Select a save slot",
        ("en", SystemTextKey::LoadQuestion) => "Select a save to load",
        ("en", SystemTextKey::OverwriteQuestion) => "Overwrite this save?",
        ("en", SystemTextKey::NotEnoughMoney) => "Not enough money",
        ("en", SystemTextKey::OutOfStock) => "This item cannot be purchased",
        ("en", SystemTextKey::AutoSaveFailed) => "Autosave failed",
        ("en", SystemTextKey::AutoSaveSkipped) => "Autosave skipped",
        ("en", SystemTextKey::ContinuousTrainProgress) => "<Continuous command: {0}/{1}>",
        ("en", SystemTextKey::ContinuousTrainCommandFailed) => "The command could not be executed",
        ("en", SystemTextKey::PressAnyKey) => "Press any key",
        ("en", SystemTextKey::SaveSlot) => "Save",
        ("en", SystemTextKey::Back) => "Back",
        ("en", SystemTextKey::NewGame) => "Start a new game",
        ("en", SystemTextKey::LoadGame) => "Load game",
        (_, SystemTextKey::InvalidValue) => "入力が正しくありません",
        (_, SystemTextKey::SaveQuestion) => "セーブするデータを選択してください",
        (_, SystemTextKey::LoadQuestion) => "ロードするデータを選択してください",
        (_, SystemTextKey::OverwriteQuestion) => "上書きしてよろしいですか？",
        (_, SystemTextKey::NotEnoughMoney) => "所持金が足りません",
        (_, SystemTextKey::OutOfStock) => "購入できません",
        (_, SystemTextKey::AutoSaveFailed) => "オートセーブに失敗しました",
        (_, SystemTextKey::AutoSaveSkipped) => "オートセーブをスキップしました",
        (_, SystemTextKey::ContinuousTrainProgress) => "＜コマンド連続実行：{0}/{1}＞",
        (_, SystemTextKey::ContinuousTrainCommandFailed) => "コマンドを実行できませんでした",
        (_, SystemTextKey::PressAnyKey) => "何かキーを押してください",
        (_, SystemTextKey::SaveSlot) => "セーブデータ",
        (_, SystemTextKey::Back) => "戻る",
        (_, SystemTextKey::NewGame) => "最初からはじめる",
        (_, SystemTextKey::LoadGame) => "ロードする",
    };
    value.into()
}

pub(super) fn protocol_to_vm(value: &era_runtime_protocol::ProtocolValue) -> VmValue {
    match value {
        era_runtime_protocol::ProtocolValue::Integer(value) => VmValue::Integer(*value),
        era_runtime_protocol::ProtocolValue::String(value) => VmValue::String(value.clone()),
        era_runtime_protocol::ProtocolValue::Boolean(value) => VmValue::Integer(i64::from(*value)),
        era_runtime_protocol::ProtocolValue::Bytes(_) => VmValue::String(String::new()),
    }
}

pub(super) fn extension_protocol_value(
    value: era_runtime_protocol::ProtocolValue,
) -> Option<VmValue> {
    match value {
        era_runtime_protocol::ProtocolValue::Integer(value) => Some(VmValue::Integer(value)),
        era_runtime_protocol::ProtocolValue::String(value) => Some(VmValue::String(value)),
        era_runtime_protocol::ProtocolValue::Boolean(value) => {
            Some(VmValue::Integer(i64::from(value)))
        }
        era_runtime_protocol::ProtocolValue::Bytes(_) => None,
    }
}

pub(super) fn calendar_number(time: LocalDateTimeResponse) -> i64 {
    let date = i64::from(time.year) * 10_000_000_000
        + i64::from(time.month) * 100_000_000
        + i64::from(time.day) * 1_000_000
        + i64::from(time.hour) * 10_000
        + i64::from(time.minute) * 100
        + i64::from(time.second);
    date * 1000 + i64::from(time.millisecond)
}

pub(super) fn complete_frozen_clock(
    vm: &mut RuntimeVm,
    request: &VmHostRequest,
    time: LocalDateTimeResponse,
) -> Result<(), RuntimeError> {
    let name = request.import.import.name.to_ascii_uppercase();
    let operation = match name.as_str() {
        "GETTIME" => ClockOperation::Time,
        "GETTIMES" => ClockOperation::Times,
        "GETMILLISECOND" => ClockOperation::Millisecond,
        "GETSECOND" => ClockOperation::Second,
        _ => {
            return Err(RuntimeError::Internal(format!(
                "clock operation {name} has no frozen candidate implementation"
            )));
        }
    };
    let mut writes = Vec::new();
    let value = if request.import.import.result.is_none() {
        if let Some(target) = global_place(vm, "RESULT") {
            writes.push(HostWrite {
                target,
                value: VmValue::Integer(calendar_number(time)),
            });
        }
        if let Some(target) = global_place(vm, "RESULTS") {
            writes.push(HostWrite {
                target,
                value: VmValue::String(calendar_string(time)),
            });
        }
        None
    } else {
        Some(match operation {
            ClockOperation::Time => VmValue::Integer(calendar_number(time)),
            ClockOperation::Times => VmValue::String(calendar_string(time)),
            ClockOperation::Millisecond => VmValue::Integer(milliseconds_since_year_one(time)),
            ClockOperation::Second => VmValue::Integer(milliseconds_since_year_one(time) / 1_000),
        })
    };
    commit_completion(
        vm,
        request.id,
        VmHostCompletion::Ready(HostReady { value, writes }),
    )
}

pub(super) fn calendar_string(time: LocalDateTimeResponse) -> String {
    format!(
        "{:04}/{:02}/{:02} {:02}:{:02}:{:02}",
        time.year, time.month, time.day, time.hour, time.minute, time.second
    )
}

pub(super) fn milliseconds_since_year_one(time: LocalDateTimeResponse) -> i64 {
    const DAYS_BEFORE_MONTH: [i64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    // This is the proleptic Gregorian calendar used by DateTime.Now.Ticks.
    let year_before = i64::from(time.year) - 1;
    let days_before_year =
        year_before * 365 + year_before / 4 - year_before / 100 + year_before / 400;
    let mut days = days_before_year
        + DAYS_BEFORE_MONTH[usize::from(time.month.saturating_sub(1).min(11))]
        + i64::from(time.day.saturating_sub(1));
    if time.month > 2 && is_leap_year(time.year) {
        days += 1;
    }
    (((days * 24 + i64::from(time.hour)) * 60 + i64::from(time.minute)) * 60
        + i64::from(time.second))
        * 1000
        + i64::from(time.millisecond)
}

const fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

pub(super) fn intersect_limits(left: RuntimeLimits, right: RuntimeLimits) -> RuntimeLimits {
    RuntimeLimits {
        maximum_envelope_bytes: left
            .maximum_envelope_bytes
            .min(right.maximum_envelope_bytes),
        maximum_payload_bytes: left.maximum_payload_bytes.min(right.maximum_payload_bytes),
        maximum_pending_requests: left
            .maximum_pending_requests
            .min(right.maximum_pending_requests),
        maximum_journal_entries: left
            .maximum_journal_entries
            .min(right.maximum_journal_entries),
        maximum_drive_instructions: left
            .maximum_drive_instructions
            .min(right.maximum_drive_instructions),
        maximum_transfer_bytes: left
            .maximum_transfer_bytes
            .min(right.maximum_transfer_bytes),
    }
}

pub(super) fn debugger_suspends_message(message: &RuntimeMessage) -> bool {
    matches!(
        message,
        RuntimeMessage::ProjectManifest(_)
            | RuntimeMessage::ProjectLoad(_)
            | RuntimeMessage::ProjectAnalysisRequest(_)
            | RuntimeMessage::KeyMacroProfileSubmit(_)
            | RuntimeMessage::KeyMacroCommand(_)
            | RuntimeMessage::ExtensionRegistrySubmit(_)
            | RuntimeMessage::ReturnToTitle(_)
            | RuntimeMessage::Start(_)
            | RuntimeMessage::Input(_)
            | RuntimeMessage::InputUndoRequest(_)
            | RuntimeMessage::ServiceResponse(_)
            | RuntimeMessage::StorageResponse(_)
            | RuntimeMessage::StateExportRequest(_)
            | RuntimeMessage::StateImportBegin(_)
            | RuntimeMessage::StateImportChunk(_)
            | RuntimeMessage::StateImportCommit(_)
            | RuntimeMessage::StateExportChunkRequest(_)
            | RuntimeMessage::StateTransferCancel(_)
            | RuntimeMessage::ReloadProject(_)
    )
}

pub(super) fn format_era_integer(value: i64, format: &str) -> Result<String, &'static str> {
    if format.is_empty() {
        return Ok(value.to_string());
    }
    let mut chars = format.chars();
    let first = chars.next().expect("non-empty format");
    let precision = chars.as_str().parse::<usize>().ok();
    match first.to_ascii_uppercase() {
        'D' if chars.as_str().is_empty() || precision.is_some() => {
            let width = precision.unwrap_or(0);
            let magnitude = value.unsigned_abs().to_string();
            let digits = format!("{magnitude:0>width$}");
            Ok(if value < 0 {
                format!("-{digits}")
            } else {
                digits
            })
        }
        'X' if chars.as_str().is_empty() || precision.is_some() => {
            let width = precision.unwrap_or(0);
            if first.is_ascii_lowercase() {
                Ok(format!("{value:0>width$x}"))
            } else {
                Ok(format!("{value:0>width$X}"))
            }
        }
        'N' if chars.as_str().is_empty() || precision.is_some() => {
            let decimals = precision.unwrap_or(2);
            let grouped = group_decimal(value);
            Ok(if decimals == 0 {
                grouped
            } else {
                format!("{grouped}.{}", "0".repeat(decimals))
            })
        }
        _ => {
            let Some((numeric_format, literal_suffix)) = custom_decimal_format(format) else {
                return Err("unsupported integer format");
            };
            let minimum = numeric_format
                .chars()
                .filter(|character| *character == '0')
                .count();
            let magnitude = value.unsigned_abs().to_string();
            let mut digits = format!("{magnitude:0>minimum$}");
            if numeric_format.contains(',') {
                digits = group_unsigned_decimal(&digits);
            }
            let formatted = if value < 0 {
                format!("-{digits}")
            } else {
                digits
            };
            Ok(format!("{formatted}{literal_suffix}"))
        }
    }
}

fn custom_decimal_format(format: &str) -> Option<(&str, &str)> {
    let suffix_start = format
        .char_indices()
        .find_map(|(index, character)| (!matches!(character, '#' | '0' | ',')).then_some(index))
        .unwrap_or(format.len());
    let (numeric_format, literal_suffix) = format.split_at(suffix_start);
    if !numeric_format.contains('0')
        || literal_suffix.chars().any(|character| {
            matches!(
                character,
                '#' | '0' | ',' | '.' | '%' | '‰' | 'E' | 'e' | '\\' | '\'' | '"' | ';'
            )
        })
    {
        return None;
    }
    Some((numeric_format, literal_suffix))
}

pub(super) fn group_decimal(value: i64) -> String {
    let digits = group_unsigned_decimal(&value.unsigned_abs().to_string());
    if value < 0 {
        format!("-{digits}")
    } else {
        digits
    }
}

pub(super) fn group_unsigned_decimal(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + value.len() / 3);
    for (index, character) in value.chars().enumerate() {
        if index != 0 && (value.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(character);
    }
    output
}

// Version 1 of the deterministic width table covers the ASCII block and the
// half-width katakana block used by Emuera projects.  It deliberately avoids
// the platform-dependent VisualBasic StrConv implementation.
const HALF_KANA: &str = "｡｢｣､･ｦｧｨｩｪｫｬｭｮｯｰｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝﾞﾟ";
const FULL_KANA: &str = "。「」、・ヲァィゥェォャュョッーアイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワン゛゜";

pub(super) fn to_full_width(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut input = value.chars().peekable();
    while let Some(character) = input.next() {
        if let Some(mark) = input.peek().copied()
            && matches!(mark, 'ﾞ' | 'ﾟ')
            && let Some(composed) = compose_half_kana(character, mark)
        {
            output.push(composed);
            input.next();
            continue;
        }
        match character {
            ' ' => output.push('　'),
            '!'..='~' => output.push(char::from_u32(u32::from(character) + 0xfee0).unwrap()),
            _ => output.push(map_width_char(character, HALF_KANA, FULL_KANA).unwrap_or(character)),
        }
    }
    output
}

pub(super) fn to_half_width(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        if let Some(pair) = decompose_full_kana(character) {
            output.extend(pair);
            continue;
        }
        match character {
            '　' => output.push(' '),
            '\u{ff01}'..='\u{ff5e}' => {
                output.push(char::from_u32(u32::from(character) - 0xfee0).unwrap());
            }
            _ => output.push(map_width_char(character, FULL_KANA, HALF_KANA).unwrap_or(character)),
        }
    }
    output
}

/// Apply the pinned Japanese LCID 0x0411 subset used by FORCEKANA. The table is
/// embedded so execution never depends on the host locale or platform APIs.
pub(super) fn convert_kana_mode(value: &str, mode: u8) -> String {
    let value = if mode == 3 {
        to_full_width(value)
    } else {
        value.to_owned()
    };
    value
        .chars()
        .map(|character| match mode {
            1 => hiragana_to_katakana(character),
            2 | 3 => katakana_to_hiragana(character),
            _ => character,
        })
        .collect()
}

pub(super) fn hiragana_to_katakana(character: char) -> char {
    match character {
        '\u{3041}'..='\u{3096}' => char::from_u32(u32::from(character) + 0x60).unwrap_or(character),
        'ゝ' => 'ヽ',
        'ゞ' => 'ヾ',
        _ => character,
    }
}

pub(super) fn katakana_to_hiragana(character: char) -> char {
    match character {
        '\u{30a1}'..='\u{30f6}' => char::from_u32(u32::from(character) - 0x60).unwrap_or(character),
        'ヽ' => 'ゝ',
        'ヾ' => 'ゞ',
        _ => character,
    }
}

pub(super) fn map_width_char(character: char, source: &str, target: &str) -> Option<char> {
    source
        .chars()
        .position(|candidate| candidate == character)
        .and_then(|index| target.chars().nth(index))
}

pub(super) fn compose_half_kana(base: char, mark: char) -> Option<char> {
    let bases = "ｳｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾊﾋﾌﾍﾎﾊﾋﾌﾍﾎ";
    let marks = "ﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾟﾟﾟﾟﾟ";
    let full = "ヴガギグゲゴザジズゼゾダヂヅデドバビブベボパピプペポ";
    bases
        .chars()
        .zip(marks.chars())
        .position(|(candidate, candidate_mark)| candidate == base && candidate_mark == mark)
        .and_then(|index| full.chars().nth(index))
}

pub(super) fn decompose_full_kana(character: char) -> Option<[char; 2]> {
    let full = "ヴガギグゲゴザジズゼゾダヂヅデドバビブベボパピプペポ";
    let bases = "ｳｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾊﾋﾌﾍﾎﾊﾋﾌﾍﾎ";
    let marks = "ﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾟﾟﾟﾟﾟ";
    full.chars()
        .position(|candidate| candidate == character)
        .and_then(|index| Some([bases.chars().nth(index)?, marks.chars().nth(index)?]))
}

#[cfg(test)]
mod color_tests {
    use super::*;

    #[test]
    fn emuera_color_arguments_accept_packed_or_three_channels() {
        assert_eq!(
            color_argument_value(&[VmValue::Integer(0x01_18_3c)]),
            Ok(0x01_18_3c)
        );
        assert_eq!(
            color_argument_value(&[
                VmValue::Integer(1),
                VmValue::Integer(24),
                VmValue::Integer(60),
            ]),
            Ok(0x01_18_3c)
        );
        assert_eq!(color_argument_value(&[VmValue::Integer(-1)]), Ok(0xff_ffff));
        assert!(color_argument_value(&[VmValue::Integer(1), VmValue::Integer(2)]).is_err());
        assert!(
            color_argument_value(&[
                VmValue::Integer(256),
                VmValue::Integer(0),
                VmValue::Integer(0),
            ])
            .is_err()
        );
    }
}
