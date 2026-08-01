use std::collections::BTreeMap;

use erabasic_hir::SemanticType;

use super::{ArgumentConstraint, CallableSignature};

#[allow(clippy::items_after_statements, clippy::too_many_lines)]
pub(super) fn builtin_functions() -> BTreeMap<String, CallableSignature> {
    use ArgumentConstraint::{
        Any, Integer, IntegerOrMutableString, IntegerOrReference, MutableString, ReferenceAny,
        ReferenceOrString, String,
    };
    use SemanticType::{Integer as IntType, String as StrType};

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
    for name in [
        "ABS",
        "SIGN",
        "SQRT",
        "CBRT",
        "LOG",
        "LOG10",
        "EXPONENT",
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
    for name in ["STRFORM", "REPLACE", "UNICODETOSTR", "TOLOWER", "TOUPPER"] {
        add(name, StrType, &[Any], 1, true);
    }
    // Emuera's UNICODE converts one UTF-16 code unit value to a string.  It is
    // the inverse-shaped operation of ENCODETOUNI; keeping the signature here
    // exact prevents lowering it as the old string-to-integer approximation.
    add("UNICODE", StrType, &[Integer], 1, false);
    add("TOINT", IntType, &[String], 1, false);
    add("ISNUMERIC", IntType, &[String], 1, false);
    add("VARSIZE", IntType, &[String, Integer], 1, false);
    add("EXISTFUNCTION", IntType, &[String, Integer], 1, false);
    add("EXISTVAR", IntType, &[String], 1, false);
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
    add("MOVETEXTBOX", IntType, &[Integer; 3], 3, false);
    add("RESUMETEXTBOX", IntType, &[Integer; 3], 3, false);
    add("BITMAP_CACHE_ENABLE", IntType, &[Integer], 1, false);
    result
        .get_mut("STRJOIN")
        .expect("STRJOIN signature was inserted")
        .allow_omitted = true;
    result
        .get_mut("RAND")
        .expect("RAND signature was inserted")
        .allow_omitted = true;
    for name in ["FINDELEMENT", "FINDLASTELEMENT"] {
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

    const INTEGER_FALLBACKS: &[&str] = &[
        "ALLSAMES",
        "ARRAYMSORT",
        "ARRAYMSORTEX",
        "BITMAP_CACHE_ENABLE",
        "MOVETEXTBOX",
        "RESUMETEXTBOX",
        "CBGCLEAR",
        "CBGCLEARBUTTON",
        "CBGREMOVEBMAP",
        "CBGREMOVERANGE",
        "CBGSETBMAPG",
        "CBGSETBUTTONSPRITE",
        "CBGSETG",
        "CBGSETSPRITE",
        "CHKCHARADATA",
        "CHKDATA",
        "CHKFONT",
        "CLIENTHEIGHT",
        "CLIENTWIDTH",
        "CMATCH",
        "COLOR_FROMNAME",
        "COLOR_FROMRGB",
        "CONVERT",
        "CSVABL",
        "CSVBASE",
        "CSVCFLAG",
        "CSVEQUIP",
        "CSVEXP",
        "CSVJUEL",
        "CSVMARK",
        "CSVRELATION",
        "CSVTALENT",
        "CURRENTREDRAW",
        "DT_CELL_GET",
        "DT_CELL_ISNULL",
        "DT_CELL_SET",
        "DT_CLEAR",
        "DT_COLUMN_ADD",
        "DT_COLUMN_EXIST",
        "DT_COLUMN_LENGTH",
        "DT_COLUMN_REMOVE",
        "DT_CREATE",
        "DT_EXIST",
        "DT_FROMXML",
        "DT_NOCASE",
        "DT_RELEASE",
        "DT_ROW_ADD",
        "DT_ROW_LENGTH",
        "DT_ROW_REMOVE",
        "DT_ROW_SET",
        "ENUMFILES",
        "EXISTFILE",
        "FIND_CHARADATA",
        "EXISTFUNCTION",
        "EXISTMETH",
        "EXISTSOUND",
        "EXISTVAR",
        "FINDCHARA",
        "FINDLASTCHARA",
        "GCLEAR",
        "GCREATE",
        "GCREATED",
        "GCREATEFROMFILE",
        "GDISPOSE",
        "GDRAWG",
        "GDRAWGWITHMASK",
        "GDRAWGWITHROTATE",
        "GDRAWLINE",
        "GDRAWSPRITE",
        "GDRAWTEXT",
        "GFILLRECTANGLE",
        "GETBGCOLOR",
        "GETCHARA",
        "GETCOLOR",
        "GETCONFIG",
        "GETDEFBGCOLOR",
        "GETDEFCOLOR",
        "GETEXPLV",
        "GETFOCUSCOLOR",
        "GETKEY",
        "GETKEYTRIGGERED",
        "GETMEMORYUSAGE",
        "GETMETH",
        "GETNUMB",
        "GETPALAMLV",
        "GETSECOND",
        "GETSPCHARA",
        "GETSTYLE",
        "GHEIGHT",
        "GLOAD",
        "GROUPMATCH",
        "GSAVE",
        "GSETBRUSH",
        "GSETCOLOR",
        "GSETFONT",
        "GSETPEN",
        "GDASHSTYLE",
        "GWIDTH",
        "GGETFONTSIZE",
        "GGETFONTSTYLE",
        "GGETPENWIDTH",
        "GGETBRUSH",
        "GGETPEN",
        "HOTKEY_STATE",
        "HOTKEY_STATE_INIT",
        "HTML_STRINGLEN",
        "HTML_STRINGLINES",
        "INRANGEARRAY",
        "INRANGECARRAY",
        "ISACTIVE",
        "ISDEFINED",
        "ISSKIP",
        "LINEISEMPTY",
        "MAP_CLEAR",
        "MAP_CREATE",
        "MAP_EXIST",
        "MAP_FROMXML",
        "MAP_HAS",
        "MAP_RELEASE",
        "MAP_REMOVE",
        "MAP_SET",
        "MAP_SIZE",
        "MATCH",
        "MAXARRAY",
        "MAXCARRAY",
        "MESSKIP",
        "MINARRAY",
        "MINCARRAY",
        "MOUSEB",
        "MOUSESKIP",
        "MOUSEX",
        "MOUSEY",
        "NOSAMES",
        "OUTPUTLOG",
        "PRINTCLENGTH",
        "PRINTCPERLINE",
        "SAVENOS",
        "SAVETEXT",
        "SETANIMETIMER",
        "SETTEXTBOX",
        "SETVAR",
        "SPRITEANIMEADDFRAME",
        "SPRITEANIMECREATE",
        "SPRITECREATE",
        "SPRITECREATED",
        "SPRITEDISPOSE",
        "SPRITEDISPOSEALL",
        "SPRITEGETCOLOR",
        "SPRITEHEIGHT",
        "SPRITEMOVE",
        "SPRITEPOSX",
        "SPRITEPOSY",
        "SPRITESETPOS",
        "SPRITEWIDTH",
        "STRCOUNT",
        "STRFINDU",
        "STRLENS",
        "STRLENSU",
        "SUMARRAY",
        "SUMCARRAY",
        "UNICODEBYTE",
        "VARSETEX",
        "XML_ADDATTRIBUTE",
        "XML_ADDATTRIBUTE_BYNAME",
        "XML_ADDNODE",
        "XML_ADDNODE_BYNAME",
        "XML_DOCUMENT",
        "XML_EXIST",
        "XML_GET",
        "XML_GET_BYNAME",
        "XML_RELEASE",
        "XML_REMOVEATTRIBUTE",
        "XML_REMOVEATTRIBUTE_BYNAME",
        "XML_REMOVENODE",
        "XML_REMOVENODE_BYNAME",
        "XML_REPLACE",
        "XML_REPLACE_BYNAME",
        "XML_SET",
        "XML_SET_BYNAME",
    ];
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
    const STRING_FALLBACKS: &[&str] = &[
        "BARSTR",
        "CHARATU",
        "CSVCALLNAME",
        "CSVCSTR",
        "CSVMASTERNAME",
        "CSVNAME",
        "CSVNICKNAME",
        "CURRENTALIGN",
        "DT_CELL_GETS",
        "DT_COLUMN_NAMES",
        "DT_SELECT",
        "DT_TOXML",
        "ENCODETOUNI",
        "ENUMFUNCBEGINSWITH",
        "ENUMFUNCENDSWITH",
        "ENUMFUNCWITH",
        "ENUMMACROBEGINSWITH",
        "ENUMMACROENDSWITH",
        "ENUMMACROWITH",
        "ENUMVARBEGINSWITH",
        "ENUMVARENDSWITH",
        "ENUMVARWITH",
        "ERDNAME",
        "ESCAPE",
        "FLOWINPUTS",
        "GETDISPLAYLINE",
        "GETDOINGFUNCTION",
        "GETFONT",
        "GETLINESTR",
        "GETMETHS",
        "GETTEXTBOX",
        "GETTIMES",
        "GETVARS",
        "GGETFONT",
        "HTML_ESCAPE",
        "HTML_GETPRINTEDSTR",
        "HTML_POPPRINTINGSTR",
        "HTML_SUBSTRING",
        "HTML_TOPLAINTEXT",
        "LOADTEXT",
        "MAP_GET",
        "MAP_GETKEYS",
        "MAP_TOXML",
        "MONEYSTR",
        "STRJOIN",
        "TOFULL",
        "TOHALF",
        "TOLOWER",
        "TOUPPER",
        "XML_TOSTR",
    ];
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
