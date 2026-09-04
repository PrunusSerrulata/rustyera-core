//! Pure source-to-physical argument mapping; no service authority or evaluation.
use crate::{BytecodeType, RuntimeCallableShape, RuntimeExpressionShape};
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeSourceBinding {
    pub parameters: Vec<BytecodeType>,
    pub omitted_arguments: Vec<usize>,
}
/// Native and Host callers supply their independently authorized source shape and
/// place policy. Missing tail slots are never invented; explicit omission retains
/// presence metadata alongside the existing Integer MIN physical sentinel.
#[must_use]
pub fn bind_runtime_source_arguments(
    shape: &RuntimeCallableShape,
    actuals: &[Option<RuntimeExpressionShape>],
    keeps_place: impl Fn(usize, RuntimeExpressionShape) -> bool,
) -> Option<RuntimeSourceBinding> {
    if !shape.accepts(actuals) || actuals.len() > 65_535 {
        return None;
    }
    let mut omitted_arguments = Vec::new();
    let parameters = actuals
        .iter()
        .enumerate()
        .map(|(index, actual)| {
            let Some(actual) = actual else {
                omitted_arguments.push(index);
                return Some(BytecodeType::Integer);
            };
            Some(if actual.variable && keeps_place(index, *actual) {
                match actual.value_type {
                    BytecodeType::Integer => BytecodeType::IntegerPlace,
                    BytecodeType::String => BytecodeType::StringPlace,
                    _ => return None,
                }
            } else {
                actual.value_type
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(RuntimeSourceBinding {
        parameters,
        omitted_arguments,
    })
}
