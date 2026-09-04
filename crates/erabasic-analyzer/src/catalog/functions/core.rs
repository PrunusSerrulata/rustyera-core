use erabasic_hir::SemanticType::{Integer as IntType, String as StrType};

use super::{
    ArgumentConstraint::{
        Any, Integer, IntegerOrReference, MutableString, ReferenceAny, ReferenceOrString, String,
    },
    FunctionCatalog,
};

pub(super) fn register(catalog: &mut FunctionCatalog<'_>) {
    register_numeric_and_text(catalog);
    register_dynamic_and_reflection(catalog);
    register_project_data(catalog);
    register_array_queries(catalog);
}

fn register_numeric_and_text(catalog: &mut FunctionCatalog<'_>) {
    for name in ["ABS", "SIGN", "SQRT", "CBRT", "LOG", "LOG10", "EXPONENT"] {
        // Fixed arity is checked even in a discarded user-call tail. Keep the
        // existing operand constraint; snake's lazy user arity never applies here.
        catalog.add(name, IntType, &[Any], 1, false);
    }
    for name in ["GETBIT", "BITCOUNT", "CHARANUM", "STRLEN", "STRLENU"] {
        catalog.add(name, IntType, &[Any], 1, true);
    }
    catalog.add("HTML_STRINGLEN", IntType, &[String, Integer], 1, false);
    catalog.add("HTML_STRINGLINES", IntType, &[String, Integer], 2, false);
    catalog.add("HTML_SUBSTRING", StrType, &[String, Integer], 2, false);
    catalog.add("RAND", IntType, &[Integer, Integer], 1, false);
    for name in ["MAX", "MIN", "LIMIT", "POWER", "INRANGE"] {
        catalog.add(name, IntType, &[Integer], 1, true);
    }
    catalog.add("GETMILLISECOND", IntType, &[], 0, false);
    catalog.add("GETTIME", IntType, &[], 0, false);
    catalog.add("CLEARMEMORY", IntType, &[], 0, false);
    catalog.add("GETSOUNDORBGMINFO", IntType, &[Integer, Integer], 1, false);
    catalog.add("ISPLAYINGSOUND", IntType, &[Integer], 1, false);
    catalog.add(
        "SOUNDCONTROL",
        IntType,
        &[Integer, Integer, Integer, Integer],
        2,
        false,
    );
    catalog.add("ISPLAYINGBGM", IntType, &[], 0, false);
    catalog.add(
        "BGMCONTROL",
        IntType,
        &[Integer, Integer, Integer],
        1,
        false,
    );
    // FunctionIdentifier exposes these as formatted METHOD statements. Their
    // integer result follows the same RESULT convention as other methods.
    for name in ["STRLENFORM", "STRLENFORMU"] {
        catalog.add(name, IntType, &[String], 1, false);
    }
    for name in ["SUBSTRING", "SUBSTRINGU"] {
        catalog.add(name, StrType, &[String, Integer, Integer], 1, false);
    }
    for name in ["STRFORM", "UNICODETOSTR", "TOLOWER", "TOUPPER"] {
        catalog.add(name, StrType, &[Any], 1, true);
    }
    // The third REPLACE operand is normally a string value, but mode 1 accepts
    // a one-dimensional string array and consumes one replacement per match.
    catalog.add(
        "REPLACE",
        StrType,
        &[String, String, ReferenceOrString, Integer],
        3,
        false,
    );
    // Emuera's UNICODE converts one UTF-16 code unit value to a string.  It is
    // the inverse-shaped operation of ENCODETOUNI; keeping the signature here
    // exact prevents lowering it as the old string-to-integer approximation.
    catalog.add("UNICODE", StrType, &[Integer], 1, false);
    catalog.add("TOINT", IntType, &[String], 1, false);
    catalog.add("ISNUMERIC", IntType, &[String], 1, false);
    for name in ["UNCHECKED_ADD", "UNCHECKED_SUB", "UNCHECKED_MUL"] {
        catalog.add(name, IntType, &[Integer, Integer], 2, false);
    }
    catalog.add("UNCHECKED_NEG", IntType, &[Integer], 1, false);
    catalog.add("VARSIZE", IntType, &[String, Integer], 1, false);
    catalog.add("EXISTFUNCTION", IntType, &[String, Integer], 1, false);
    catalog.add("EXISTMETH", IntType, &[String], 1, false);
}

fn register_dynamic_and_reflection(catalog: &mut FunctionCatalog<'_>) {
    // Only the target name is required. The second slot is a typed fallback;
    // remaining slots retain their value/place shape for runtime resolution.
    catalog.add("GETMETH", IntType, &[String, Integer, Any], 1, true);
    catalog.add("GETMETHS", StrType, &[String, String, Any], 1, true);
    catalog.add("EXISTVAR", IntType, &[String], 1, false);
    catalog.add(
        "MATCHALL",
        IntType,
        &[ReferenceAny, Any, Any, Any, ReferenceAny],
        2,
        false,
    );
    catalog.add(
        "MATCHALLEX",
        IntType,
        &[String, Any, Any, Any, ReferenceAny],
        2,
        false,
    );
    catalog.add("STRFORMCHECK", IntType, &[String], 1, false);
    catalog.add("GETVAR", IntType, &[String], 1, false);
    catalog.add("GETVARS", StrType, &[String], 1, false);
    catalog.add("GETDOINGFUNCTION", StrType, &[], 0, false);
    for name in [
        "ENUMFUNCBEGINSWITH",
        "ENUMFUNCENDSWITH",
        "ENUMFUNCWITH",
        "ENUMVARBEGINSWITH",
        "ENUMVARENDSWITH",
        "ENUMVARWITH",
    ] {
        catalog.add(name, IntType, &[String, MutableString], 1, false);
    }
    catalog.add("CONVERT", StrType, &[Integer, Integer], 2, false);
    catalog.add(
        "COLOR_FROMRGB",
        IntType,
        &[Integer, Integer, Integer],
        3,
        false,
    );
    catalog.add("COLOR_FROMNAME", IntType, &[String], 1, false);
    catalog.add("TOSTR", StrType, &[Integer, String], 1, false);
    for name in ["TOFULL", "TOHALF"] {
        catalog.add(name, StrType, &[String], 1, false);
    }
    catalog.add("MONEYSTR", StrType, &[Integer, String], 1, false);
    catalog.add("STRFIND", IntType, &[String, String, Integer], 2, true);
    catalog.add("STRFINDU", IntType, &[String, String, Integer], 2, true);
    for name in ["STRLENS", "STRLENSU", "UNICODEBYTE"] {
        catalog.add(name, IntType, &[String], 1, false);
    }
    catalog.add("ENCODETOUNI", IntType, &[String, Integer], 1, false);
    catalog.add("SETVAR", IntType, &[String, Any], 2, false);
    catalog.add(
        "VARSETEX",
        IntType,
        &[String, Any, Integer, Integer, Integer],
        2,
        false,
    );
    catalog.add("CHARATU", StrType, &[String, Integer], 2, false);
    catalog.add(
        "STRJOIN",
        StrType,
        &[ReferenceAny, String, Integer, Integer],
        1,
        false,
    );
    catalog.add("BARSTR", StrType, &[Integer, Integer, Integer], 3, false);
    catalog.add("GETCONFIG", IntType, &[String], 1, false);
    catalog.add("GETCONFIGS", StrType, &[String], 1, false);
}

fn register_project_data(catalog: &mut FunctionCatalog<'_>) {
    catalog.add(
        "GETNUM",
        IntType,
        &[ReferenceAny, String, Integer],
        2,
        false,
    );
    catalog.add(
        "ERDNAME",
        StrType,
        &[ReferenceAny, Integer, Integer],
        2,
        false,
    );
    for name in ["GETCHARA", "EXISTCSV"] {
        catalog.add(name, IntType, &[Integer, Integer], 1, false);
    }
    catalog.add("GETSPCHARA", IntType, &[Integer], 1, false);
    for name in [
        "GETCSVNOBYNAME",
        "GETCSVNOBYCALLNAME",
        "GETCSVNOBYNICKNAME",
        "GETCSVNOBYMASTERNAME",
    ] {
        catalog.add(name, IntType, &[String], 1, false);
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
        catalog.add(name, IntType, &[Integer, Integer, Integer], 2, false);
    }
    for name in ["CSVNAME", "CSVCALLNAME", "CSVNICKNAME", "CSVMASTERNAME"] {
        catalog.add(name, StrType, &[Integer, Integer], 1, false);
    }
    catalog.add("CSVCSTR", StrType, &[Integer, Integer, Integer], 2, false);
    for name in ["FINDCHARA", "FINDLASTCHARA"] {
        catalog.add(
            name,
            IntType,
            &[ReferenceAny, Any, Integer, Integer],
            2,
            false,
        );
    }
    for name in ["FINDELEMENT", "FINDLASTELEMENT"] {
        catalog.add(
            name,
            IntType,
            &[ReferenceAny, Any, Integer, Integer, Integer],
            2,
            false,
        );
    }
    catalog.add(
        "REGEXPMATCH",
        IntType,
        &[String, String, IntegerOrReference, MutableString],
        2,
        false,
    );
}

fn register_array_queries(catalog: &mut FunctionCatalog<'_>) {
    for name in [
        "SUMARRAY",
        "SUMCARRAY",
        "MAXARRAY",
        "MAXCARRAY",
        "MINARRAY",
        "MINCARRAY",
    ] {
        catalog.add(name, IntType, &[ReferenceAny, Integer, Integer], 1, false);
    }
    for name in ["MATCH", "CMATCH"] {
        catalog.add(
            name,
            IntType,
            &[ReferenceAny, Any, Integer, Integer],
            2,
            false,
        );
    }
    for name in ["INRANGEARRAY", "INRANGECARRAY"] {
        catalog.add(
            name,
            IntType,
            &[ReferenceAny, Integer, Integer, Integer, Integer],
            3,
            false,
        );
    }
    for name in ["GROUPMATCH", "NOSAMES", "ALLSAMES"] {
        catalog.add(name, IntType, &[Any], 2, true);
    }
    catalog.add("ARRAYMSORT", IntType, &[ReferenceAny], 1, true);
    catalog.add(
        "ARRAYMSORTEX",
        IntType,
        &[ReferenceOrString, ReferenceAny, Integer, Integer],
        2,
        false,
    );
}
