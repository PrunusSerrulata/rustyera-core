use std::collections::BTreeMap;

use erabasic_hir::SemanticType;

use super::{ArgumentConstraint, CallableSignature};

mod core;
mod fallbacks;
mod graphics;
mod host;
mod structured;

use fallbacks::{INTEGER_FALLBACKS, STRING_FALLBACKS};

pub(super) fn builtin_functions() -> BTreeMap<String, CallableSignature> {
    use ArgumentConstraint::Any;
    use SemanticType::{Integer as IntType, String as StrType};

    let mut result = BTreeMap::new();
    {
        let mut catalog = FunctionCatalog {
            entries: &mut result,
        };
        core::register(&mut catalog);
        host::register(&mut catalog);
        structured::register(&mut catalog);
        graphics::register(&mut catalog);
    }

    for name in [
        "SPRITECREATEFROMFILE",
        "CBGSETSPRITE",
        "BITSET",
        "BITGET",
        "BITTOGGLE",
        "BITINDEXOFFIRST",
        "STRJOIN",
        "RAND",
        "SQL_CONNECT",
        "SQL_P_EXECUTE_NONQUERY",
        "SQL_P_EXECUTE_READER",
        "SQL_P_EXECUTE_SCALAR_LONG",
        "SQL_P_EXECUTE_SCALAR_STRING",
        "SQL_P_EXECUTE_SCALAR_FLOAT",
        "GETMETH",
        "GETMETHS",
        "FINDELEMENT",
        "FINDLASTELEMENT",
        "MATCHALL",
        "MATCHALLEX",
        "SUBSTRING",
        "SUBSTRINGU",
        "ENCODETOUNI",
        "VARSETEX",
        "DT_SELECT",
        "DT_ROW_ADD",
        "DT_ROW_SET",
        "DT_CELL_SET",
    ] {
        result
            .get_mut(name)
            .expect("omittable function signature was inserted")
            .allow_omitted = true;
    }
    for name in INTEGER_FALLBACKS {
        result
            .entry((*name).to_owned())
            .or_insert_with(|| CallableSignature {
                name: (*name).to_owned(),
                return_type: IntType,
                arguments: vec![Any],
                minimum_arguments: 0,
                variadic: true,
                allow_omitted: true,
            });
    }
    for name in STRING_FALLBACKS {
        result
            .entry((*name).to_owned())
            .or_insert_with(|| CallableSignature {
                name: (*name).to_owned(),
                return_type: StrType,
                arguments: vec![Any],
                minimum_arguments: 0,
                variadic: true,
                allow_omitted: true,
            });
    }
    result
}

pub(super) struct FunctionCatalog<'a> {
    entries: &'a mut BTreeMap<String, CallableSignature>,
}

impl FunctionCatalog<'_> {
    pub(super) fn add(
        &mut self,
        name: &str,
        return_type: SemanticType,
        arguments: &[ArgumentConstraint],
        minimum_arguments: usize,
        variadic: bool,
    ) {
        self.entries.insert(
            name.to_owned(),
            CallableSignature {
                name: name.to_owned(),
                return_type,
                arguments: arguments.to_vec(),
                minimum_arguments,
                variadic,
                allow_omitted: false,
            },
        );
    }
}
