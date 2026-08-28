//! Serialize catalog signatures, not only the Native/Host imports reached by codegen.
use erabasic_analyzer::{ArgumentConstraint as A, CallableSignature};
use erabasic_bytecode::{
    RuntimeArgumentConstraint as B, RuntimeBuiltinSymbol, RuntimeCallableShape,
};

pub(super) fn runtime_builtin_symbols(
    signatures: impl IntoIterator<Item = CallableSignature>,
) -> Vec<RuntimeBuiltinSymbol> {
    let mut symbols = signatures
        .into_iter()
        .filter_map(|signature| {
            let result = crate::lowering::bytecode_type(signature.return_type)?;
            let mut shapes = Vec::new();
            // Materialize catalog arity-specific contracts (e.g. XML_REPLACE's short form)
            // here, so the VM never recognizes builtin names with its own rule table.
            for arity in signature.minimum_arguments..=signature.arguments.len() {
                shapes.push(RuntimeCallableShape {
                    minimum: arity,
                    maximum: Some(arity),
                    omitted_from: signature.minimum_arguments,
                    arguments: signature
                        .arguments_for_arity(arity)
                        .iter()
                        .copied()
                        .map(constraint)
                        .collect(),
                    allow_omitted: signature.allow_omitted,
                });
            }
            if signature.variadic {
                shapes.push(RuntimeCallableShape {
                    minimum: signature.arguments.len().max(signature.minimum_arguments),
                    maximum: None,
                    omitted_from: signature.minimum_arguments,
                    arguments: signature
                        .arguments
                        .iter()
                        .copied()
                        .map(constraint)
                        .collect(),
                    allow_omitted: signature.allow_omitted,
                });
            }
            Some(RuntimeBuiltinSymbol {
                name: signature.name,
                result,
                shapes,
            })
        })
        .collect::<Vec<_>>();
    symbols.sort_by(|left, right| left.name.cmp(&right.name));
    symbols
}
fn constraint(value: A) -> B {
    match value {
        A::Integer => B::Integer,
        A::String => B::String,
        A::Any => B::Any,
        A::MutableInteger => B::MutableInteger,
        A::MutableString => B::MutableString,
        A::MutableAny => B::MutableAny,
        A::ReferenceAny => B::ReferenceAny,
        A::ReferenceOrString => B::ReferenceOrString,
        A::MutableReferenceOrString => B::MutableReferenceOrString,
        A::IntegerOrReference => B::IntegerOrReference,
        A::IntegerOrMutableString => B::IntegerOrMutableString,
        A::Formatted => B::Formatted,
        A::Raw => B::Raw,
    }
}
