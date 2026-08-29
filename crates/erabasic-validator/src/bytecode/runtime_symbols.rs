//! Bounds and canonical shape checks for the complete parse-time builtin namespace.
use erabasic_bytecode::{BytecodeType, RuntimeBuiltinSymbol};

pub(super) fn validate_runtime_builtins(symbols: &[RuntimeBuiltinSymbol]) -> Result<(), String> {
    if symbols.len() > 65_535 {
        return Err("runtime builtin namespace is too large".into());
    }
    let mut previous: Option<&str> = None;
    for symbol in symbols {
        if symbol.name.is_empty()
            || symbol.name.len() > 1024
            || symbol.name != symbol.name.to_ascii_uppercase()
            || previous.is_some_and(|previous| previous >= symbol.name.as_str())
            || !matches!(symbol.result, BytecodeType::Integer | BytecodeType::String)
            || symbol.shapes.is_empty()
            || symbol.shapes.len() > 65_535
        {
            return Err("runtime builtin symbol is noncanonical or invalid".into());
        }
        for shape in &symbol.shapes {
            if shape.minimum > 65_535
                || shape.arguments.len() > 65_535
                || shape.omitted_from > 65_535
                || shape.maximum.is_some_and(|maximum| {
                    maximum < shape.minimum || maximum > shape.arguments.len()
                })
                || (shape.maximum.is_none() && shape.arguments.is_empty())
            {
                return Err("runtime builtin callable shape exceeds its bounds".into());
            }
        }
        previous = Some(&symbol.name);
    }
    Ok(())
}

pub(super) fn validate_runtime_variables(
    artifact: &erabasic_bytecode::BytecodeArtifact,
) -> Result<(), String> {
    use erabasic_bytecode::{BytecodeStorage, CharacterArrayDisposal};
    if artifact.runtime_variables.len() != artifact.globals.len() {
        return Err("runtime variable metadata does not exactly cover globals".into());
    }
    let globals = artifact
        .globals
        .iter()
        .map(|global| (global.key, global))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut previous = None;
    for symbol in &artifact.runtime_variables {
        if previous.is_some_and(|previous| previous >= symbol.key) {
            return Err("runtime variable metadata is not sorted and unique".into());
        }
        previous = Some(symbol.key);
        let definition = globals
            .get(&symbol.key)
            .copied()
            .ok_or("runtime variable metadata key is not a global")?;
        if symbol.reference
            && (!definition.mutable
                || definition.storage != BytecodeStorage::FunctionLocal
                || definition.owner.is_none()
                || !(1..=3).contains(&definition.dimensions.len())
                || !matches!(
                    definition.value_type,
                    BytecodeType::Integer | BytecodeType::String
                )
                || symbol.match_name_rejection.is_some())
        {
            return Err("runtime REF metadata has an invalid declaration shape".into());
        }
        if (symbol.reference_semantics.can_restructure && !symbol.reference_semantics.is_const)
            || (symbol.reference && symbol.reference_semantics.is_const)
        {
            return Err("runtime reference token semantics are inconsistent".into());
        }
        let sparse = definition.owner.is_none()
            && definition.storage == BytecodeStorage::Character
            && definition.dimensions.len() == 1
            && artifact
                .project_data
                .schema
                .variable(&definition.name)
                .is_some_and(|schema| matches!(&schema.id, erabasic_data::VariableId::Builtin(_)));
        let expected_disposal = if sparse {
            CharacterArrayDisposal::ClearSparse
        } else {
            CharacterArrayDisposal::Preserve
        };
        if symbol.character_disposal != expected_disposal || (sparse && symbol.reference) {
            return Err("runtime character disposal does not match its declaration source".into());
        }
    }
    for function in &artifact.functions {
        for parameter in &function.parameters {
            let metadata = artifact
                .runtime_variables
                .binary_search_by_key(&parameter.key, |symbol| symbol.key)
                .ok()
                .map(|index| &artifact.runtime_variables[index])
                .ok_or("formal lacks variable metadata")?;
            if parameter.by_reference != metadata.reference {
                return Err("formal and runtime variable REF metadata disagree".into());
            }
        }
    }
    Ok(())
}
