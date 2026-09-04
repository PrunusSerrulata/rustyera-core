use erabasic_hir::SemanticType::{Error as ErrorType, Integer as IntType, String as StrType};

use super::{
    ArgumentConstraint::{Integer, String},
    FunctionCatalog,
};

pub(super) fn register(catalog: &mut FunctionCatalog<'_>) {
    register_input_and_platform(catalog);
    register_sql(catalog);
    register_deferred_sql(catalog);
}

fn register_input_and_platform(catalog: &mut FunctionCatalog<'_>) {
    // GETKEY is deliberately non-constant in Emuera and accepts exactly one
    // integer virtual-key code. HIR calls are never folded, so the signature is
    // sufficient to retain that behavior in the current analyzer.
    catalog.add("GETKEY", IntType, &[Integer], 1, false);
    catalog.add("GETKEYTRIGGERED", IntType, &[Integer], 1, false);
    catalog.add("SEQUENCEINPUT", IntType, &[String], 1, false);
    catalog.add("DISABLE_INPUT_MACRO", IntType, &[], 0, false);
    catalog.add("ENABLE_INPUT_MACRO", IntType, &[], 0, false);
    catalog.add("ENV_HAS_CAPABILITY", IntType, &[String, Integer], 1, false);
    catalog.add("GETPLATFORM", IntType, &[], 0, false);
    catalog.add("GETTEXTBOX", StrType, &[], 0, false);
    catalog.add("SETTEXTBOX", IntType, &[String], 1, false);
    catalog.add("HOTKEY_STATE_INIT", IntType, &[Integer], 1, false);
    catalog.add("HOTKEY_STATE", IntType, &[Integer, Integer], 2, false);
    catalog.add(
        "FLOWINPUT",
        IntType,
        &[Integer, Integer, Integer, Integer],
        1,
        false,
    );
    catalog.add("FLOWINPUTS", IntType, &[Integer, String], 1, false);
    catalog.add("MOUSEX", IntType, &[], 0, false);
    catalog.add("MOUSEY", IntType, &[], 0, false);
    catalog.add("MOUSEB", StrType, &[], 0, false);
}

fn register_sql(catalog: &mut FunctionCatalog<'_>) {
    // Safe SQL is a snake-profile Host service. Parameterized calls keep the third String
    // constraint as the repeated variadic tail; omitted tail slots become SQL NULL at runtime.
    catalog.add("SQL_CONNECT", IntType, &[String, String], 1, false);
    for name in ["SQL_DISCONNECT", "SQL_READER_READ", "SQL_READER_CLOSE"] {
        catalog.add(
            name,
            IntType,
            &[if name == "SQL_DISCONNECT" {
                String
            } else {
                Integer
            }],
            1,
            false,
        );
    }
    for name in [
        "SQL_EXECUTE_NONQUERY",
        "SQL_EXECUTE_READER",
        "SQL_EXECUTE_SCALAR_LONG",
    ] {
        catalog.add(name, IntType, &[String, String], 2, false);
    }
    catalog.add(
        "SQL_EXECUTE_SCALAR_STRING",
        StrType,
        &[String, String],
        2,
        false,
    );
    for name in ["SQL_READER_GET_LONG", "SQL_READER_ISNULL"] {
        catalog.add(name, IntType, &[Integer, Integer], 2, false);
    }
    catalog.add(
        "SQL_READER_GET_STRING",
        StrType,
        &[Integer, Integer],
        2,
        false,
    );
    catalog.add(
        "SQL_IMPORT_MAP_XML",
        IntType,
        &[String, String, String],
        3,
        false,
    );
    for (name, result_type) in [
        ("SQL_P_EXECUTE_NONQUERY", IntType),
        ("SQL_P_EXECUTE_READER", IntType),
        ("SQL_P_EXECUTE_SCALAR_LONG", IntType),
        ("SQL_P_EXECUTE_SCALAR_STRING", StrType),
    ] {
        catalog.add(name, result_type, &[String, String, String], 2, true);
    }
}

fn register_deferred_sql(catalog: &mut FunctionCatalog<'_>) {
    // Deferred SQL names remain known to the catalog so the compiler can return one stable
    // missing-capability diagnostic rather than misclassifying them as unknown functions.
    catalog.add("SQL_CONNECTION_OPEN", IntType, &[String], 1, false);
    catalog.add("SQL_ESCAPE", StrType, &[String], 1, false);
    catalog.add(
        "SQL_READER_GET_FLOAT",
        ErrorType,
        &[Integer, Integer],
        2,
        false,
    );
    catalog.add(
        "SQL_EXECUTE_SCALAR_FLOAT",
        ErrorType,
        &[String, String],
        2,
        false,
    );
    catalog.add(
        "SQL_P_EXECUTE_SCALAR_FLOAT",
        ErrorType,
        &[String, String, String],
        2,
        true,
    );
    catalog.add(
        "SQL_IMPORT_DT_XML",
        IntType,
        &[String, String, String, String],
        4,
        false,
    );
    catalog.add(
        "SQL_EXPORT_MAP_XML",
        IntType,
        &[String, String, String],
        3,
        false,
    );
    catalog.add(
        "SQL_EXPORT_DT_XML",
        IntType,
        &[String, String, String, String],
        4,
        false,
    );
    catalog.add(
        "SQL_IMPORT_XML_CUSTOM",
        IntType,
        &[String, String, String, String, String],
        5,
        false,
    );
}
