#[allow(clippy::wildcard_imports)]
use super::super::*;

pub(in super::super) fn commit_completion(
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

pub(in super::super) fn commit_integer_result(
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

pub(in super::super) fn commit_host_result_write(
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

pub(in super::super) fn global_place(vm: &RuntimeVm, name: &str) -> Option<PlaceDescriptor> {
    vm.vm().global_by_name(name).map(|global| PlaceDescriptor {
        variable: global.key,
        indices: vec![0; global.dimensions.len()],
        character: None,
        fiber: None,
        frame: None,
    })
}

pub(in super::super) fn global_place_at(
    vm: &RuntimeVm,
    name: &str,
    index: usize,
) -> Option<PlaceDescriptor> {
    let mut place = global_place(vm, name)?;
    let first = place.indices.first_mut()?;
    *first = u64::try_from(index).ok()?;
    Some(place)
}

pub(in super::super) fn enum_name_matches(operation: &str, candidate: &str, query: &str) -> bool {
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

pub(in super::super) fn make_bar(
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

pub(in super::super) fn named_color(name: &str) -> Option<i64> {
    erabasic_html::named_color(name).map(i64::from)
}

pub(in super::super) fn string_array_writes(
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

pub(in super::super) fn is_print(name: &str) -> bool {
    name.starts_with("PRINT") || name == "REUSELASTLINE"
}

pub(in super::super) fn print_uses_kana_conversion(name: &str) -> bool {
    name.starts_with("PRINT") && name.contains('K')
}

pub(in super::super) fn print_uses_default_color(name: &str) -> bool {
    name.starts_with("PRINT") && name.contains('D') && !name.starts_with("PRINTDATA")
}

pub(in super::super) fn is_input_command(name: &str) -> bool {
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

pub(in super::super) fn is_runtime_print_command(name: &str) -> bool {
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

pub(in super::super) fn column_print_alignment(name: &str) -> Option<CellAlignment> {
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

pub(in super::super) fn print_commits_line(name: &str) -> bool {
    name.ends_with('L') || name.ends_with('W')
}
