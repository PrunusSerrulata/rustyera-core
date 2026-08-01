#[allow(clippy::wildcard_imports)]
use super::*;

pub(in super::super) fn execute_strjoin(
    vm: &Vm,
    fiber: &Fiber,
    arguments: &[VmValue],
) -> Result<VmValue, VmError> {
    let place = array_place(arguments)?;
    let values = array_snapshot_any_rank(vm, fiber, place)?;
    let delimiter = match arguments.get(1) {
        None => ",",
        Some(VmValue::String(value)) => value,
        _ => {
            return Err(VmError::InvalidArguments(
                "STRJOIN delimiter must be a string".into(),
            ));
        }
    };
    let start = match arguments.get(2) {
        None => 0,
        Some(VmValue::Integer(value)) => usize::try_from(*value)
            .map_err(|_| VmError::InvalidArguments("STRJOIN start is negative".into()))?,
        _ => {
            return Err(VmError::InvalidArguments(
                "STRJOIN start must be an integer".into(),
            ));
        }
    };
    if start > values.len() {
        return Err(VmError::InvalidArguments(
            "STRJOIN start exceeds the array".into(),
        ));
    }
    let count = match arguments.get(3) {
        None => values.len() - start,
        Some(VmValue::Integer(value)) => usize::try_from(*value)
            .map_err(|_| VmError::InvalidArguments("STRJOIN count is negative".into()))?,
        _ => {
            return Err(VmError::InvalidArguments(
                "STRJOIN count must be an integer".into(),
            ));
        }
    };
    let end = start
        .checked_add(count)
        .filter(|end| *end <= values.len())
        .ok_or_else(|| VmError::InvalidArguments("STRJOIN range exceeds the array".into()))?;
    let joined = values[start..end]
        .iter()
        .map(|value| match value {
            VmValue::Integer(value) => Ok(value.to_string()),
            VmValue::String(value) => Ok(value.clone()),
            VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => Err(VmError::InvalidState(
                "STRJOIN array contains a place".into(),
            )),
        })
        .collect::<Result<Vec<_>, _>>()?
        .join(delimiter);
    Ok(VmValue::String(joined))
}

pub(in super::super) fn array_snapshot(
    vm: &Vm,
    fiber: &Fiber,
    place: &PlaceDescriptor,
) -> Result<Vec<VmValue>, VmError> {
    let generation = fiber.frames.last().expect("frame exists").generation;
    let program = vm
        .generations
        .get(&generation)
        .ok_or_else(|| VmError::InvalidState("array generation is missing".into()))?;
    let definition = program
        .global(place.variable)
        .ok_or_else(|| VmError::InvalidState("array variable is missing".into()))?;
    if definition.dimensions.len() != 1 || place.indices.len() > 1 {
        return Err(VmError::InvalidArguments(
            "array operation requires a one-dimensional variable".into(),
        ));
    }
    // Reference-taking one-dimensional operations accept both `ARRAY` and
    // `ARRAY:element`. For character data, the character selector has already
    // been split into `place.character`; discarding the remaining element
    // selector therefore preserves the selected character while exposing its
    // complete array. The operation range comes from the following arguments.
    let mut array = place.clone();
    array.indices.clear();
    vm.read_place_array(fiber, &array)
}

pub(in super::super) fn commit_array(
    vm: &mut Vm,
    fiber: &mut Fiber,
    place: &PlaceDescriptor,
    values: Vec<VmValue>,
) -> Result<(), VmError> {
    let mut array = place.clone();
    array.indices.clear();
    vm.write_place_array(fiber, &array, values)
}

pub(in super::super) fn execute_array_mutation(
    vm: &mut Vm,
    fiber: &mut Fiber,
    operation: &str,
    arguments: &[VmValue],
) -> Result<(), VmError> {
    let place = array_place(arguments)?.clone();
    let mut values = array_snapshot(vm, fiber, &place)?;
    match operation {
        "arrayremove" => {
            let start = usize::try_from(integer_argument(arguments, 1)?)
                .map_err(|_| VmError::InvalidArguments("ARRAYREMOVE start is negative".into()))?;
            let count = integer_argument(arguments, 2)?;
            if count <= 0 || start >= values.len() {
                return Ok(());
            }
            let count = usize::try_from(count).unwrap_or(usize::MAX);
            let end = start.saturating_add(count).min(values.len());
            let removed = end - start;
            for source in end..values.len() {
                values[source - removed] = values[source].clone();
            }
            let default = VmValue::default_for(values[0].value_type());
            let fill_start = values.len() - removed;
            for value in &mut values[fill_start..] {
                *value = default.clone();
            }
        }
        "arrayshift" => {
            let shift = integer_argument(arguments, 1)?;
            if shift == 0 {
                return Ok(());
            }
            let fill = arguments.get(2).cloned().ok_or_else(|| {
                VmError::InvalidArguments("ARRAYSHIFT fill value is missing".into())
            })?;
            if values
                .first()
                .is_some_and(|value| value.value_type() != fill.value_type())
            {
                return Err(VmError::InvalidArguments(
                    "ARRAYSHIFT fill type differs".into(),
                ));
            }
            let start = match integer_argument(arguments, 3).unwrap_or(0) {
                i64::MIN => 0,
                value => usize::try_from(value).map_err(|_| {
                    VmError::InvalidArguments("ARRAYSHIFT start is negative".into())
                })?,
            };
            if start > values.len() {
                return Err(VmError::InvalidArguments(
                    "ARRAYSHIFT start exceeds array".into(),
                ));
            }
            let count = match integer_argument(arguments, 4).unwrap_or(i64::MIN) {
                i64::MIN => values.len() - start,
                value => usize::try_from(value).map_err(|_| {
                    VmError::InvalidArguments("ARRAYSHIFT count is negative".into())
                })?,
            };
            let end = start.saturating_add(count).min(values.len());
            let source = values[start..end].to_vec();
            for (relative, value) in values[start..end].iter_mut().enumerate() {
                let source_index = i64::try_from(relative).unwrap_or(i64::MAX) - shift;
                *value = usize::try_from(source_index)
                    .ok()
                    .and_then(|source_index| source.get(source_index).cloned())
                    .unwrap_or_else(|| fill.clone());
            }
        }
        "arraysort" => {
            let descending = arguments.get(1).is_some_and(|value| {
                matches!(value, VmValue::String(value) if value.eq_ignore_ascii_case("BACK"))
                    || matches!(value, VmValue::Integer(value) if *value < 0)
            });
            let start = match integer_argument(arguments, 2).unwrap_or(0) {
                i64::MIN => 0,
                value => usize::try_from(value)
                    .map_err(|_| VmError::InvalidArguments("ARRAYSORT start is negative".into()))?,
            };
            let count = match integer_argument(arguments, 3).unwrap_or(i64::MIN) {
                i64::MIN => values.len().saturating_sub(start),
                value => usize::try_from(value)
                    .map_err(|_| VmError::InvalidArguments("ARRAYSORT count is negative".into()))?,
            };
            let end = start.saturating_add(count).min(values.len());
            if start > end {
                return Err(VmError::InvalidArguments(
                    "ARRAYSORT range is invalid".into(),
                ));
            }
            values[start..end].sort_by(|left, right| match (left, right) {
                (VmValue::Integer(left), VmValue::Integer(right)) => left.cmp(right),
                (VmValue::String(left), VmValue::String(right)) => left.cmp(right),
                _ => std::cmp::Ordering::Equal,
            });
            if descending {
                values[start..end].reverse();
            }
        }
        _ => return Err(VmError::InvalidArguments("unknown array mutation".into())),
    }
    commit_array(vm, fiber, &place, values)
}

#[allow(clippy::too_many_lines)]
pub(in super::super) fn execute_variable_fill(
    vm: &mut Vm,
    fiber: &mut Fiber,
    operation: &str,
    arguments: &[VmValue],
) -> Result<(), VmError> {
    let place = match arguments.first() {
        Some(VmValue::IntegerPlace(place) | VmValue::StringPlace(place)) => place.clone(),
        _ => {
            return Err(VmError::InvalidArguments(format!(
                "{operation} destination must be a mutable variable place"
            )));
        }
    };
    let generation = fiber.frames.last().expect("frame exists").generation;
    let definition = vm
        .generations
        .get(&generation)
        .and_then(|generation| generation.global(place.variable))
        .cloned()
        .ok_or_else(|| VmError::InvalidState("VARSET variable is missing".into()))?;
    if !definition.mutable {
        return Err(VmError::InvalidArguments(
            "VARSET destination is read-only".into(),
        ));
    }
    let default = VmValue::default_for(definition.value_type);
    if operation == "varset" {
        if definition.storage == BytecodeStorage::Character && place.character.is_none() {
            return Err(VmError::InvalidArguments(
                "VARSET character destination has no character".into(),
            ));
        }
        let value = fill_value_or_default(arguments, 1, default);
        if value.value_type() != definition.value_type {
            return Err(VmError::InvalidArguments(
                "VARSET value type differs".into(),
            ));
        }
        if definition.dimensions.is_empty() {
            let _ = vm.read_place(fiber, &place)?;
            return vm.write_place(fiber, &place, value);
        }
        let mut array = place;
        array.indices.clear();
        let length = vm.place_array_len(fiber, &array)?;
        let (start, end) = if definition.dimensions.len() == 1 {
            let mut start = optional_nonnegative(arguments, 2, 0, "VARSET start")?;
            let mut end = optional_nonnegative(arguments, 3, length, "VARSET end")?;
            if start > end {
                std::mem::swap(&mut start, &mut end);
            }
            if end > length {
                return Err(VmError::InvalidArguments("VARSET range is invalid".into()));
            }
            (start, end)
        } else {
            // The reference ignores range arguments for higher-rank arrays and
            // applies VARSET to their complete flattened storage.
            (0, length)
        };
        return vm.fill_place_array_range(fiber, &array, start, end, value);
    }

    if definition.storage != BytecodeStorage::Character || definition.dimensions.len() > 1 {
        return Err(VmError::InvalidArguments(
            "CVARSET requires a scalar or one-dimensional character variable".into(),
        ));
    }
    let element = optional_nonnegative(arguments, 1, 0, "CVARSET element")?;
    let value = fill_value_or_default(arguments, 2, default);
    if value.value_type() != definition.value_type {
        return Err(VmError::InvalidArguments(
            "CVARSET value type differs".into(),
        ));
    }
    let character_count = vm.memory.characters.len();
    let mut start = optional_nonnegative(arguments, 3, 0, "CVARSET start")?;
    let mut end = optional_nonnegative(arguments, 4, character_count, "CVARSET end")?;
    if start > end {
        std::mem::swap(&mut start, &mut end);
    }
    if end > character_count {
        return Err(VmError::InvalidArguments("CVARSET range is invalid".into()));
    }
    let indices = if definition.dimensions.is_empty() {
        Vec::new()
    } else {
        if element >= usize::try_from(definition.dimensions[0]).unwrap_or(0) {
            return Err(VmError::InvalidArguments(
                "CVARSET element is out of range".into(),
            ));
        }
        vec![u64::try_from(element).unwrap_or(u64::MAX)]
    };
    let destinations = (start..end)
        .map(|character| PlaceDescriptor {
            indices: indices.clone(),
            character: Some(u64::try_from(character).unwrap_or(u64::MAX)),
            ..place.as_ref().clone()
        })
        .collect::<Vec<_>>();
    for destination in &destinations {
        let previous = vm.read_place(fiber, destination)?;
        if previous.value_type() != value.value_type() {
            return Err(VmError::InvalidArguments(
                "CVARSET value type differs".into(),
            ));
        }
    }
    for destination in destinations {
        vm.write_place(fiber, &destination, value.clone())?;
    }
    Ok(())
}

fn fill_value_or_default(arguments: &[VmValue], index: usize, default: VmValue) -> VmValue {
    match arguments.get(index) {
        None | Some(VmValue::Integer(i64::MIN)) => default,
        // Translated projects commonly use numeric zero to clear a string array.
        // Treat it as the string default instead of materializing the text "0".
        Some(VmValue::Integer(0)) if default.value_type() == BytecodeType::String => default,
        Some(value) => value.clone(),
    }
}

pub(in super::super) fn optional_nonnegative(
    arguments: &[VmValue],
    index: usize,
    default: usize,
    label: &str,
) -> Result<usize, VmError> {
    match integer_argument(arguments, index) {
        Err(_) | Ok(i64::MIN) => Ok(default),
        Ok(value) => usize::try_from(value)
            .map_err(|_| VmError::InvalidArguments(format!("{label} is negative"))),
    }
}

pub(in super::super) fn execute_find_element(
    vm: &mut Vm,
    fiber: &Fiber,
    last: bool,
    arguments: &[VmValue],
) -> Result<VmValue, VmError> {
    let place = array_place(arguments)?;
    let values = array_snapshot(vm, fiber, place)?;
    let needle = arguments
        .get(1)
        .ok_or_else(|| VmError::InvalidArguments("FINDELEMENT target is missing".into()))?;
    let start = match integer_argument(arguments, 2).unwrap_or(0) {
        i64::MIN => 0,
        value => usize::try_from(value)
            .map_err(|_| VmError::InvalidArguments("FINDELEMENT start is negative".into()))?,
    };
    let end = match integer_argument(arguments, 3).unwrap_or(i64::MIN) {
        i64::MIN => values.len(),
        value => usize::try_from(value)
            .map_err(|_| VmError::InvalidArguments("FINDELEMENT end is negative".into()))?,
    };
    if start > end || end > values.len() {
        return Err(VmError::InvalidArguments(
            "FINDELEMENT range is invalid".into(),
        ));
    }
    let exact = !matches!(integer_argument(arguments, 4), Ok(0) | Err(_));
    // FINDELEMENT treats the string needle as one regular expression for the
    // whole query. Compile it lazily so an empty range keeps its historical
    // no-op behavior, but do not rebuild the same automaton for every element.
    let mut compiled_regex = None;
    let mut matched = |value: &VmValue| -> Result<bool, VmError> {
        match (value, needle) {
            (VmValue::Integer(value), VmValue::Integer(needle)) => Ok(value == needle),
            (VmValue::String(value), VmValue::String(needle)) => {
                match crate::regex_compat::find_repeated_character(needle, value) {
                    crate::regex_compat::RepeatedCharacterMatch::Unsupported => {}
                    crate::regex_compat::RepeatedCharacterMatch::NoMatch => return Ok(false),
                    crate::regex_compat::RepeatedCharacterMatch::Match(matched) => {
                        return Ok(!exact || matched.len() == value.len());
                    }
                }
                if compiled_regex.is_none() {
                    compiled_regex = Some(
                        vm.compile_regex(needle)
                            .map_err(VmError::InvalidArguments)?,
                    );
                }
                let regex = compiled_regex
                    .as_ref()
                    .expect("the regex was initialized immediately above");
                Ok(regex
                    .find(value)
                    .is_some_and(|matched| !exact || matched.as_str().len() == value.len()))
            }
            _ => Err(VmError::InvalidArguments("FINDELEMENT types differ".into())),
        }
    };
    let range: Box<dyn Iterator<Item = usize>> = if last {
        Box::new((start..end).rev())
    } else {
        Box::new(start..end)
    };
    for index in range {
        if matched(&values[index])? {
            return Ok(VmValue::Integer(i64::try_from(index).unwrap_or(i64::MAX)));
        }
    }
    Ok(VmValue::Integer(-1))
}

#[allow(clippy::too_many_lines)]
pub(in super::super) fn execute_array_query(
    vm: &Vm,
    fiber: &Fiber,
    operation: &str,
    arguments: &[VmValue],
) -> Result<VmValue, VmError> {
    if matches!(operation, "groupmatch" | "nosames" | "allsames") {
        let Some(first) = arguments.first() else {
            return Err(VmError::InvalidArguments(format!(
                "{operation} requires at least two arguments"
            )));
        };
        if arguments.len() < 2
            || arguments
                .iter()
                .any(|value| value.value_type() != first.value_type())
        {
            return Err(VmError::InvalidArguments(format!(
                "{operation} arguments must have one value type"
            )));
        }
        let value = match operation {
            "groupmatch" => i64::try_from(
                arguments[1..]
                    .iter()
                    .filter(|candidate| *candidate == first)
                    .count(),
            )
            .unwrap_or(i64::MAX),
            "nosames" => i64::from(
                arguments
                    .iter()
                    .enumerate()
                    .all(|(index, value)| !arguments[..index].contains(value)),
            ),
            "allsames" => i64::from(arguments[1..].iter().all(|value| value == first)),
            _ => unreachable!(),
        };
        return Ok(VmValue::Integer(value));
    }

    let place = array_place(arguments)?;
    // CMATCH ranges over one selected field across characters, just like the
    // CARRAY family. Its first place is therefore intentionally indexed.
    let character_range = operation == "cmatch" || operation.contains("carray");
    let values = if character_range {
        character_series(vm, fiber, place)?
    } else {
        array_snapshot(vm, fiber, place)?
    };
    let (start_argument, end_argument) = if matches!(operation, "match" | "cmatch") {
        (2, 3)
    } else if matches!(operation, "inrangearray" | "inrangecarray") {
        (3, 4)
    } else {
        (1, 2)
    };
    let start = optional_index(arguments, start_argument, 0, operation)?;
    let end = optional_index(arguments, end_argument, values.len(), operation)?;
    if start > end || end > values.len() {
        return Err(VmError::InvalidArguments(format!(
            "{operation} range is invalid"
        )));
    }
    let range = &values[start..end];
    let result = match operation {
        "sumarray" | "sumcarray" => range.iter().try_fold(0i64, |sum, value| match value {
            VmValue::Integer(value) => Ok(sum.wrapping_add(*value)),
            _ => Err(VmError::InvalidArguments(format!(
                "{operation} requires an integer array"
            ))),
        })?,
        "maxarray" | "maxcarray" | "minarray" | "mincarray" => {
            let values = range
                .iter()
                .map(|value| match value {
                    VmValue::Integer(value) => Ok(*value),
                    _ => Err(VmError::InvalidArguments(format!(
                        "{operation} requires an integer array"
                    ))),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let value = if operation.starts_with("max") {
                values.into_iter().max()
            } else {
                values.into_iter().min()
            };
            value.ok_or_else(|| VmError::InvalidArguments(format!("{operation} range is empty")))?
        }
        "match" | "cmatch" => {
            let needle = arguments.get(1).ok_or_else(|| {
                VmError::InvalidArguments(format!("{operation} target is missing"))
            })?;
            if range
                .iter()
                .any(|candidate| candidate.value_type() != needle.value_type())
            {
                return Err(VmError::InvalidArguments(format!(
                    "{operation} target type differs"
                )));
            }
            i64::try_from(
                range
                    .iter()
                    .filter(|candidate| *candidate == needle)
                    .count(),
            )
            .unwrap_or(i64::MAX)
        }
        "inrangearray" | "inrangecarray" => {
            let minimum = integer_argument(arguments, 1)?;
            let maximum = integer_argument(arguments, 2)?;
            i64::try_from(
                range
                    .iter()
                    .filter(|value| {
                        matches!(value, VmValue::Integer(value) if *value >= minimum && *value <= maximum)
                    })
                    .count(),
            )
            .unwrap_or(i64::MAX)
        }
        _ => return Err(VmError::InvalidArguments("unknown array query".into())),
    };
    Ok(VmValue::Integer(result))
}

pub(in super::super) fn optional_index(
    arguments: &[VmValue],
    index: usize,
    default: usize,
    operation: &str,
) -> Result<usize, VmError> {
    match arguments.get(index) {
        None | Some(VmValue::Integer(i64::MIN)) => Ok(default),
        Some(VmValue::Integer(value)) => usize::try_from(*value).map_err(|_| {
            VmError::InvalidArguments(format!("{operation} range cannot be negative"))
        }),
        _ => Err(VmError::InvalidArguments(format!(
            "{operation} range must be integer"
        ))),
    }
}
