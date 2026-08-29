//! Fixed scalar Native source domains. The analyzer catalog may deliberately be broader.
//! Integer/String is the supported domain here; Float overloads are not authorized.
use crate::{
    BytecodeType, RuntimeArgumentConstraint as C, RuntimeBuiltinSymbol, RuntimeCallableShape,
    RuntimeExpressionShape,
};

#[must_use]
pub fn canonical_native_source_shapes(symbol: &RuntimeBuiltinSymbol) -> Vec<RuntimeCallableShape> {
    let shape = |arguments: &[C], minimum: usize, variadic: bool| RuntimeCallableShape {
        minimum,
        maximum: (!variadic).then_some(arguments.len()),
        omitted_from: if variadic { usize::MAX } else { minimum },
        arguments: arguments.to_vec(),
        allow_omitted: false,
    };
    let exact = |arguments: &[C]| shape(arguments, arguments.len(), false);
    let value = match symbol.name.to_ascii_lowercase().as_str() {
        // TOSTR is normally Host; an explicitly selected CoreNative provider
        // remains integer-only and must not ignore a supplied format argument.
        "abs" | "sign" | "sqrt" | "cbrt" | "log" | "log10" | "exponent" | "bitcount"
        | "unchecked_neg" | "unicode" | "unicodetostr" | "tostr" => exact(&[C::Integer]),
        "power" | "getbit" | "convert" | "getpalamlv" | "getexplv" | "unchecked_add"
        | "unchecked_sub" | "unchecked_mul" => exact(&[C::Integer, C::Integer]),
        "limit" | "inrange" | "color_fromrgb" => exact(&[C::Integer, C::Integer, C::Integer]),
        "strlen" | "strlenu" | "strlens" | "strlensu" | "strform" | "strformcheck" | "toint"
        | "isnumeric" | "escape" | "unicodebyte" | "tolower" | "toupper" | "color_fromname"
        | "existmeth" => exact(&[C::String]),
        "max" | "min" => shape(&[C::Integer], 1, true),
        "substring" | "substringu" => shape(&[C::String, C::Integer, C::Integer], 1, false),
        "strfind" | "strfindu" => shape(&[C::String, C::String, C::Integer], 2, false),
        "strcount" => exact(&[C::String, C::String]),
        "encodetouni" => shape(&[C::String, C::Integer], 1, false),
        "charatu" => exact(&[C::String, C::Integer]),
        "replace" => shape(
            &[C::String, C::String, C::ReferenceOrString, C::Integer],
            3,
            false,
        ),
        _ => return symbol.shapes.clone(),
    };
    vec![value]
}
#[must_use]
pub fn native_source_relations(name: &str, actuals: &[Option<RuntimeExpressionShape>]) -> bool {
    // ReferenceOrString alone also accepts Integer variables. REPLACE does not.
    !name.eq_ignore_ascii_case("replace")
        || actuals
            .get(2)
            .and_then(Option::as_ref)
            .is_some_and(|argument| argument.value_type == BytecodeType::String)
}
