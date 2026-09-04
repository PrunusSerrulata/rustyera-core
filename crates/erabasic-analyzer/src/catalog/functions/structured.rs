use erabasic_hir::SemanticType::{Integer as IntType, String as StrType};

use super::{
    ArgumentConstraint::{
        Any, Integer, IntegerOrMutableString, MutableString, ReferenceAny, String,
    },
    FunctionCatalog,
};

pub(super) fn register(catalog: &mut FunctionCatalog<'_>) {
    register_maps(catalog);
    register_xml(catalog);
    register_data_tables(catalog);
}

fn register_maps(catalog: &mut FunctionCatalog<'_>) {
    // Structured native functions are declared explicitly. The older fallback
    // catalog accepted any arity and hid both reference mistakes and missing
    // output places until execution.
    for name in [
        "MAP_CREATE",
        "MAP_EXIST",
        "MAP_RELEASE",
        "MAP_CLEAR",
        "MAP_SIZE",
    ] {
        catalog.add(name, IntType, &[String], 1, false);
    }
    for name in ["MAP_HAS", "MAP_REMOVE", "MAP_FROMXML"] {
        catalog.add(name, IntType, &[String, String], 2, false);
    }
    catalog.add("MAP_SET", IntType, &[String, String, String], 3, false);
    catalog.add("MAP_GET", StrType, &[String, String], 2, false);
    catalog.add("MAP_TOXML", StrType, &[String], 1, false);
    // Availability is restricted by the selected snake compatibility identity.
    catalog.add(
        "MAP_VALUES",
        StrType,
        &[String, MutableString, Integer],
        1,
        false,
    );
    catalog.add("MAP_MERGE", IntType, &[String, String], 2, false);
    catalog.add("MAP_REMOVEIF", IntType, &[String, String, String], 3, false);
    catalog.add("MAP_FINDKEY", StrType, &[String, String, String], 3, false);
    catalog.add("MAP_TOSTRING", StrType, &[String, String, String], 1, false);
    catalog.add(
        "MAP_FROMSTRING",
        IntType,
        &[String, String, String, String],
        2,
        false,
    );
    catalog.add(
        "MAP_GETKEYS",
        StrType,
        &[String, IntegerOrMutableString, Integer],
        1,
        false,
    );
}

fn register_xml(catalog: &mut FunctionCatalog<'_>) {
    for name in ["XML_EXIST", "XML_RELEASE"] {
        catalog.add(name, IntType, &[Any], 1, false);
    }
    catalog.add("XML_DOCUMENT", IntType, &[Any, String], 2, false);
    catalog.add("XML_TOSTR", StrType, &[Any], 1, false);
    for name in ["XML_GET", "XML_GET_BYNAME"] {
        catalog.add(
            name,
            IntType,
            &[Any, String, IntegerOrMutableString, Integer],
            2,
            false,
        );
    }
    catalog.add(
        "XML_SET",
        IntType,
        &[IntegerOrMutableString, String, String, Integer, Integer],
        3,
        false,
    );
    catalog.add(
        "XML_SET_BYNAME",
        IntType,
        &[String, String, String, Integer, Integer],
        3,
        false,
    );
    catalog.add(
        "XML_ADDNODE",
        IntType,
        &[IntegerOrMutableString, String, String, Integer, Integer],
        3,
        false,
    );
    catalog.add(
        "XML_ADDNODE_BYNAME",
        IntType,
        &[String, String, String, Integer, Integer],
        3,
        false,
    );
    catalog.add(
        "XML_ADDATTRIBUTE",
        IntType,
        &[
            IntegerOrMutableString,
            String,
            String,
            String,
            Integer,
            Integer,
        ],
        3,
        false,
    );
    catalog.add(
        "XML_ADDATTRIBUTE_BYNAME",
        IntType,
        &[String, String, String, String, Integer, Integer],
        3,
        false,
    );
    for name in ["XML_REMOVENODE", "XML_REMOVEATTRIBUTE"] {
        catalog.add(
            name,
            IntType,
            &[IntegerOrMutableString, String, Integer],
            2,
            false,
        );
    }
    for name in ["XML_REMOVENODE_BYNAME", "XML_REMOVEATTRIBUTE_BYNAME"] {
        catalog.add(name, IntType, &[String, String, Integer], 2, false);
    }
    catalog.add(
        "XML_REPLACE",
        IntType,
        &[IntegerOrMutableString, String, String, Integer],
        2,
        false,
    );
    catalog.add(
        "XML_REPLACE_BYNAME",
        IntType,
        &[String, String, String, Integer],
        2,
        false,
    );
}

fn register_data_tables(catalog: &mut FunctionCatalog<'_>) {
    for name in [
        "DT_CREATE",
        "DT_EXIST",
        "DT_RELEASE",
        "DT_CLEAR",
        "DT_COLUMN_LENGTH",
        "DT_ROW_LENGTH",
    ] {
        catalog.add(name, IntType, &[String], 1, false);
    }
    catalog.add("DT_NOCASE", IntType, &[String, Integer], 2, false);
    for name in ["DT_COLUMN_EXIST", "DT_COLUMN_REMOVE"] {
        catalog.add(name, IntType, &[String, String], 2, false);
    }
    catalog.add(
        "DT_COLUMN_ADD",
        IntType,
        &[String, String, Any, Integer],
        2,
        false,
    );
    catalog.add(
        "DT_COLUMN_NAMES",
        IntType,
        &[String, ReferenceAny],
        1,
        false,
    );
    catalog.add("DT_ROW_ADD", IntType, &[Any], 1, true);
    catalog.add("DT_ROW_SET", IntType, &[Any], 2, true);
    catalog.add("DT_ROW_REMOVE", IntType, &[String, Any, Integer], 2, false);
    for name in ["DT_CELL_GET", "DT_CELL_GETS", "DT_CELL_ISNULL"] {
        catalog.add(
            name,
            if name == "DT_CELL_GETS" {
                StrType
            } else {
                IntType
            },
            &[String, Integer, String, Integer],
            3,
            false,
        );
    }
    catalog.add(
        "DT_CELL_SET",
        IntType,
        &[String, Integer, String, Any, Integer],
        3,
        false,
    );
    catalog.add(
        "DT_SELECT",
        IntType,
        &[String, String, String, ReferenceAny],
        1,
        false,
    );
    catalog.add("DT_TOXML", StrType, &[String, MutableString], 1, false);
    catalog.add("DT_FROMXML", IntType, &[String, String, String], 3, false);
}
