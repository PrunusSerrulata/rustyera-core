use erabasic_bytecode::BytecodeArtifact;

use super::ValidationContext;
use crate::{ValidationCode, ValidationDiagnostic};

pub(super) fn validate(
    artifact: &BytecodeArtifact,
    context: &ValidationContext,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    if artifact.runtime_native_authorizations.len() > 65_535 {
        diagnostics.push(ValidationDiagnostic::project(
            ValidationCode::ResourceLimit,
            "too many runtime Native authorizations",
        ));
        return;
    }
    let mut previous = None;
    for family in &artifact.runtime_native_authorizations {
        let symbol = artifact
            .runtime_builtins
            .iter()
            .find(|symbol| symbol.name.eq_ignore_ascii_case(&family.name));
        let valid = previous.is_none_or(|key| key < family.key)
            && family.key == family.canonical_key()
            && family.namespace == "rustyera.vm" && family.abi_version == context.native_abi
            && !family.name.starts_with("__") && !family.name.starts_with("dt__column_") && family.name == family.name.to_ascii_lowercase()
            && context.runtime_native_authorizations.get(&family.key) == Some(family)
            && symbol.is_some_and(|symbol| symbol.result == family.result && erabasic_bytecode::canonical_native_source_shapes(symbol) == family.shapes)
            // Classifying special VM providers cannot be weakened by an artifact.
            && family.contract == erabasic_bytecode::canonical_native_contract(&family.name);
        if !valid {
            diagnostics.push(ValidationDiagnostic::project(
                ValidationCode::HostAbiMismatch,
                format!(
                    "runtime Native family {} is not authorized by this validation context",
                    family.name
                ),
            ));
        }
        previous = Some(family.key);
    }
}
