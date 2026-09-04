use super::ValidationContext;
use crate::{ValidationCode, ValidationDiagnostic};
use erabasic_bytecode::{BytecodeArtifact, BytecodeType, RuntimeStagedKind};
use std::collections::BTreeMap;

pub(super) fn require(
    artifact: &BTreeMap<
        erabasic_bytecode::SymbolKey,
        &erabasic_bytecode::RuntimeStagedAuthorization,
    >,
    trusted: &BTreeMap<erabasic_bytecode::SymbolKey, erabasic_bytecode::RuntimeStagedAuthorization>,
    name: &str,
    kind: RuntimeStagedKind,
    shapes: &[Option<erabasic_bytecode::RuntimeExpressionShape>],
) -> Result<(), (ValidationCode, String)> {
    let family = artifact
        .values()
        .copied()
        .find(|family| family.name.eq_ignore_ascii_case(name) && family.kind == kind)
        .ok_or_else(|| {
            (
                ValidationCode::HostAbiMismatch,
                format!("runtime staged operation {name} lacks artifact authorization"),
            )
        })?;
    if trusted.get(&family.key) != Some(family) {
        return Err((
            ValidationCode::HostAbiMismatch,
            format!("runtime staged operation {name} lacks trusted authorization"),
        ));
    }
    if !family.accepts(shapes) {
        return Err((
            ValidationCode::InvalidOperand,
            format!("runtime staged operation {name} has incompatible arguments"),
        ));
    }
    Ok(())
}
pub(super) fn validate(
    artifact: &BytecodeArtifact,
    context: &ValidationContext,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    if artifact.runtime_staged_authorizations.len() > 6 {
        diagnostics.push(ValidationDiagnostic::project(
            ValidationCode::ResourceLimit,
            "too many staged VM authorizations",
        ));
        return;
    }
    let mut previous = None;
    for family in &artifact.runtime_staged_authorizations {
        let symbol = artifact
            .runtime_builtins
            .iter()
            .find(|symbol| symbol.name.eq_ignore_ascii_case(&family.name));
        let valid = artifact.manifest.compatibility.supports_snake_data_apis()
            && previous.is_none_or(|key| key < family.key)
            && family.key == family.canonical_key()
            && RuntimeStagedKind::from_name(&family.name) == Some(family.kind)
            && family.name == family.name.to_ascii_lowercase()
            && family.result == BytecodeType::Integer
            && context.runtime_staged_authorizations.get(&family.key) == Some(family)
            && symbol.is_some_and(|symbol| {
                symbol.result == family.result && symbol.shapes == family.shapes
            });
        if !valid {
            diagnostics.push(ValidationDiagnostic::project(
                ValidationCode::HostAbiMismatch,
                format!(
                    "runtime staged operation {} is not authorized by this validation context",
                    family.name
                ),
            ));
        }
        previous = Some(family.key);
    }
}
