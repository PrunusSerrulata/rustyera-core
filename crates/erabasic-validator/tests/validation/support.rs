use erabasic_bytecode::BytecodeArtifact;
use erabasic_csv::{CsvLoadOptions, ProjectFiles, load_project};

pub(super) fn project_data() -> erabasic_data::ProjectData {
    load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
        .data
        .expect("default project data should load")
}

pub(super) fn fixture_runtime_variables(artifact: &mut BytecodeArtifact) {
    let previous = std::mem::take(&mut artifact.runtime_variables);
    artifact.runtime_variables = artifact
        .globals
        .iter()
        .map(|global| {
            let reference = artifact
                .functions
                .iter()
                .flat_map(|function| &function.parameters)
                .any(|formal| formal.key == global.key && formal.by_reference);
            let sparse = global.owner.is_none()
                && global.storage == erabasic_bytecode::BytecodeStorage::Character
                && global.dimensions.len() == 1
                && artifact
                    .project_data
                    .schema
                    .variable(&global.name)
                    .is_some_and(|schema| {
                        matches!(&schema.id, erabasic_data::VariableId::Builtin(_))
                    });
            erabasic_bytecode::RuntimeVariableSymbol {
                reference_semantics: previous
                    .iter()
                    .find(|symbol| symbol.key == global.key)
                    .map_or(
                        erabasic_bytecode::RuntimeReferenceSemantics {
                            is_const: false,
                            can_restructure: false,
                        },
                        |symbol| symbol.reference_semantics,
                    ),
                key: global.key,
                reference,
                match_name_rejection: None,
                character_disposal: if sparse {
                    erabasic_bytecode::CharacterArrayDisposal::ClearSparse
                } else {
                    erabasic_bytecode::CharacterArrayDisposal::Preserve
                },
            }
        })
        .collect();
}
