use std::collections::{BTreeMap, BTreeSet};

use erabasic_hir::SemanticType;
use erabasic_parser::ArgumentStyle;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArgumentConstraint {
    Integer,
    String,
    Any,
    MutableInteger,
    MutableString,
    MutableAny,
    ReferenceAny,
    ReferenceOrString,
    MutableReferenceOrString,
    IntegerOrReference,
    /// An integer value or a mutable string place. This models legacy APIs
    /// which use a nonzero integer to select RESULTS and a string array as an
    /// explicit output target.
    IntegerOrMutableString,
    Formatted,
    Raw,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionCallableKind {
    Instruction,
    ExpressionFunction,
}

/// Portability provenance shared by semantic analysis and bytecode lowering.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallablePortability {
    Portable,
    FrontendObservation,
    PlatformIntent,
    ExtensionDefined,
}

/// Classify built-ins whose result depends on the authoritative frontend.
/// Keeping this list in the language catalog prevents analyzer and compiler
/// diagnostics from silently disagreeing as new compatibility calls are added.
#[must_use]
pub fn builtin_callable_portability(name: &str) -> CallablePortability {
    if matches!(
        name.to_ascii_uppercase().as_str(),
        "CHKFONT"
            | "GETTEXTBOX"
            | "MOUSEX"
            | "MOUSEY"
            | "MOUSEB"
            | "GETKEY"
            | "GETKEYTRIGGERED"
            | "CLIENTWIDTH"
            | "CLIENTHEIGHT"
            | "GETLINESTR"
            | "GETDISPLAYLINE"
            | "HTML_GETPRINTEDSTR"
            | "HTML_STRINGLEN"
            | "HTML_SUBSTRING"
            | "HTML_STRINGLINES"
            | "GGETTEXTSIZE"
            | "GGETCOLOR"
    ) {
        CallablePortability::FrontendObservation
    } else {
        CallablePortability::Portable
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CallableSignature {
    pub name: String,
    pub return_type: SemanticType,
    pub arguments: Vec<ArgumentConstraint>,
    pub minimum_arguments: usize,
    pub variadic: bool,
    pub allow_omitted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InstructionSignature {
    pub name: String,
    pub argument_style: ArgumentStyle,
    pub arguments: Vec<ArgumentConstraint>,
    pub minimum_arguments: usize,
    pub variadic: bool,
    pub allow_omitted: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExtensionRegistry {
    pub instructions: BTreeMap<String, InstructionSignature>,
    pub functions: BTreeMap<String, CallableSignature>,
}

impl ExtensionRegistry {
    pub fn register_instruction(&mut self, mut signature: InstructionSignature) -> bool {
        let key = signature.name.to_ascii_uppercase();
        signature.name.clone_from(&key);
        self.instructions.insert(key, signature).is_none()
    }

    pub fn register_function(&mut self, mut signature: CallableSignature) -> bool {
        let key = signature.name.to_ascii_uppercase();
        signature.name.clone_from(&key);
        self.functions.insert(key, signature).is_none()
    }
}

pub(crate) struct Catalog {
    pub instructions: BTreeMap<String, InstructionSignature>,
    pub functions: BTreeMap<String, CallableSignature>,
    pub extension_instructions: BTreeSet<String>,
    pub extension_functions: BTreeSet<String>,
}

/// Return the pinned built-in instruction namespace in deterministic order.
///
/// Execution layers use this inventory to require an explicit Native/Host or
/// unsupported classification for every analyzer-visible built-in.
#[must_use]
pub fn builtin_instruction_names() -> Vec<String> {
    builtin_instructions().into_keys().collect()
}

/// Return the pinned built-in expression-function namespace in deterministic order.
#[must_use]
pub fn builtin_function_names() -> Vec<String> {
    builtin_functions().into_keys().collect()
}

impl Catalog {
    pub fn build(extensions: &ExtensionRegistry) -> Self {
        let mut catalog = Self {
            instructions: builtin_instructions(),
            functions: builtin_functions(),
            extension_instructions: extensions
                .instructions
                .keys()
                .map(|name| name.to_ascii_uppercase())
                .collect(),
            extension_functions: extensions
                .functions
                .keys()
                .map(|name| name.to_ascii_uppercase())
                .collect(),
        };
        // Emuera plugins are registered after built-ins. They may add names but may
        // not silently replace the core instruction identity used by the loader.
        for (name, signature) in &extensions.instructions {
            catalog
                .instructions
                .entry(name.to_ascii_uppercase())
                .or_insert_with(|| signature.clone());
        }
        for (name, signature) in &extensions.functions {
            catalog
                .functions
                .entry(name.to_ascii_uppercase())
                .or_insert_with(|| signature.clone());
        }
        catalog
    }
}

fn instruction(
    name: &str,
    style: ArgumentStyle,
    arguments: &[ArgumentConstraint],
    minimum_arguments: usize,
    variadic: bool,
    allow_omitted: bool,
) -> InstructionSignature {
    InstructionSignature {
        name: name.to_owned(),
        argument_style: style,
        arguments: arguments.to_vec(),
        minimum_arguments,
        variadic,
        allow_omitted,
    }
}

#[allow(clippy::items_after_statements, clippy::too_many_lines)]
fn builtin_instructions() -> BTreeMap<String, InstructionSignature> {
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

#[allow(clippy::items_after_statements, clippy::too_many_lines)]
fn builtin_functions() -> BTreeMap<String, CallableSignature> {
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
        "ENUMFILES",
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
