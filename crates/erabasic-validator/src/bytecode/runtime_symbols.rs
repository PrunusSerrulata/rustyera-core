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
