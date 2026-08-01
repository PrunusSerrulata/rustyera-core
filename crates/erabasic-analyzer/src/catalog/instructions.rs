use std::collections::BTreeMap;

use erabasic_parser::ArgumentStyle;

use super::{ArgumentConstraint, InstructionSignature, instruction};

#[allow(clippy::items_after_statements, clippy::too_many_lines)]
pub(super) fn builtin_instructions() -> BTreeMap<String, InstructionSignature> {
    use ArgumentConstraint::{
        Any, Formatted, Integer, MutableAny, MutableInteger, MutableReferenceOrString,
        MutableString, ReferenceAny, ReferenceOrString, String,
    };
    use ArgumentStyle::{
        Expressions, Formatted as FormStyle, FormattedFirst, None as NoArgs, PrintV, Raw, Times,
    };

    let mut result = BTreeMap::new();
    let mut add =
        |name: &str, style, arguments: &[ArgumentConstraint], minimum, variadic, omitted| {
            result.insert(
                name.to_owned(),
                instruction(name, style, arguments, minimum, variadic, omitted),
            );
        };
    add(
        "HTML_TAGSPLIT",
        Expressions,
        &[String, MutableString, MutableInteger],
        1,
        false,
        false,
    );

    for name in [
        "ELSE",
        "ENDIF",
        "REND",
        "NEXT",
        "WEND",
        "CASEELSE",
        "ENDSELECT",
        "CATCH",
        "ENDCATCH",
        "ENDFUNC",
        "ENDLIST",
        "BREAK",
        "CONTINUE",
        "RESTART",
        "QUIT",
        "WAIT",
        "WAITANYKEY",
        "FORCEWAIT",
        "DRAWLINE",
        "RESETCOLOR",
        "NOSKIP",
        "ENDNOSKIP",
        "ENDDATA",
    ] {
        add(name, NoArgs, &[], 0, false, false);
    }
    // Function methods may appear as METHOD statements; the official game uses
    // this no-argument clock method and observes its value through RESULT.
    add("GETMILLISECOND", NoArgs, &[], 0, false, false);
    // A bare CURRENTREDRAW is a stringlessly discarded METHOD expression.
    add("CURRENTREDRAW", NoArgs, &[], 0, false, false);
    for name in ["PRINTCPERLINE", "SAVENOS"] {
        // Statement forms write through their argument; expression functions with
        // the same spelling are registered independently below.
        add(name, Expressions, &[MutableInteger], 1, false, false);
    }
    // Unlike VARSIZE("NAME", dimension), the statement form consumes one bare
    // array variable and writes all of its dimensions to RESULT.
    add("VARSIZE", Expressions, &[ReferenceAny], 1, false, false);
    add("ASSERT", Expressions, &[Integer], 1, false, false);
    add("THROW", FormStyle, &[Formatted], 0, false, true);
    // The statement form is distinct from ENCODETOUNI(string, position): it
    // consumes one nullable FORM string and writes its code points to RESULT.
    add("ENCODETOUNI", FormStyle, &[Formatted], 0, false, true);
    add("FORCEKANA", Expressions, &[Integer], 1, false, false);
    add("UPCHECK", NoArgs, &[], 0, false, false);
    add("CUPCHECK", Expressions, &[Integer], 1, false, false);
    add(
        "CUSTOMDRAWLINE",
        Raw,
        &[ArgumentConstraint::Raw],
        1,
        false,
        false,
    );
    add("DRAWLINEFORM", FormStyle, &[Formatted], 1, false, false);
    for name in [
        "PRINT_ABL",
        "PRINT_TALENT",
        "PRINT_MARK",
        "PRINT_EXP",
        "PRINT_PALAM",
    ] {
        add(name, Expressions, &[Integer], 1, false, false);
    }
    for name in ["PRINT_ITEM", "PRINT_SHOPITEM"] {
        add(name, NoArgs, &[], 0, false, false);
    }
    for name in ["DEBUGPRINT", "DEBUGPRINTL"] {
        add(name, Raw, &[ArgumentConstraint::Raw], 0, false, true);
    }
    for name in ["DEBUGPRINTFORM", "DEBUGPRINTFORML"] {
        add(name, FormStyle, &[Formatted], 0, false, true);
    }
    for name in [
        "PRINT",
        "PRINTL",
        "PRINTW",
        "PRINTPLAIN",
        "PRINTSINGLE",
        "DATA",
    ] {
        add(name, Raw, &[ArgumentConstraint::Raw], 0, false, true);
    }
    for name in [
        "PRINTFORM",
        "PRINTFORML",
        "PRINTFORMW",
        "PRINTPLAINFORM",
        "PRINTSINGLEFORM",
        "RETURNFORM",
        "DATAFORM",
        "PUTFORM",
        "REUSELASTLINE",
    ] {
        add(name, FormStyle, &[Formatted], 0, false, true);
    }
    add("DATALIST", NoArgs, &[], 0, false, false);
    for name in ["PRINTV", "PRINTVL", "PRINTVW"] {
        add(name, PrintV, &[Any], 1, true, false);
    }
    for name in [
        "PRINTS",
        "PRINTSL",
        "PRINTSW",
        "PRINTFORMS",
        "PRINTFORMSL",
        "PRINTFORMSW",
    ] {
        add(name, Expressions, &[String], 1, false, false);
    }
    for name in ["IF", "ELSEIF", "SIF", "WHILE", "REPEAT"] {
        add(name, Expressions, &[Integer], 1, false, false);
    }
    // LOOP accepts an optional continuation condition. With no expression it
    // remains the unconditional DO/LOOP form.
    add("LOOP", Expressions, &[Integer], 0, false, false);
    add("SELECTCASE", Expressions, &[Any], 1, false, false);
    // CASE is not a comma-separated expression list: the reference also accepts
    // `IS <operator> value` and `lower TO upper`. Keep the selector source raw so
    // the compiler's structured SELECTCASE lowering can interpret it as a unit.
    add("CASE", Raw, &[Any], 1, false, false);
    add(
        "FOR",
        Expressions,
        &[MutableInteger, Integer, Integer, Integer],
        3,
        false,
        true,
    );
    // The assignment spelling `ARRAY = a, b, c` is parsed as SET with one
    // mutable destination followed by consecutive values.
    add("SET", Expressions, &[MutableAny, Any], 2, true, false);
    add(
        "TIMES",
        Times,
        &[MutableInteger, Integer, Integer],
        3,
        false,
        false,
    );
    add(
        "POWER",
        Expressions,
        &[MutableInteger, Integer],
        2,
        true,
        false,
    );
    for name in ["SWAP", "SWAPVAR"] {
        add(
            name,
            Expressions,
            &[MutableAny, MutableAny],
            2,
            false,
            false,
        );
    }
    add(
        "ARRAYREMOVE",
        Expressions,
        &[MutableAny, Integer, Integer],
        3,
        false,
        false,
    );
    add(
        "ARRAYSHIFT",
        Expressions,
        &[MutableAny, Integer, Any, Integer, Integer],
        3,
        false,
        true,
    );
    add(
        "ARRAYSORT",
        Expressions,
        &[MutableAny, String, Integer, Integer],
        1,
        false,
        true,
    );
    add(
        "ARRAYCOPY",
        Expressions,
        &[ReferenceOrString, MutableReferenceOrString],
        2,
        false,
        false,
    );
    for name in [
        "ADDCHARA",
        "ADDSPCHARA",
        "DELCHARA",
        "ADDCOPYCHARA",
        "PICKUPCHARA",
    ] {
        add(name, Expressions, &[Integer], 1, true, false);
    }
    for name in ["SWAPCHARA", "COPYCHARA"] {
        add(name, Expressions, &[Integer, Integer], 2, false, false);
    }
    for name in ["ADDDEFCHARA", "ADDVOIDCHARA", "DELALLCHARA"] {
        add(name, NoArgs, &[], 0, false, false);
    }
    add(
        "SORTCHARA",
        Expressions,
        &[ReferenceOrString, String],
        0,
        false,
        true,
    );
    add("RESET_STAIN", Expressions, &[Integer], 1, false, false);
    add(
        "VARSET",
        Expressions,
        &[MutableAny, Any, Integer, Integer],
        1,
        false,
        true,
    );
    add(
        "CVARSET",
        Expressions,
        &[MutableAny, Any, Any, Integer, Integer],
        1,
        false,
        true,
    );
    for name in [
        "CALL",
        "CALLF",
        "JUMP",
        "BEGIN",
        "TRYCALL",
        "TRYJUMP",
        "GOTO",
        "TRYGOTO",
        "GOTOFORM",
        "TRYGOTOFORM",
    ] {
        add(name, ArgumentStyle::DynamicCall, &[Any], 1, true, true);
    }
    add("CALLEVENT", Raw, &[], 1, false, false);
    for name in [
        "CALLFORM",
        "CALLFORMF",
        "JUMPFORM",
        "TRYCALLFORM",
        "TRYCALLFORMF",
        "TRYJUMPFORM",
        "TRYCCALLFORM",
        "TRYCCALL",
        "TRYCJUMP",
        "TRYCJUMPFORM",
        "TRYCGOTO",
        "TRYCGOTOFORM",
        "FUNC",
    ] {
        add(
            name,
            ArgumentStyle::DynamicCall,
            &[Formatted, Any],
            1,
            true,
            true,
        );
    }
    add("RETURNF", Expressions, &[Any], 0, true, true);
    add("AWAIT", Expressions, &[Integer], 0, true, true);
    for name in ["INPUT", "ONEINPUT", "BINPUT", "ONEBINPUT"] {
        add(
            name,
            Expressions,
            &[Integer, Integer, Integer],
            0,
            false,
            true,
        );
    }
    // RETURN stores every supplied value in RESULT and therefore accepts an
    // arbitrary-length integer list (FunctionArgType.INT_ANY in Emuera).
    add("RETURN", Expressions, &[Integer], 0, true, true);
    for name in ["INPUTS", "ONEINPUTS", "BINPUTS", "ONEBINPUTS"] {
        add(
            name,
            FormattedFirst,
            &[Formatted, Integer, Integer],
            0,
            false,
            true,
        );
    }
    // The timed input builders in the reference implementation share a strict
    // six-slot layout. Optional trailing slots may be absent, but an interior
    // omission is not accepted by ArgumentBuilder.checkArgumentType.
    for name in ["TINPUT", "TONEINPUT"] {
        add(
            name,
            Expressions,
            &[Integer, Integer, Integer, String, Integer, Integer],
            2,
            false,
            false,
        );
    }
    for name in ["TINPUTS", "TONEINPUTS"] {
        add(
            name,
            Expressions,
            &[Integer, String, Integer, String, Integer, Integer],
            2,
            false,
            false,
        );
    }
    add("TWAIT", Expressions, &[Integer, Integer], 2, false, false);
    add(
        "SPLIT",
        Expressions,
        &[String, String, MutableString, MutableInteger],
        3,
        false,
        true,
    );
    for name in ["SETBIT", "CLEARBIT", "INVERTBIT"] {
        add(
            name,
            Expressions,
            &[MutableInteger, Integer],
            2,
            true,
            false,
        );
    }

    // Known instructions without a specialized signature still remain known. Their
    // arguments are preserved and type checked as general expressions.
    for name in [
        "BAR",
        "BARL",
        "SAVEDATA",
        "LOADDATA",
        "DELDATA",
        "SAVEGLOBAL",
        "LOADGLOBAL",
        "RESETDATA",
        "RESETGLOBAL",
        "SETCOLOR",
        "SETBGCOLOR",
        "FONTBOLD",
        "FONTITALIC",
        "FONTREGULAR",
        "REDRAW",
        "DOTRAIN",
        "DO",
        "TRYC",
        "RANDOMIZE",
        "DUMPRAND",
        "INITRAND",
    ] {
        add(name, Expressions, &[Any], 0, true, true);
    }
    // Emuera's STR argument accepts the unquoted alignment keywords used by
    // existing games. Preserve that token as raw string data instead of trying
    // to resolve CENTER/LEFT/RIGHT as EraBasic identifiers.
    add(
        "ALIGNMENT",
        Raw,
        &[ArgumentConstraint::Raw],
        1,
        false,
        false,
    );
    // These two instructions use Emuera's STR argument builder, which consumes the
    // unquoted remainder as a color name instead of parsing an expression.
    for name in ["SETCOLORBYNAME", "SETBGCOLORBYNAME"] {
        add(name, Raw, &[ArgumentConstraint::Raw], 1, false, false);
    }
    // SETFONT uses STR_EXPRESSION_NULLABLE: an empty invocation resets the font.
    add("SETFONT", Expressions, &[String], 0, false, true);
    add("PRINTDATA", Expressions, &[MutableInteger], 0, false, true);
    add("STRDATA", Expressions, &[MutableString], 0, false, false);
    // Raw is used only for host/plugin statements whose grammar is intentionally
    // opaque to the core analyzer.
    add("CALLSHARP", Raw, &[], 0, false, true);
    // Emuera.NET scoped declarations have their own `name[, sizes][ = value]`
    // grammar. Keeping the tail raw prevents the ordinary expression parser from
    // treating the declaration name as a variable use or rejecting its `=`.
    for name in ["VARI", "VARS"] {
        add(name, Raw, &[ArgumentConstraint::Raw], 1, false, false);
    }

    // Keep the complete pinned instruction namespace even where the first HIR
    // version represents a specialized ArgumentBuilder as variadic expressions.
    // Focused signatures above take precedence over these catalog fallbacks.
    const KNOWN: &[&str] = &[
        "ADDCOPYCHARA",
        "ADDDEFCHARA",
        "ADDSPCHARA",
        "ADDVOIDCHARA",
        "ARRAYCOPY",
        "ARRAYREMOVE",
        "ARRAYSHIFT",
        "ARRAYSORT",
        "ASSERT",
        "BINPUT",
        "BINPUTS",
        "BREAKBUTTON",
        "CALLEVENT",
        "CALLFORMF",
        "CALLTRAIN",
        "CLEARLINE",
        "CLEARBGIMAGE",
        "CLEARTEXTBOX",
        "CUPCHECK",
        "CUSTOMDRAWLINE",
        "CVARSET",
        "DATAFORM",
        "DEBUGCLEAR",
        "DEBUGPRINT",
        "DEBUGPRINTFORM",
        "DEBUGPRINTFORML",
        "DEBUGPRINTL",
        "DELALLCHARA",
        "DRAWLINEFORM",
        "DT_COLUMN_OPTIONS",
        "ENCODETOUNI",
        "FONTSTYLE",
        "FORCEKANA",
        "FORCE_BEGIN",
        "FORCE_QUIT",
        "FORCE_QUIT_AND_RESTART",
        "GETTIME",
        "HTML_PRINT",
        "HTML_PRINT_ISLAND",
        "HTML_PRINT_ISLAND_CLEAR",
        "HTML_TAGSPLIT",
        "INPUTANY",
        "INPUTMOUSEKEY",
        "JUMPFORM",
        "LOADCHARA",
        "LOADGAME",
        "LOADVAR",
        "ONEBINPUT",
        "ONEBINPUTS",
        "OUTPUTLOG",
        "PICKUPCHARA",
        "PLAYBGM",
        "PLAYSOUND",
        "PRINTBUTTON",
        "PRINTBUTTONC",
        "PRINTBUTTONLC",
        "PRINTCPERLINE",
        "PRINT_IMG",
        "PRINT_RECT",
        "PRINT_SPACE",
        "PUTFORM",
        "QUIT_AND_RESTART",
        "REF",
        "REFBYNAME",
        "REMOVEBGIMAGE",
        "RESETBGCOLOR",
        "RESET_STAIN",
        "REUSELASTLINE",
        "SAVECHARA",
        "SAVEGAME",
        "SAVENOS",
        "SAVEVAR",
        "SETBGCOLORBYNAME",
        "SETBGIMAGE",
        "SETBGMVOLUME",
        "SETCOLORBYNAME",
        "SETFONT",
        "SETSOUNDVOLUME",
        "SKIPDISP",
        "SKIPLOG",
        "STOPBGM",
        "STOPCALLTRAIN",
        "STOPSOUND",
        "THROW",
        "TONEINPUT",
        "TONEINPUTS",
        "TOOLTIP_CUSTOM",
        "TOOLTIP_FORMAT",
        "TOOLTIP_IMG",
        "TOOLTIP_SETCOLOR",
        "TOOLTIP_SETDELAY",
        "TOOLTIP_SETDURATION",
        "TOOLTIP_SETFONT",
        "TOOLTIP_SETFONTSIZE",
        "TRYCALLF",
        "TRYCALLFORMF",
        "TRYCALLLIST",
        "TRYCCALL",
        "TRYCCALLFORM",
        "TRYCGOTO",
        "TRYCGOTOFORM",
        "TRYCJUMP",
        "TRYCJUMPFORM",
        "TRYGOTOLIST",
        "TRYJUMPLIST",
        "UPDATECHECK",
        "VARI",
        "VARS",
        "VARSET",
    ];
    for name in KNOWN {
        result
            .entry((*name).to_owned())
            .or_insert_with(|| instruction(name, Expressions, &[Any], 0, true, true));
    }

    const PRINT_FAMILY: &[&str] = &[
        "PRINTC",
        "PRINTCD",
        "PRINTCK",
        "PRINTD",
        "PRINTDL",
        "PRINTDW",
        "PRINTFORMC",
        "PRINTFORMCD",
        "PRINTFORMCK",
        "PRINTFORMD",
        "PRINTFORMDL",
        "PRINTFORMDW",
        "PRINTFORMK",
        "PRINTFORMKL",
        "PRINTFORMKW",
        "PRINTFORMLC",
        "PRINTFORMLCD",
        "PRINTFORMLCK",
        "PRINTFORMN",
        "PRINTFORMSD",
        "PRINTFORMSDL",
        "PRINTFORMSDW",
        "PRINTFORMSK",
        "PRINTFORMSKL",
        "PRINTFORMSKW",
        "PRINTFORMSN",
        "PRINTK",
        "PRINTKL",
        "PRINTKW",
        "PRINTLC",
        "PRINTLCD",
        "PRINTLCK",
        "PRINTN",
        "PRINTSINGLEFORMD",
        "PRINTSINGLEFORMK",
        "PRINTSINGLEFORMS",
        "PRINTSINGLEFORMSD",
        "PRINTSINGLEFORMSK",
        "PRINTSINGLEK",
        "PRINTSINGLED",
    ];
    for name in PRINT_FAMILY {
        let signature = if name.contains("FORMS") {
            instruction(name, Expressions, &[String], 1, false, false)
        } else if name.contains("FORM") {
            instruction(name, FormStyle, &[Formatted], 0, false, true)
        } else {
            instruction(name, Raw, &[ArgumentConstraint::Raw], 0, false, true)
        };
        result.insert((*name).to_owned(), signature);
    }
    const PRINT_VALUE_FAMILY: &[&str] = &[
        "PRINTVD",
        "PRINTVDL",
        "PRINTVDW",
        "PRINTVK",
        "PRINTVKL",
        "PRINTVKW",
        "PRINTVN",
        "PRINTSINGLEV",
        "PRINTSINGLEVD",
        "PRINTSINGLEVK",
    ];
    for name in PRINT_VALUE_FAMILY {
        result.insert(
            (*name).to_owned(),
            instruction(name, Expressions, &[Any], 1, true, false),
        );
    }
    const PRINT_STRING_FAMILY: &[&str] = &[
        "PRINTSD",
        "PRINTSDL",
        "PRINTSDW",
        "PRINTSK",
        "PRINTSKL",
        "PRINTSKW",
        "PRINTSN",
        "PRINTSINGLES",
        "PRINTSINGLESD",
        "PRINTSINGLESK",
    ];
    for name in PRINT_STRING_FAMILY {
        result.insert(
            (*name).to_owned(),
            instruction(name, Expressions, &[String], 1, true, false),
        );
    }
    const PRINT_DATA_FAMILY: &[&str] = &[
        "PRINTDATAD",
        "PRINTDATADL",
        "PRINTDATADW",
        "PRINTDATAK",
        "PRINTDATAKL",
        "PRINTDATAKW",
        "PRINTDATAL",
        "PRINTDATAW",
    ];
    for name in PRINT_DATA_FAMILY {
        result.insert(
            (*name).to_owned(),
            instruction(name, Expressions, &[MutableInteger], 0, false, true),
        );
    }
    result
}
