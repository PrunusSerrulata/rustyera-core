use std::collections::BTreeMap;

use erabasic_hir::SemanticType;

use super::{ArgumentConstraint, CallableSignature};

mod fallbacks;

use fallbacks::{INTEGER_FALLBACKS, STRING_FALLBACKS};

#[allow(clippy::items_after_statements, clippy::too_many_lines)]
pub(super) fn builtin_functions() -> BTreeMap<String, CallableSignature> {
    use ArgumentConstraint::{
        Any, Integer, IntegerOrMutableString, IntegerOrReference, MutableInteger, MutableString,
        ReferenceAny, ReferenceOrString, String,
    };
    use SemanticType::{Error as ErrorType, Integer as IntType, String as StrType};

    let mut result = BTreeMap::new();
    let mut add = |name: &str, return_type, arguments: &[ArgumentConstraint], minimum, variadic| {
        result.insert(
            name.to_owned(),
            CallableSignature {
                name: name.to_owned(),
                return_type,
                arguments: arguments.to_vec(),
                minimum_arguments: minimum,
                variadic,
                allow_omitted: false,
            },
        );
    };
    for name in ["ABS", "SIGN", "SQRT", "CBRT", "LOG", "LOG10", "EXPONENT"] {
        // Fixed arity is checked even in a discarded user-call tail. Keep the
        // existing operand constraint; snake's lazy user arity never applies here.
        add(name, IntType, &[Any], 1, false);
    }
    for name in [
        "GETBIT",
        "BITCOUNT",
        "CHARANUM",
        "GETTIME",
        "GETMILLISECOND",
        "VARSIZE",
        "EXISTCSV",
        "FINDELEMENT",
        "FINDLASTELEMENT",
        "STRLEN",
        "STRLENU",
    ] {
        add(name, IntType, &[Any], 1, true);
    }
    add("HTML_STRINGLEN", IntType, &[String, Integer], 1, false);
    add("HTML_STRINGLINES", IntType, &[String, Integer], 2, false);
    add("HTML_SUBSTRING", StrType, &[String, Integer], 2, false);
    add("RAND", IntType, &[Integer, Integer], 1, false);
    for name in ["MAX", "MIN", "LIMIT", "POWER", "INRANGE"] {
        add(name, IntType, &[Integer], 1, true);
    }
    add("GETMILLISECOND", IntType, &[], 0, false);
    add("GETTIME", IntType, &[], 0, false);
    // FunctionIdentifier exposes these as formatted METHOD statements. Their
    // integer result follows the same RESULT convention as other methods.
    for name in ["STRLENFORM", "STRLENFORMU"] {
        add(name, IntType, &[String], 1, false);
    }
    for name in ["SUBSTRING", "SUBSTRINGU"] {
        add(name, StrType, &[String, Integer, Integer], 1, false);
    }
    for name in ["STRFORM", "UNICODETOSTR", "TOLOWER", "TOUPPER"] {
        add(name, StrType, &[Any], 1, true);
    }
    // The third REPLACE operand is normally a string value, but mode 1 accepts
    // a one-dimensional string array and consumes one replacement per match.
    add(
        "REPLACE",
        StrType,
        &[String, String, ReferenceOrString, Integer],
        3,
        false,
    );
    // Emuera's UNICODE converts one UTF-16 code unit value to a string.  It is
    // the inverse-shaped operation of ENCODETOUNI; keeping the signature here
    // exact prevents lowering it as the old string-to-integer approximation.
    add("UNICODE", StrType, &[Integer], 1, false);
    add("TOINT", IntType, &[String], 1, false);
    add("ISNUMERIC", IntType, &[String], 1, false);
    for name in ["UNCHECKED_ADD", "UNCHECKED_SUB", "UNCHECKED_MUL"] {
        add(name, IntType, &[Integer, Integer], 2, false);
    }
    add("UNCHECKED_NEG", IntType, &[Integer], 1, false);
    add("VARSIZE", IntType, &[String, Integer], 1, false);
    add("EXISTFUNCTION", IntType, &[String, Integer], 1, false);
    add("EXISTMETH", IntType, &[String], 1, false);
    // Only the target name is required. The second slot is a typed fallback;
    // remaining slots retain their value/place shape for runtime resolution.
    add("GETMETH", IntType, &[String, Integer, Any], 1, true);
    add("GETMETHS", StrType, &[String, String, Any], 1, true);
    add("EXISTVAR", IntType, &[String], 1, false);
    add(
        "MATCHALL",
        IntType,
        &[ReferenceAny, Any, Any, Any, ReferenceAny],
        2,
        false,
    );
    add(
        "MATCHALLEX",
        IntType,
        &[String, Any, Any, Any, ReferenceAny],
        2,
        false,
    );
    add("STRFORMCHECK", IntType, &[String], 1, false);
    add("GETVAR", IntType, &[String], 1, false);
    add("GETVARS", StrType, &[String], 1, false);
    add("GETDOINGFUNCTION", StrType, &[], 0, false);
    for name in [
        "ENUMFUNCBEGINSWITH",
        "ENUMFUNCENDSWITH",
        "ENUMFUNCWITH",
        "ENUMVARBEGINSWITH",
        "ENUMVARENDSWITH",
        "ENUMVARWITH",
    ] {
        add(name, IntType, &[String, MutableString], 1, false);
    }
    add("CONVERT", StrType, &[Integer, Integer], 2, false);
    add(
        "COLOR_FROMRGB",
        IntType,
        &[Integer, Integer, Integer],
        3,
        false,
    );
    add("COLOR_FROMNAME", IntType, &[String], 1, false);
    add("TOSTR", StrType, &[Integer, String], 1, false);
    for name in ["TOFULL", "TOHALF"] {
        add(name, StrType, &[String], 1, false);
    }
    add("MONEYSTR", StrType, &[Integer, String], 1, false);
    add("STRFIND", IntType, &[String, String, Integer], 2, true);
    add("STRFINDU", IntType, &[String, String, Integer], 2, true);
    for name in ["STRLENS", "STRLENSU", "UNICODEBYTE"] {
        add(name, IntType, &[String], 1, false);
    }
    add("ENCODETOUNI", IntType, &[String, Integer], 1, false);
    add("SETVAR", IntType, &[String, Any], 2, false);
    add("CHARATU", StrType, &[String, Integer], 2, false);
    add(
        "STRJOIN",
        StrType,
        &[ReferenceAny, String, Integer, Integer],
        1,
        false,
    );
    add("BARSTR", StrType, &[Integer, Integer, Integer], 3, false);
    add("GETCONFIG", IntType, &[String], 1, false);
    add("GETCONFIGS", StrType, &[String], 1, false);
    add(
        "GETNUM",
        IntType,
        &[ReferenceAny, String, Integer],
        2,
        false,
    );
    add(
        "ERDNAME",
        StrType,
        &[ReferenceAny, Integer, Integer],
        2,
        false,
    );
    for name in ["GETCHARA", "EXISTCSV"] {
        add(name, IntType, &[Integer, Integer], 1, false);
    }
    add("GETSPCHARA", IntType, &[Integer], 1, false);
    for name in [
        "GETCSVNOBYNAME",
        "GETCSVNOBYCALLNAME",
        "GETCSVNOBYNICKNAME",
        "GETCSVNOBYMASTERNAME",
    ] {
        add(name, IntType, &[String], 1, false);
    }
    for name in [
        "CSVBASE",
        "CSVABL",
        "CSVMARK",
        "CSVEXP",
        "CSVRELATION",
        "CSVTALENT",
        "CSVCFLAG",
        "CSVEQUIP",
        "CSVJUEL",
    ] {
        add(name, IntType, &[Integer, Integer, Integer], 2, false);
    }
    for name in ["CSVNAME", "CSVCALLNAME", "CSVNICKNAME", "CSVMASTERNAME"] {
        add(name, StrType, &[Integer, Integer], 1, false);
    }
    add("CSVCSTR", StrType, &[Integer, Integer, Integer], 2, false);
    for name in ["FINDCHARA", "FINDLASTCHARA"] {
        add(
            name,
            IntType,
            &[ReferenceAny, Any, Integer, Integer],
            2,
            false,
        );
    }
    for name in ["FINDELEMENT", "FINDLASTELEMENT"] {
        add(
            name,
            IntType,
            &[ReferenceAny, Any, Integer, Integer, Integer],
            2,
            false,
        );
    }
    add(
        "REGEXPMATCH",
        IntType,
        &[String, String, IntegerOrReference, MutableString],
        2,
        false,
    );
    for name in [
        "SUMARRAY",
        "SUMCARRAY",
        "MAXARRAY",
        "MAXCARRAY",
        "MINARRAY",
        "MINCARRAY",
    ] {
        add(name, IntType, &[ReferenceAny, Integer, Integer], 1, false);
    }
    for name in ["MATCH", "CMATCH"] {
        add(
            name,
            IntType,
            &[ReferenceAny, Any, Integer, Integer],
            2,
            false,
        );
    }
    for name in ["INRANGEARRAY", "INRANGECARRAY"] {
        add(
            name,
            IntType,
            &[ReferenceAny, Integer, Integer, Integer, Integer],
            3,
            false,
        );
    }
    for name in ["GROUPMATCH", "NOSAMES", "ALLSAMES"] {
        add(name, IntType, &[Any], 2, true);
    }
    add("ARRAYMSORT", IntType, &[ReferenceAny], 1, true);
    add(
        "ARRAYMSORTEX",
        IntType,
        &[ReferenceOrString, ReferenceAny, Integer, Integer],
        2,
        false,
    );
    // GETKEY is deliberately non-constant in Emuera and accepts exactly one
    // integer virtual-key code. HIR calls are never folded, so the signature is
    // sufficient to retain that behavior in the current analyzer.
    add("GETKEY", IntType, &[Integer], 1, false);
    add("GETKEYTRIGGERED", IntType, &[Integer], 1, false);
    add("SEQUENCEINPUT", IntType, &[String], 1, false);
    add("DISABLE_INPUT_MACRO", IntType, &[], 0, false);
    add("ENABLE_INPUT_MACRO", IntType, &[], 0, false);
    add("ENV_HAS_CAPABILITY", IntType, &[String, Integer], 1, false);
    add("GETPLATFORM", IntType, &[], 0, false);
    add("GETTEXTBOX", StrType, &[], 0, false);
    add("SETTEXTBOX", IntType, &[String], 1, false);
    add("HOTKEY_STATE_INIT", IntType, &[Integer], 1, false);
    add("HOTKEY_STATE", IntType, &[Integer, Integer], 2, false);
    add(
        "FLOWINPUT",
        IntType,
        &[Integer, Integer, Integer, Integer],
        1,
        false,
    );
    add("FLOWINPUTS", IntType, &[Integer, String], 1, false);
    add("MOUSEX", IntType, &[], 0, false);
    add("MOUSEY", IntType, &[], 0, false);
    add("MOUSEB", StrType, &[], 0, false);

    // Safe SQL is a snake-profile Host service. Parameterized calls keep the third String
    // constraint as the repeated variadic tail; omitted tail slots become SQL NULL at runtime.
    add("SQL_CONNECT", IntType, &[String, String], 1, false);
    for name in ["SQL_DISCONNECT", "SQL_READER_READ", "SQL_READER_CLOSE"] {
        add(
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
        add(name, IntType, &[String, String], 2, false);
    }
    add(
        "SQL_EXECUTE_SCALAR_STRING",
        StrType,
        &[String, String],
        2,
        false,
    );
    for name in ["SQL_READER_GET_LONG", "SQL_READER_ISNULL"] {
        add(name, IntType, &[Integer, Integer], 2, false);
    }
    add(
        "SQL_READER_GET_STRING",
        StrType,
        &[Integer, Integer],
        2,
        false,
    );
    add(
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
        add(name, result_type, &[String, String, String], 2, true);
    }

    // Deferred SQL names remain known to the catalog so the compiler can return one stable
    // missing-capability diagnostic rather than misclassifying them as unknown functions.
    add("SQL_CONNECTION_OPEN", IntType, &[String], 1, false);
    add("SQL_ESCAPE", StrType, &[String], 1, false);
    add(
        "SQL_READER_GET_FLOAT",
        ErrorType,
        &[Integer, Integer],
        2,
        false,
    );
    add(
        "SQL_EXECUTE_SCALAR_FLOAT",
        ErrorType,
        &[String, String],
        2,
        false,
    );
    add(
        "SQL_P_EXECUTE_SCALAR_FLOAT",
        ErrorType,
        &[String, String, String],
        2,
        true,
    );
    add(
        "SQL_IMPORT_DT_XML",
        IntType,
        &[String, String, String, String],
        4,
        false,
    );
    add(
        "SQL_EXPORT_MAP_XML",
        IntType,
        &[String, String, String],
        3,
        false,
    );
    add(
        "SQL_EXPORT_DT_XML",
        IntType,
        &[String, String, String, String],
        4,
        false,
    );
    add(
        "SQL_IMPORT_XML_CUSTOM",
        IntType,
        &[String, String, String, String, String],
        5,
        false,
    );
    add("CURRENTALIGN", StrType, &[], 0, false);
    add("CURRENTREDRAW", IntType, &[], 0, false);
    add("GETFONT", StrType, &[], 0, false);
    add("GETSTYLE", IntType, &[], 0, false);
    add("GGETCOLOR", IntType, &[Integer, Integer, Integer], 3, false);
    add(
        "GGETTEXTSIZE",
        IntType,
        &[String, String, Integer, Integer],
        3,
        false,
    );
    for name in [
        "GETBGCOLOR",
        "GETCOLOR",
        "GETDEFBGCOLOR",
        "GETDEFCOLOR",
        "GETFOCUSCOLOR",
    ] {
        add(name, IntType, &[], 0, false);
    }
    for name in ["HTML_ESCAPE", "HTML_TOPLAINTEXT"] {
        add(name, StrType, &[String], 1, false);
    }
    for name in [
        "ENUMFUNCBEGINSWITH",
        "ENUMFUNCENDSWITH",
        "ENUMFUNCWITH",
        "ENUMVARBEGINSWITH",
        "ENUMVARENDSWITH",
        "ENUMVARWITH",
    ] {
        add(name, IntType, &[String, MutableString], 1, false);
    }

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
        add(name, IntType, &[String], 1, false);
    }
    for name in ["MAP_HAS", "MAP_REMOVE", "MAP_FROMXML"] {
        add(name, IntType, &[String, String], 2, false);
    }
    add("MAP_SET", IntType, &[String, String, String], 3, false);
    add("MAP_GET", StrType, &[String, String], 2, false);
    add("MAP_TOXML", StrType, &[String], 1, false);
    // Availability is restricted by the selected snake compatibility identity.
    add(
        "MAP_VALUES",
        StrType,
        &[String, MutableString, Integer],
        1,
        false,
    );
    add("MAP_MERGE", IntType, &[String, String], 2, false);
    add("MAP_REMOVEIF", IntType, &[String, String, String], 3, false);
    add("MAP_FINDKEY", StrType, &[String, String, String], 3, false);
    add("MAP_TOSTRING", StrType, &[String, String, String], 1, false);
    add(
        "MAP_FROMSTRING",
        IntType,
        &[String, String, String, String],
        2,
        false,
    );
    add(
        "MAP_GETKEYS",
        StrType,
        &[String, IntegerOrMutableString, Integer],
        1,
        false,
    );

    for name in ["XML_EXIST", "XML_RELEASE"] {
        add(name, IntType, &[Any], 1, false);
    }
    add("XML_DOCUMENT", IntType, &[Any, String], 2, false);
    add("XML_TOSTR", StrType, &[Any], 1, false);
    for name in ["XML_GET", "XML_GET_BYNAME"] {
        add(
            name,
            IntType,
            &[Any, String, IntegerOrMutableString, Integer],
            2,
            false,
        );
    }
    add(
        "XML_SET",
        IntType,
        &[IntegerOrMutableString, String, String, Integer, Integer],
        3,
        false,
    );
    add(
        "XML_SET_BYNAME",
        IntType,
        &[String, String, String, Integer, Integer],
        3,
        false,
    );
    add(
        "XML_ADDNODE",
        IntType,
        &[IntegerOrMutableString, String, String, Integer, Integer],
        3,
        false,
    );
    add(
        "XML_ADDNODE_BYNAME",
        IntType,
        &[String, String, String, Integer, Integer],
        3,
        false,
    );
    add(
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
    add(
        "XML_ADDATTRIBUTE_BYNAME",
        IntType,
        &[String, String, String, String, Integer, Integer],
        3,
        false,
    );
    for name in ["XML_REMOVENODE", "XML_REMOVEATTRIBUTE"] {
        add(
            name,
            IntType,
            &[IntegerOrMutableString, String, Integer],
            2,
            false,
        );
    }
    for name in ["XML_REMOVENODE_BYNAME", "XML_REMOVEATTRIBUTE_BYNAME"] {
        add(name, IntType, &[String, String, Integer], 2, false);
    }
    add(
        "XML_REPLACE",
        IntType,
        &[IntegerOrMutableString, String, String, Integer],
        2,
        false,
    );
    add(
        "XML_REPLACE_BYNAME",
        IntType,
        &[String, String, String, Integer],
        2,
        false,
    );

    for name in [
        "DT_CREATE",
        "DT_EXIST",
        "DT_RELEASE",
        "DT_CLEAR",
        "DT_COLUMN_LENGTH",
        "DT_ROW_LENGTH",
    ] {
        add(name, IntType, &[String], 1, false);
    }
    add("DT_NOCASE", IntType, &[String, Integer], 2, false);
    for name in ["DT_COLUMN_EXIST", "DT_COLUMN_REMOVE"] {
        add(name, IntType, &[String, String], 2, false);
    }
    add(
        "DT_COLUMN_ADD",
        IntType,
        &[String, String, Any, Integer],
        2,
        false,
    );
    add(
        "DT_COLUMN_NAMES",
        IntType,
        &[String, ReferenceAny],
        1,
        false,
    );
    add("DT_ROW_ADD", IntType, &[Any], 1, true);
    add("DT_ROW_SET", IntType, &[Any], 2, true);
    add("DT_ROW_REMOVE", IntType, &[String, Any, Integer], 2, false);
    for name in ["DT_CELL_GET", "DT_CELL_GETS", "DT_CELL_ISNULL"] {
        add(
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
    add(
        "DT_CELL_SET",
        IntType,
        &[String, Integer, String, Any, Integer],
        3,
        false,
    );
    add(
        "DT_SELECT",
        IntType,
        &[String, String, String, ReferenceAny],
        1,
        false,
    );
    add("DT_TOXML", StrType, &[String, MutableString], 1, false);
    add("DT_FROMXML", IntType, &[String, String, String], 3, false);
    add("GCREATE", IntType, &[Integer, Integer, Integer], 3, false);
    add(
        "GCREATEFROMFILE",
        IntType,
        &[Integer, String, Integer],
        2,
        false,
    );
    for name in ["GLOAD", "GSAVE", "GSETBRUSH"] {
        add(name, IntType, &[Integer, Integer], 2, false);
    }
    for name in [
        "GCREATED",
        "GDISPOSE",
        "GWIDTH",
        "GHEIGHT",
        "GGETBRUSH",
        "GGETPEN",
        "GGETPENWIDTH",
        "GGETFONTSIZE",
        "GGETFONTSTYLE",
    ] {
        add(name, IntType, &[Integer], 1, false);
    }
    add("GGETFONT", StrType, &[Integer], 1, false);
    add(
        "GSETCOLOR",
        IntType,
        &[Integer, Integer, Integer, Integer],
        4,
        false,
    );
    add("GSETPEN", IntType, &[Integer, Integer, Integer], 3, false);
    add(
        "GDASHSTYLE",
        IntType,
        &[Integer, Integer, Integer],
        3,
        false,
    );
    add(
        "GSETFONT",
        IntType,
        &[Integer, String, Integer, Integer],
        3,
        false,
    );
    add("GFILLRECTANGLE", IntType, &[Integer; 5], 5, false);
    add("GDRAWLINE", IntType, &[Integer; 5], 5, false);
    add(
        "GDRAWTEXT",
        IntType,
        &[Integer, String, Integer, Integer],
        2,
        false,
    );
    add("GGETCOLOR", IntType, &[Integer, Integer, Integer], 3, false);
    add("GDRAWGWITHMASK", IntType, &[Integer; 5], 5, false);
    add("GDRAWGWITHROTATE", IntType, &[Integer; 5], 3, false);
    add(
        "GDRAWG",
        IntType,
        &[
            Integer,
            Integer,
            Integer,
            Integer,
            Integer,
            Integer,
            Integer,
            Integer,
            Integer,
            Integer,
            ReferenceAny,
        ],
        10,
        false,
    );
    add(
        "GDRAWSPRITE",
        IntType,
        &[
            Integer,
            String,
            Integer,
            Integer,
            Integer,
            Integer,
            ReferenceAny,
        ],
        2,
        false,
    );
    add(
        "SPRITECREATE",
        IntType,
        &[
            String, Integer, Integer, Integer, Integer, Integer, Integer, Integer, Integer, Integer,
        ],
        2,
        false,
    );
    add(
        "SPRITECREATEFROMFILE",
        IntType,
        &[String, String, Integer],
        2,
        false,
    );
    for name in ["G_POLYGON_DRAW", "G_POLYGON_FILL", "G_POLYGON_POINT_CLEAR"] {
        add(name, IntType, &[Integer], 1, false);
    }
    add(
        "G_POLYGON_POINT_ADD",
        IntType,
        &[Integer, Integer, Integer],
        3,
        false,
    );
    add("MOVETEXTBOX", IntType, &[Integer; 3], 3, false);
    add("RESUMETEXTBOX", IntType, &[Integer; 3], 3, false);
    add("BITMAP_CACHE_ENABLE", IntType, &[Integer], 1, false);
    add("SETANIMETIMER", IntType, &[Integer], 1, false);
    add("GETANIMETIMER", IntType, &[], 0, false);
    for name in ["CBGCLEAR", "CBGCLEARBUTTON", "CBGREMOVEBMAP"] {
        add(name, IntType, &[], 0, false);
    }
    add("CBGREMOVERANGE", IntType, &[Integer, Integer], 2, false);
    add(
        "CBGSETG",
        IntType,
        &[Integer, Integer, Integer, Integer],
        4,
        false,
    );
    add("CBGSETBMAPG", IntType, &[Integer], 1, false);
    add(
        "CBGSETSPRITE",
        IntType,
        &[
            String,
            Integer,
            Integer,
            Integer,
            Integer,
            Integer,
            Integer,
            ReferenceAny,
        ],
        1,
        false,
    );
    add(
        "CBGSETBUTTONSPRITE",
        IntType,
        &[Integer, String, String, Integer, Integer, Integer, String],
        6,
        false,
    );
    add("EXISTSIMAGELAYER", IntType, &[Integer], 1, false);
    add("GETLINEY", IntType, &[Integer], 1, false);
    add(
        "BITSET",
        IntType,
        &[MutableInteger, Integer, Integer, Integer],
        2,
        false,
    );
    for name in ["BITGET", "BITTOGGLE", "BITINDEXOFFIRST"] {
        add(name, IntType, &[MutableInteger, Integer], 1, false);
    }
    result
        .get_mut("SPRITECREATEFROMFILE")
        .expect("file sprite signature was inserted")
        .allow_omitted = true;
    result
        .get_mut("CBGSETSPRITE")
        .expect("CBG signature was inserted")
        .allow_omitted = true;
    for name in ["BITSET", "BITGET", "BITTOGGLE", "BITINDEXOFFIRST"] {
        result
            .get_mut(name)
            .expect("BIT signature inserted")
            .allow_omitted = true;
    }
    result
        .get_mut("STRJOIN")
        .expect("STRJOIN signature was inserted")
        .allow_omitted = true;
    result
        .get_mut("RAND")
        .expect("RAND signature was inserted")
        .allow_omitted = true;
    result
        .get_mut("SQL_CONNECT")
        .expect("SQL_CONNECT signature was inserted")
        .allow_omitted = true;
    for name in [
        "SQL_P_EXECUTE_NONQUERY",
        "SQL_P_EXECUTE_READER",
        "SQL_P_EXECUTE_SCALAR_LONG",
        "SQL_P_EXECUTE_SCALAR_STRING",
        "SQL_P_EXECUTE_SCALAR_FLOAT",
    ] {
        result
            .get_mut(name)
            .expect("parameterized SQL signature was inserted")
            .allow_omitted = true;
    }
    for name in [
        "GETMETH",
        "GETMETHS",
        "FINDELEMENT",
        "FINDLASTELEMENT",
        "MATCHALL",
        "MATCHALLEX",
    ] {
        result
            .get_mut(name)
            .expect("find-element signature was inserted")
            .allow_omitted = true;
    }
    for name in ["SUBSTRING", "SUBSTRINGU", "ENCODETOUNI"] {
        result
            .get_mut(name)
            .expect("optional string method signature was inserted")
            .allow_omitted = true;
    }
    for name in ["DT_SELECT", "DT_ROW_ADD", "DT_ROW_SET", "DT_CELL_SET"] {
        result
            .get_mut(name)
            .expect("structured signature was inserted")
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
