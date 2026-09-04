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
