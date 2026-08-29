//! A source plan consumes trusted stage grants; parse symbols alone never grant execution.
use super::{StepError, VmFaultCode, support};
use erabasic_bytecode::{RuntimeExpressionShape, RuntimeStagedKind};
pub(super) fn authorize(
    program: &crate::ProgramGeneration,
    name: &str,
    expected: RuntimeStagedKind,
    shapes: &[Option<RuntimeExpressionShape>],
) -> Result<(), StepError> {
    if !program
        .artifact
        .manifest
        .compatibility
        .supports_snake_data_apis()
    {
        return Err(support::permission_denied(
            "staged array operation is unavailable in this identity",
        ));
    }
    let family = program
        .artifact
        .runtime_staged_authorizations
        .iter()
        .find(|family| family.name.eq_ignore_ascii_case(name) && family.kind == expected)
        .ok_or_else(|| {
            support::permission_denied(format!(
                "runtime staged operation {name} lacks trusted authorization"
            ))
        })?;
    if !family.accepts(shapes) {
        return Err(StepError::script(
            crate::ScriptFaultKind::Argument,
            VmFaultCode::TypeMismatch,
            format!("runtime staged operation {name} has incompatible arguments"),
        ));
    }
    Ok(())
}
