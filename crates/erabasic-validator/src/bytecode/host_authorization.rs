use super::ValidationContext;
use crate::{ValidationCode, ValidationDiagnostic};
use erabasic_bytecode::BytecodeArtifact;
pub(super) fn validate(
    artifact: &BytecodeArtifact,
    context: &ValidationContext,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    if artifact.runtime_host_authorizations.len() > 65_535 {
        diagnostics.push(ValidationDiagnostic::project(
            ValidationCode::ResourceLimit,
            "too many runtime Host authorizations",
        ));
        return;
    }
    let snake = artifact.manifest.compatibility.profile.as_str() == "emuera.skia.snake";
    let mut previous = None;
    for family in &artifact.runtime_host_authorizations {
        let symbol = artifact
            .runtime_builtins
            .iter()
            .find(|symbol| symbol.name.eq_ignore_ascii_case(&family.name));
        let valid = previous.is_none_or(|key| key < family.key)
            && family.key == family.canonical_key()
            && family.name == family.name.to_ascii_lowercase()
            && context.runtime_host_authorizations.get(&family.key) == Some(family)
            && symbol.is_some_and(|symbol| symbol.result == family.result)
            && erabasic_bytecode::canonical_host_source_shapes(&family.name, snake).as_ref()
                == Some(&family.shapes)
            && std::iter::once(&family.prototype)
                .chain(family.stages.iter().map(|(_, import)| import))
                .all(|import| {
                    import.effect == import.contract.effect()
                        && import.snapshot_capability == import.contract.snapshot_capability()
                        && context.host_capabilities.contains(&import.capability)
                });
        if !valid {
            diagnostics.push(ValidationDiagnostic::project(
                ValidationCode::HostAbiMismatch,
                format!(
                    "runtime Host family {} is not authorized by this validation context",
                    family.name
                ),
            ));
        }
        previous = Some(family.key);
    }
}
