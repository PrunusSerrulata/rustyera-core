#[allow(clippy::wildcard_imports)]
use super::*;
#[allow(
    clippy::too_many_lines,
    reason = "range validation, revision caching, and regex compatibility form one ordered query"
)]
pub(in super::super) fn execute_find_element(
    vm: &mut Vm,
    fiber: &Fiber,
    last: bool,
    arguments: &[VmValue],
) -> Result<VmValue, VmError> {
    let place = array_place(arguments)?;
    let mut array = place.clone();
    array.indices.clear();
    let array_len = vm.place_array_len(fiber, &array)?;
    let needle = arguments
        .get(1)
        .ok_or_else(|| VmError::InvalidArguments("FINDELEMENT target is missing".into()))?;
    let start = match optional_integer_argument(arguments, 2, 0)? {
        i64::MIN => 0,
        value => usize::try_from(value).map_err(|_| {
            script_native_error(
                crate::ScriptFaultKind::Bounds,
                "FINDELEMENT start is negative".into(),
            )
        })?,
    };
    let end = match optional_integer_argument(arguments, 3, i64::MIN)? {
        i64::MIN => array_len,
        value => usize::try_from(value).map_err(|_| {
            script_native_error(
                crate::ScriptFaultKind::Bounds,
                "FINDELEMENT end is negative".into(),
            )
        })?,
    };
    if start > end || end > array_len {
        return Err(script_native_error(
            crate::ScriptFaultKind::Bounds,
            "FINDELEMENT range is invalid".into(),
        ));
    }
    let exact = optional_integer_argument(arguments, 4, 0)? != 0;
    let array_revision = vm.place_array_revision(fiber, &array)?;
    let cache_key = array_revision
        .as_ref()
        .and_then(|(generation, variable, revision)| {
            let needle = match needle {
                VmValue::Integer(value) => FindElementNeedle::Integer(*value),
                VmValue::String(value) => FindElementNeedle::String(value.clone()),
                VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => return None,
            };
            Some(FindElementCacheKey {
                generation: *generation,
                variable: *variable,
                revision: *revision,
                start,
                end,
                last,
                exact,
                needle,
            })
        });
    let path_memo_active = vm.path_memo_is_active_for(fiber.id);
    let revision_covers_read = path_memo_active
        && array_revision
            .as_ref()
            .is_some_and(|(generation, variable, revision)| {
                vm.observe_path_memo_cell_revision(fiber.id, *generation, *variable, *revision)
            });
    if (!path_memo_active || revision_covers_read)
        && let Some(result) = cache_key
            .as_ref()
            .and_then(|key| vm.find_element_cache.get(key))
    {
        return Ok(VmValue::Integer(*result));
    }
    // Mutation observers invalidate a trace that later writes this cell, so one
    // revision dependency safely covers both a cache hit and the whole scan.
    let values = if revision_covers_read {
        vm.read_place_array_range_unobserved(fiber, &array, start, end)?
    } else {
        vm.read_place_array_range(fiber, &array, start, end)?
    };
    validate_array_storage_values(vm, fiber, &array, &values)?;
    // FINDELEMENT treats the string needle as one regular expression for the
    // whole query. Compile it lazily so an empty range keeps its historical
    // no-op behavior, but do not rebuild the same automaton for every element.
    let mut compiled_regex = None;
    let mut matched = |value: &VmValue| -> Result<bool, VmError> {
        match (value, needle) {
            (VmValue::Integer(value), VmValue::Integer(needle)) => Ok(value == needle),
            (VmValue::String(value), VmValue::String(needle)) => {
                if !needle
                    .bytes()
                    .any(|byte| b"\\.^$*+?()[]{}|".contains(&byte))
                {
                    return Ok(if exact {
                        value == needle
                    } else {
                        value.contains(needle)
                    });
                }
                match crate::regex_compat::find_repeated_character(needle, value) {
                    crate::regex_compat::RepeatedCharacterMatch::Unsupported => {}
                    crate::regex_compat::RepeatedCharacterMatch::NoMatch => return Ok(false),
                    crate::regex_compat::RepeatedCharacterMatch::Match(matched) => {
                        return Ok(!exact || matched.len() == value.len());
                    }
                }
                if compiled_regex.is_none() {
                    compiled_regex =
                        Some(vm.compile_regex(needle).map_err(VmError::ScriptFailure)?);
                }
                let regex = compiled_regex
                    .as_ref()
                    .expect("the regex was initialized immediately above");
                Ok(regex
                    .find(value)
                    .is_some_and(|matched| !exact || matched.as_str().len() == value.len()))
            }
            (_, VmValue::Integer(_) | VmValue::String(_)) => Err(script_native_error(
                crate::ScriptFaultKind::Argument,
                "FINDELEMENT types differ".into(),
            )),
            _ => Err(VmError::InvalidArguments("FINDELEMENT types differ".into())),
        }
    };
    let range: Box<dyn Iterator<Item = usize>> = if last {
        Box::new((0..values.len()).rev())
    } else {
        Box::new(0..values.len())
    };
    let mut result = -1;
    for index in range {
        if matched(&values[index])? {
            result = i64::try_from(start.saturating_add(index)).unwrap_or(i64::MAX);
            break;
        }
    }
    if let Some(key) = cache_key {
        vm.cache_find_element(key, result);
    }
    Ok(VmValue::Integer(result))
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
                .any(|value| matches!(value, VmValue::IntegerPlace(_) | VmValue::StringPlace(_)))
        {
            return Err(VmError::InvalidArguments(format!(
                "{operation} arguments must have one value type"
            )));
        }
        if arguments
            .iter()
            .any(|value| value.value_type() != first.value_type())
        {
            return Err(script_native_error(
                crate::ScriptFaultKind::Argument,
                format!("{operation} arguments must have one value type"),
            ));
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
    validate_array_storage_values(vm, fiber, place, &values)?;
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
        return Err(script_native_error(
            crate::ScriptFaultKind::Bounds,
            format!("{operation} range is invalid"),
        ));
    }
    let range = &values[start..end];
    let result = match operation {
        "sumarray" | "sumcarray" => range.iter().try_fold(0i64, |sum, value| match value {
            VmValue::Integer(value) => Ok(sum.wrapping_add(*value)),
            _ => Err(script_native_error(
                crate::ScriptFaultKind::Argument,
                format!("{operation} requires an integer array"),
            )),
        })?,
        "maxarray" | "maxcarray" | "minarray" | "mincarray" => {
            let values = range
                .iter()
                .map(|value| match value {
                    VmValue::Integer(value) => Ok(*value),
                    _ => Err(script_native_error(
                        crate::ScriptFaultKind::Argument,
                        format!("{operation} requires an integer array"),
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let value = if operation.starts_with("max") {
                values.into_iter().max()
            } else {
                values.into_iter().min()
            };
            value.ok_or_else(|| {
                script_native_error(
                    crate::ScriptFaultKind::Bounds,
                    format!("{operation} range is empty"),
                )
            })?
        }
        "match" | "cmatch" => {
            let needle = arguments.get(1).ok_or_else(|| {
                VmError::InvalidArguments(format!("{operation} target is missing"))
            })?;
            if matches!(needle, VmValue::IntegerPlace(_) | VmValue::StringPlace(_)) {
                return Err(VmError::InvalidArguments(format!(
                    "{operation} target type differs"
                )));
            }
            if range
                .iter()
                .any(|candidate| candidate.value_type() != needle.value_type())
            {
                return Err(script_native_error(
                    crate::ScriptFaultKind::Argument,
                    format!("{operation} target type differs"),
                ));
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
            script_native_error(
                crate::ScriptFaultKind::Bounds,
                format!("{operation} range cannot be negative"),
            )
        }),
        _ => Err(VmError::InvalidArguments(format!(
            "{operation} range must be integer"
        ))),
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;

    #[test]
    fn optional_array_bounds_do_not_swallow_physical_argument_failures() {
        assert_eq!(optional_nonnegative(&[], 0, 7, "range").unwrap(), 7);
        assert_eq!(
            optional_nonnegative(&[VmValue::Integer(i64::MIN)], 0, 7, "range").unwrap(),
            7
        );
        let failure = optional_nonnegative(&[VmValue::Integer(-1)], 0, 7, "range").unwrap_err();
        assert!(matches!(
            failure,
            VmError::ScriptFailure(crate::ExecutionFailure {
                category: crate::FaultCategory::Script(crate::ScriptFaultKind::Bounds),
                ..
            })
        ));
        let failure = optional_nonnegative(
            &[VmValue::String("range is negative".into())],
            0,
            7,
            "range",
        )
        .unwrap_err();
        assert!(matches!(failure, VmError::InvalidArguments(_)));
        assert!(matches!(
            optional_integer_argument(&[VmValue::String("0".into())], 0, 0),
            Err(VmError::InvalidArguments(_))
        ));
    }
}
