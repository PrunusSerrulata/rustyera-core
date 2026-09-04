//! Typed MAP extensions applied only to the object captured by their staged caller.

#[allow(clippy::wildcard_imports)]
use super::*;
use crate::compat_text::{self, TextBudget};

#[derive(Clone, Copy)]
enum MapMatchMode {
    KeyContains,
    KeyPrefix,
    KeySuffix,
    ValueContains,
    ValueEqual,
    ValueNotEqual,
}

impl MapMatchMode {
    fn parse(mode: &str, allow_not_equal: bool) -> Option<Self> {
        Some(match mode {
            "KEY_CONTAINS" => Self::KeyContains,
            "KEY_PREFIX" => Self::KeyPrefix,
            "KEY_SUFFIX" => Self::KeySuffix,
            "VAL_CONTAINS" => Self::ValueContains,
            "VAL_EQ" => Self::ValueEqual,
            "VAL_NE" if allow_not_equal => Self::ValueNotEqual,
            _ => return None,
        })
    }

    fn cultural(self) -> bool {
        matches!(self, Self::KeyPrefix | Self::KeySuffix)
    }
    fn matches(
        self,
        key: &str,
        value: &str,
        needle: &str,
        budget: &mut TextBudget,
    ) -> Result<bool, ExecutionFailure> {
        Ok(match self {
            Self::KeyContains => key.contains(needle),
            Self::KeyPrefix => compat_text::map_prefix(key, needle, budget)
                .map_err(compat_text::TextError::failure)?,
            Self::KeySuffix => compat_text::map_suffix(key, needle, budget)
                .map_err(compat_text::TextError::failure)?,
            Self::ValueContains => value.contains(needle),
            Self::ValueEqual => value == needle,
            Self::ValueNotEqual => value != needle,
        })
    }
}

pub(crate) use erabasic_bytecode::MapCallKind as MapOperation;
impl StructuredState {
    pub(crate) fn merge_maps(
        &mut self,
        request: &NativeCallRequest,
    ) -> Result<NativeReady, ExecutionFailure> {
        // Both names were evaluated by the ordinary call path before either lookup.
        let destination = string_argument(request, 0)?;
        let source = string_argument(request, 1)?;
        if !self.maps.contains_key(destination) {
            return ready_integer(0);
        }
        let Some(source) = self.maps.get(source) else {
            return ready_integer(0);
        };
        let incoming = source.entries.clone();
        let destination = self
            .maps
            .get_mut(destination)
            .expect("destination checked above");
        for (key, value) in incoming {
            destination.set(key, value);
        }
        ready_integer(1)
    }
    #[allow(clippy::too_many_lines)] // Name dispatch preserves reference mutation and error order.
    pub(crate) fn call_leased_map(
        &mut self,
        operation: MapOperation,
        lease: MapLease,
        request: &NativeCallRequest,
        budget: &mut TextBudget,
    ) -> Result<NativeReady, ExecutionFailure> {
        if request.import.namespace != "rustyera.vm"
            || !request.import.name.eq_ignore_ascii_case(operation.name())
            || !operation.valid_parameters(&request.import.parameters)
            || request.import.result != Some(operation.result_type())
            || request.arguments.len() != request.import.parameters.len()
            || !request
                .arguments
                .iter()
                .zip(&request.import.parameters)
                .all(|(value, ty)| value.value_type() == *ty)
        {
            return Err(contract_failure(
                "invalid staged MAP signature or materialized arguments",
            ));
        }
        if matches!(operation, MapOperation::RemoveIf | MapOperation::FromString) {
            self.bump_map_revision()?;
        }
        let map = self.leased_map_mut(lease)?;
        match operation.name() {
            "map_values" => map_values(map, request),
            "map_removeif" => {
                let needle = string_argument(request, 1)?;
                let mode = string_argument(request, 2)?;
                let Some(mode) = MapMatchMode::parse(mode, true) else {
                    return ready_integer(-1);
                };
                // Fixed reference builds toRemove completely before any deletion.
                // Only culture-sensitive modes consume the new comparison budget.
                let mut to_remove = Vec::new();
                for (index, (key, value)) in map.entries.iter().enumerate() {
                    if mode.cultural() {
                        budget.step().map_err(text_resource)?;
                    }
                    if mode.matches(key, value, needle, budget)? {
                        if mode.cultural() {
                            budget.push(&mut to_remove, index).map_err(text_resource)?;
                        } else {
                            to_remove.push(index);
                        }
                    }
                }
                if mode.cultural() {
                    budget
                        .charge_work(map.entries.len())
                        .map_err(text_resource)?;
                }
                let count = to_remove.len();
                let mut index = 0usize;
                let mut selected = to_remove.into_iter().peekable();
                map.entries.retain(|_| {
                    let remove = selected.peek() == Some(&index);
                    if remove {
                        selected.next();
                    }
                    index += 1;
                    !remove
                });
                ready_integer(i64::try_from(count).unwrap_or(i64::MAX))
            }
            "map_findkey" => {
                let needle = string_argument(request, 1)?;
                let mode = MapMatchMode::parse(string_argument(request, 2)?, false);
                let mut keys = String::new();
                let mut any = false;
                if let Some(mode) = mode {
                    for (key, value) in &map.entries {
                        if mode.cultural() {
                            budget.step().map_err(text_resource)?;
                        }
                        if !mode.matches(key, value, needle, budget)? {
                            continue;
                        }
                        if mode.cultural() {
                            if any {
                                budget.append(&mut keys, ",").map_err(text_resource)?;
                            }
                            budget.append(&mut keys, key).map_err(text_resource)?;
                        } else {
                            if any {
                                keys.push(',');
                            }
                            keys.push_str(key);
                        }
                        any = true;
                    }
                }
                // Serialized comma fields, including commas inside keys; an
                // empty first key still causes the separator before a later key.
                if mode.is_some_and(MapMatchMode::cultural) {
                    budget.charge_work(keys.len()).map_err(text_resource)?;
                }
                let count = if keys.is_empty() {
                    0
                } else {
                    keys.split(',').count()
                };
                Ok(NativeReady {
                    value: Some(VmValue::String(keys)),
                    writes: result_count_write(request, count)?,
                })
            }
            "map_tostring" => {
                let separator = map_separator(request, 1, ",")?;
                let key_value_separator = map_separator(request, 2, "=")?;
                let mut result = String::new();
                for (index, (key, value)) in map.entries.iter().enumerate() {
                    if index != 0 {
                        result.push_str(separator);
                    }
                    result.push_str(key);
                    result.push_str(key_value_separator);
                    result.push_str(value);
                }
                Ok(NativeReady::value(VmValue::String(result)))
            }
            "map_fromstring" => {
                let data = string_argument(request, 1)?;
                // Even empty data evaluates both explicitly supplied separators first.
                let separator = map_separator(request, 2, ",")?;
                let key_value_separator = map_separator(request, 3, "=")?;
                if data.is_empty() {
                    return ready_integer(0);
                }
                budget
                    .charge_work(data.len().saturating_add(separator.len()))
                    .map_err(text_resource)?;
                let mut count = 0usize;
                // String.Split(string[], None) ignores an empty separator.
                // No Box, no precollection and no mutation rollback on error.
                if separator.is_empty() {
                    count += map_fromstring_entry(map, data, key_value_separator, budget)?;
                } else {
                    for entry in data.split(separator) {
                        count += map_fromstring_entry(map, entry, key_value_separator, budget)?;
                    }
                }
                ready_integer(i64::try_from(count).unwrap_or(i64::MAX))
            }
            _ => unreachable!("extension name checked above"),
        }
    }
}

fn map_separator<'a>(
    request: &'a NativeCallRequest,
    index: usize,
    default: &'a str,
) -> Result<&'a str, ExecutionFailure> {
    if request.arguments.get(index).is_none() {
        Ok(default)
    } else {
        string_argument(request, index)
    }
}

fn map_values(
    map: &OrderedMap,
    request: &NativeCallRequest,
) -> Result<NativeReady, ExecutionFailure> {
    if request.arguments.len() == 1 {
        let values = map
            .entries
            .iter()
            .map(|(_, value)| value.as_str())
            .collect::<Vec<_>>()
            .join(",");
        return Ok(NativeReady::value(VmValue::String(values)));
    }
    let enabled_index = match request.arguments.len() {
        2 => 1,
        3 => 2,
        _ => return Err("MAP_VALUES expects one, two, or three arguments".into()),
    };
    if integer_argument(request, enabled_index)? == 0 {
        return Ok(NativeReady::value(VmValue::String(String::new())));
    }
    let target = if request.arguments.len() == 2 {
        implicit_place(request, "RESULTS")?
    } else {
        explicit_place(request, 1)?
    };
    let value = if request.arguments.len() == 2 {
        let Some(VmValue::String(previous)) = target.values.first() else {
            return Err("MAP_VALUES implicit RESULTS first value is unavailable".into());
        };
        map.entries
            .first()
            .map_or_else(|| previous.clone(), |(_, value)| value.clone())
    } else {
        String::new()
    };
    let mut writes = array_writes(
        target,
        0,
        map.entries
            .iter()
            .map(|(_, value)| VmValue::String(value.clone())),
    );
    writes.extend(result_count_write(request, map.entries.len())?);
    Ok(NativeReady {
        value: Some(VmValue::String(value)),
        writes,
    })
}

fn text_resource(error: crate::compat_collation::ce::CeError) -> ExecutionFailure {
    compat_text::TextError::from(error).failure()
}

fn map_fromstring_entry(
    map: &mut OrderedMap,
    entry: &str,
    separator: &str,
    budget: &mut TextBudget,
) -> Result<usize, ExecutionFailure> {
    budget.step().map_err(text_resource)?;
    if entry.is_empty() {
        return Ok(0);
    }
    let Some(found) = compat_text::map_first_match(entry, separator, budget)
        .map_err(compat_text::TextError::failure)?
    else {
        return Ok(0);
    };
    let (key, value) =
        compat_text::map_entry_at_utf16_index(entry, separator, found.start_utf16, budget)
            .map_err(compat_text::TextError::failure)?;
    // Preserve dictionary overwrite position, duplicate count and per-entry
    // commit. Allocation/work failure on a later entry cannot roll this back.
    for (existing_key, existing_value) in &mut map.entries {
        budget.step().map_err(text_resource)?;
        budget
            .charge_work(existing_key.len().min(key.len()))
            .map_err(text_resource)?;
        if *existing_key == key {
            *existing_value = value;
            return Ok(1);
        }
    }
    budget
        .push(&mut map.entries, (key, value))
        .map_err(text_resource)?;
    Ok(1)
}
