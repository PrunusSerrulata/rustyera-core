use std::collections::BTreeMap;

use erabasic_analyzer::{builtin_function_names, builtin_instruction_names};
use erabasic_bytecode::{
    CandidatePolicy, CapabilityFallback, HostCapability, HostEffect, HostSnapshotCapability,
    OperationContract, OperationDebugPolicy, OperationHotReloadPolicy, OperationPersistence,
    OperationSnapshotPolicy, OperationState, OperationWaitPolicy, TransactionPolicy,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HostBinding {
    pub namespace: String,
    pub name: String,
    pub abi_version: u32,
    pub effect: HostEffect,
    pub capability: HostCapability,
    pub snapshot_capability: HostSnapshotCapability,
    pub contract: OperationContract,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ExecutionBinding {
    Native(OperationContract),
    Host(HostBinding),
    Unsupported { reason: String },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct HostRegistry {
    bindings: BTreeMap<String, ExecutionBinding>,
}

impl HostRegistry {
    pub fn register(&mut self, era_name: impl Into<String>, binding: HostBinding) -> bool {
        self.bindings.insert(
            era_name.into().to_ascii_uppercase(),
            ExecutionBinding::Host(binding),
        );
        true
    }

    pub fn register_execution(
        &mut self,
        era_name: impl Into<String>,
        binding: ExecutionBinding,
    ) -> bool {
        self.bindings
            .insert(era_name.into().to_ascii_uppercase(), binding)
            .is_none()
    }

    #[must_use]
    pub fn classification(&self, era_name: &str) -> Option<&ExecutionBinding> {
        self.bindings.get(&era_name.to_ascii_uppercase())
    }

    #[must_use]
    pub fn resolve(&self, era_name: &str) -> Option<HostBinding> {
        match self.classification(era_name) {
            Some(ExecutionBinding::Host(binding)) => Some(binding.clone()),
            Some(ExecutionBinding::Native(_) | ExecutionBinding::Unsupported { .. }) | None => None,
        }
    }
}

#[must_use]
pub fn default_host_registry() -> HostRegistry {
    let mut registry = HostRegistry::default();
    for name in builtin_instruction_names()
        .into_iter()
        .chain(builtin_function_names())
    {
        let binding = if native_is_implemented(&name) {
            ExecutionBinding::Native(native_contract(&name))
        } else {
            ExecutionBinding::Unsupported {
                reason: "the pinned runtime has no classified implementation for this built-in"
                    .into(),
            }
        };
        registry.bindings.entry(name).or_insert(binding);
    }

    register_hosts(
        &mut registry,
        INPUT,
        "rustyera.input",
        HostCapability::Input,
        true,
    );
    register_hosts(
        &mut registry,
        TEXT,
        "rustyera.text",
        HostCapability::Text,
        false,
    );
    register_hosts(
        &mut registry,
        CLOCK,
        "rustyera.clock",
        HostCapability::Clock,
        true,
    );
    register_hosts(
        &mut registry,
        GRAPHICS,
        "rustyera.graphics",
        HostCapability::Graphics,
        true,
    );
    register_hosts(
        &mut registry,
        AUDIO,
        "rustyera.audio",
        HostCapability::Audio,
        true,
    );
    register_hosts(
        &mut registry,
        STORAGE,
        "rustyera.storage",
        HostCapability::Storage,
        true,
    );
    register_hosts(
        &mut registry,
        SYSTEM,
        "rustyera.system",
        HostCapability::System,
        true,
    );
    register_hosts(
        &mut registry,
        NETWORK,
        "rustyera.network",
        HostCapability::Network,
        true,
    );
    registry.register_execution(
        "CALLSHARP",
        ExecutionBinding::Unsupported {
            reason:
                "CLR plugins are intentionally unsupported; use the versioned Host extension ABI"
                    .into(),
        },
    );
    registry
}

fn native_is_implemented(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("map_")
        || name.starts_with("xml_")
        || name.starts_with("dt_")
        || IMPLEMENTED_NATIVE_NAMES.contains(&name.as_str())
}

const IMPLEMENTED_NATIVE_NAMES: &[&str] = &[
    "abs",
    "sign",
    "sqrt",
    "cbrt",
    "log",
    "log10",
    "exponent",
    "power",
    "getbit",
    "bitcount",
    "strlen",
    "strlenu",
    "toint",
    "isnumeric",
    "unicode",
    "convert",
    "color_fromrgb",
    "max",
    "min",
    "limit",
    "inrange",
    "tostr",
    "substring",
    "substringu",
    "strfind",
    "strfindu",
    "strcount",
    "strlens",
    "strlensu",
    "getpalamlv",
    "getexplv",
    "replace",
    "escape",
    "unicodetostr",
    "encodetouni",
    "unicodebyte",
    "charatu",
    "tolower",
    "toupper",
    "rand",
    "randomize",
    "initrand",
    "dumprand",
    "swap",
    "swapvar",
    "setbit",
    "clearbit",
    "invertbit",
    "split",
    "getnum",
    "strjoin",
    "arrayremove",
    "arrayshift",
    "arraysort",
    "arraycopy",
    "varset",
    "cvarset",
    "arraymsort",
    "arraymsortex",
    "findelement",
    "findlastelement",
    "regexpmatch",
    "sumarray",
    "sumcarray",
    "maxarray",
    "maxcarray",
    "minarray",
    "mincarray",
    "match",
    "cmatch",
    "inrangearray",
    "inrangecarray",
    "groupmatch",
    "nosames",
    "allsames",
    "charanum",
    "getchara",
    "getspchara",
    "existcsv",
    "csvname",
    "csvcallname",
    "csvnickname",
    "csvmastername",
    "csvcstr",
    "csvbase",
    "csvabl",
    "csvmark",
    "csvexp",
    "csvrelation",
    "csvtalent",
    "csvcflag",
    "csvequip",
    "csvjuel",
    "findchara",
    "findlastchara",
    "addchara",
    "addspchara",
    "adddefchara",
    "addvoidchara",
    "delchara",
    "delallchara",
    "swapchara",
    "copychara",
    "addcopychara",
    "pickupchara",
    "sortchara",
    "reset_stain",
];

fn register_hosts(
    registry: &mut HostRegistry,
    names: &[&str],
    namespace: &str,
    capability: HostCapability,
    _may_suspend: bool,
) {
    for name in names {
        let contract = host_contract(namespace, name);
        registry.register(
            *name,
            HostBinding {
                namespace: namespace.into(),
                name: name.to_ascii_lowercase(),
                abi_version: 1,
                effect: contract.effect(),
                capability,
                snapshot_capability: contract.snapshot_capability(),
                contract,
            },
        );
    }
}

const INPUT: &[&str] = &[
    "WAIT",
    "WAITANYKEY",
    "FORCEWAIT",
    "TWAIT",
    "AWAIT",
    "INPUT",
    "INPUTS",
    "ONEINPUT",
    "ONEINPUTS",
    "TINPUT",
    "TINPUTS",
    "TONEINPUT",
    "TONEINPUTS",
    "INPUTANY",
    "BINPUT",
    "BINPUTS",
    "ONEBINPUT",
    "ONEBINPUTS",
    "INPUTMOUSEKEY",
    "GETKEY",
    "GETKEYTRIGGERED",
    "GETTEXTBOX",
    "SETTEXTBOX",
    "CLEARTEXTBOX",
    "ISACTIVE",
    "HOTKEY_STATE",
    "HOTKEY_STATE_INIT",
    "MOUSEX",
    "MOUSEY",
    "MOUSEB",
    "FLOWINPUT",
    "FLOWINPUTS",
    "BREAKBUTTON",
];

const TEXT: &[&str] = &[
    "PRINT",
    "PRINTL",
    "PRINTW",
    "PRINTV",
    "PRINTVL",
    "PRINTVW",
    "PRINTS",
    "PRINTSL",
    "PRINTSW",
    "PRINTFORM",
    "PRINTFORML",
    "PRINTFORMW",
    "PRINTFORMS",
    "PRINTFORMSL",
    "PRINTFORMSW",
    "PRINTC",
    "CLEARLINE",
    "REUSELASTLINE",
    "DRAWLINE",
    "BAR",
    "BARL",
    "PRINTBUTTON",
    "PRINTBUTTONC",
    "PRINTBUTTONLC",
    "PRINTPLAIN",
    "PRINTPLAINFORM",
    "PRINTSINGLE",
    "PRINTSINGLEFORM",
    "PRINTDATA",
    "PRINTDATAL",
    "PRINTDATAW",
    "HTML_PRINT",
    "HTML_PRINT_ISLAND",
    "HTML_PRINT_ISLAND_CLEAR",
    "HTML_TAGSPLIT",
    "PRINT_IMG",
    "PRINT_RECT",
    "PRINT_SPACE",
    "DEBUGPRINT",
    "DEBUGPRINTL",
    "DEBUGPRINTFORM",
    "DEBUGPRINTFORML",
    "DEBUGCLEAR",
    "OUTPUTLOG",
    "GETDISPLAYLINE",
    "GETLINESTR",
    "HTML_GETPRINTEDSTR",
    "HTML_POPPRINTINGSTR",
    "LINEISEMPTY",
    "PRINTLC",
    "PRINTFORMC",
    "PRINTFORMLC",
    "PRINTSINGLEV",
    "PRINTSINGLES",
    "PRINTSINGLEFORMS",
    "PRINTCPERLINE",
    "PRINTK",
    "PRINTKL",
    "PRINTKW",
    "PRINTVK",
    "PRINTVKL",
    "PRINTVKW",
    "PRINTSK",
    "PRINTSKL",
    "PRINTSKW",
    "PRINTFORMK",
    "PRINTFORMKL",
    "PRINTFORMKW",
    "PRINTFORMSK",
    "PRINTFORMSKL",
    "PRINTFORMSKW",
    "PRINTCK",
    "PRINTLCK",
    "PRINTFORMCK",
    "PRINTFORMLCK",
    "PRINTSINGLEK",
    "PRINTSINGLEVK",
    "PRINTSINGLESK",
    "PRINTSINGLEFORMK",
    "PRINTSINGLEFORMSK",
    "PRINTDATAK",
    "PRINTDATAKL",
    "PRINTDATAKW",
    "PRINTD",
    "PRINTDL",
    "PRINTDW",
    "PRINTVD",
    "PRINTVDL",
    "PRINTVDW",
    "PRINTSD",
    "PRINTSDL",
    "PRINTSDW",
    "PRINTFORMD",
    "PRINTFORMDL",
    "PRINTFORMDW",
    "PRINTFORMSD",
    "PRINTFORMSDL",
    "PRINTFORMSDW",
    "PRINTCD",
    "PRINTLCD",
    "PRINTFORMCD",
    "PRINTFORMLCD",
    "PRINTSINGLED",
    "PRINTSINGLEVD",
    "PRINTSINGLESD",
    "PRINTSINGLEFORMD",
    "PRINTSINGLEFORMSD",
    "PRINTDATAD",
    "PRINTDATADL",
    "PRINTDATADW",
    "ASSERT",
    "THROW",
    "FORCEKANA",
    "UPCHECK",
    "CUPCHECK",
    "CUSTOMDRAWLINE",
    "DRAWLINEFORM",
    "PRINT_ABL",
    "PRINT_TALENT",
    "PRINT_MARK",
    "PRINT_EXP",
    "PRINT_PALAM",
    "PRINT_ITEM",
    "PRINT_SHOPITEM",
    "PRINTN",
    "PRINTVN",
    "PRINTSN",
    "PRINTFORMN",
    "PRINTFORMSN",
    "SETCOLOR",
    "SETCOLORBYNAME",
    "RESETCOLOR",
    "SETBGCOLOR",
    "SETBGCOLORBYNAME",
    "RESETBGCOLOR",
    "FONTBOLD",
    "FONTITALIC",
    "FONTREGULAR",
    "FONTSTYLE",
    "ALIGNMENT",
    "SETFONT",
    "REDRAW",
    "SKIPDISP",
    "NOSKIP",
    "ENDNOSKIP",
    "SKIPLOG",
    "TOOLTIP_SETCOLOR",
    "TOOLTIP_SETDELAY",
    "TOOLTIP_SETDURATION",
    "TOOLTIP_SETFONT",
    "TOOLTIP_SETFONTSIZE",
    "TOOLTIP_CUSTOM",
    "TOOLTIP_FORMAT",
    "TOOLTIP_IMG",
    "CURRENTALIGN",
    "CURRENTREDRAW",
    "GETBGCOLOR",
    "GETCOLOR",
    "GETDEFBGCOLOR",
    "GETDEFCOLOR",
    "GETFOCUSCOLOR",
    "GETFONT",
    "GETSTYLE",
    "HTML_STRINGLEN",
    "HTML_STRINGLINES",
    "HTML_ESCAPE",
    "HTML_SUBSTRING",
    "HTML_TOPLAINTEXT",
    "PRINTCLENGTH",
    "BARSTR",
    "MONEYSTR",
    "TOSTR",
    "TOFULL",
    "TOHALF",
    "MESSKIP",
    "MOUSESKIP",
    "ISSKIP",
];

const CLOCK: &[&str] = &["GETTIME", "GETTIMES", "GETMILLISECOND", "GETSECOND"];

const GRAPHICS: &[&str] = &[
    "SETBGIMAGE",
    "CLEARBGIMAGE",
    "REMOVEBGIMAGE",
    "CHKFONT",
    "CLIENTHEIGHT",
    "CLIENTWIDTH",
    "GCREATE",
    "GCREATEFROMFILE",
    "GDISPOSE",
    "GLOAD",
    "GSAVE",
    "GDRAWG",
    "GDRAWGWITHMASK",
    "GDRAWGWITHROTATE",
    "GDRAWLINE",
    "GDRAWSPRITE",
    "GDRAWTEXT",
    "SPRITECREATE",
    "SPRITEANIMECREATE",
    "SPRITEANIMEADDFRAME",
    "SPRITEDISPOSE",
    "SPRITEDISPOSEALL",
    "SPRITEMOVE",
    "SPRITESETPOS",
    "SPRITEWIDTH",
    "SPRITEHEIGHT",
    "BITMAP_CACHE_ENABLE",
    "CBGCLEAR",
    "CBGCLEARBUTTON",
    "CBGREMOVEBMAP",
    "CBGREMOVERANGE",
    "CBGSETBMAPG",
    "CBGSETBUTTONSPRITE",
    "CBGSETG",
    "CBGSETSPRITE",
    "GCLEAR",
    "GCREATED",
    "GHEIGHT",
    "GWIDTH",
    "GSETBRUSH",
    "GSETCOLOR",
    "GSETFONT",
    "GSETPEN",
    "GGETBRUSH",
    "GGETFONT",
    "GGETPEN",
    "SPRITECREATED",
    "SPRITEGETCOLOR",
    "SPRITEPOSX",
    "SPRITEPOSY",
    "SETANIMETIMER",
];

const AUDIO: &[&str] = &[
    "PLAYSOUND",
    "STOPSOUND",
    "PLAYBGM",
    "STOPBGM",
    "SETSOUNDVOLUME",
    "SETBGMVOLUME",
    "EXISTSOUND",
];

const STORAGE: &[&str] = &[
    "SAVEDATA",
    "LOADDATA",
    "DELDATA",
    "SAVEGLOBAL",
    "LOADGLOBAL",
    "SAVEVAR",
    "LOADVAR",
    "SAVECHARA",
    "LOADCHARA",
    "SAVETEXT",
    "LOADTEXT",
    "EXISTFILE",
    "ENUMFILES",
    "FIND_CHARADATA",
    "OUTPUTLOG",
    "CHKDATA",
    "CHKCHARADATA",
    "SAVENOS",
    "PUTFORM",
    "RESETDATA",
    "RESETGLOBAL",
];

const SYSTEM: &[&str] = &[
    "BEGIN",
    "FORCE_BEGIN",
    "SAVEGAME",
    "LOADGAME",
    "DOTRAIN",
    "QUIT",
    "QUIT_AND_RESTART",
    "FORCE_QUIT",
    "FORCE_QUIT_AND_RESTART",
    "CALLTRAIN",
    "STOPCALLTRAIN",
    "GETMEMORYUSAGE",
    "GETCONFIG",
    "GETCONFIGS",
    "VARSIZE",
    "EXISTFUNCTION",
    "EXISTVAR",
    "GETDOINGFUNCTION",
    "ENUMFUNCBEGINSWITH",
    "ENUMFUNCENDSWITH",
    "ENUMFUNCWITH",
    "ENUMVARBEGINSWITH",
    "ENUMVARENDSWITH",
    "ENUMVARWITH",
];

const NETWORK: &[&str] = &["UPDATECHECK"];

pub(crate) fn extension_binding(name: &str) -> HostBinding {
    let contract = OperationContract {
        state: OperationState::External,
        transaction: TransactionPolicy::Forbidden,
        candidate: CandidatePolicy::Forbidden,
        persistence: OperationPersistence::RuntimeOnly,
        snapshot: OperationSnapshotPolicy::PendingBlocks,
        hot_reload: OperationHotReloadPolicy::ActiveBlocks,
        wait: OperationWaitPolicy::TransientExternal,
        capability_fallback: CapabilityFallback::Unsupported,
        debug: OperationDebugPolicy::Forbidden,
        portability: erabasic_bytecode::OperationPortability::ExtensionDefined,
    };
    HostBinding {
        namespace: "rustyera.extension".into(),
        name: name.to_ascii_lowercase(),
        abi_version: 1,
        effect: contract.effect(),
        capability: HostCapability::Extension,
        snapshot_capability: HostSnapshotCapability::Never,
        contract,
    }
}

fn native_contract(name: &str) -> OperationContract {
    let name = name.to_ascii_lowercase();
    let structured =
        name.starts_with("map_") || name.starts_with("xml_") || name.starts_with("dt_");
    let random = matches!(
        name.as_str(),
        "rand" | "randomize" | "initrand" | "dumprand"
    );
    let variable_mutation = matches!(
        name.as_str(),
        "swap"
            | "swapvar"
            | "arrayremove"
            | "arrayshift"
            | "arraysort"
            | "arraycopy"
            | "varset"
            | "cvarset"
            | "arraymsort"
            | "arraymsortex"
            | "addchara"
            | "addspchara"
            | "adddefchara"
            | "addvoidchara"
            | "delchara"
            | "delallchara"
            | "swapchara"
            | "copychara"
            | "addcopychara"
            | "pickupchara"
            | "sortchara"
            | "reset_stain"
            | "setbit"
            | "clearbit"
            | "invertbit"
            | "split"
    );
    let mutable = structured || random || variable_mutation;
    OperationContract {
        state: if structured || random {
            OperationState::Native
        } else if variable_mutation {
            OperationState::Vm
        } else {
            OperationState::Pure
        },
        transaction: if mutable {
            TransactionPolicy::CloneCommit
        } else {
            TransactionPolicy::ReadOnly
        },
        candidate: if mutable {
            CandidatePolicy::CloneCommit
        } else {
            CandidatePolicy::ReadOnly
        },
        persistence: if structured {
            OperationPersistence::ExtensionScoped
        } else if variable_mutation {
            OperationPersistence::VariableScoped
        } else if random {
            OperationPersistence::RuntimeOnly
        } else {
            OperationPersistence::None
        },
        snapshot: OperationSnapshotPolicy::Included,
        hot_reload: OperationHotReloadPolicy::Preserve,
        wait: OperationWaitPolicy::Immediate,
        capability_fallback: CapabilityFallback::NotApplicable,
        debug: if mutable {
            OperationDebugPolicy::Transactional
        } else {
            OperationDebugPolicy::Pure
        },
        portability: erabasic_bytecode::OperationPortability::Portable,
    }
}

#[allow(clippy::too_many_lines)]
fn host_contract(namespace: &str, name: &str) -> OperationContract {
    let (state, transaction, persistence, snapshot, hot_reload, wait, fallback) = match namespace {
        "rustyera.text"
            if matches!(name, "BARSTR" | "MONEYSTR" | "TOSTR" | "TOFULL" | "TOHALF") =>
        {
            (
                OperationState::Controller,
                TransactionPolicy::ReadOnly,
                OperationPersistence::ProjectDerived,
                OperationSnapshotPolicy::Included,
                OperationHotReloadPolicy::Rebuild,
                OperationWaitPolicy::Immediate,
                CapabilityFallback::CanonicalProjection,
            )
        }
        "rustyera.text" => (
            OperationState::Presentation,
            TransactionPolicy::CloneCommit,
            OperationPersistence::RuntimeOnly,
            OperationSnapshotPolicy::Included,
            OperationHotReloadPolicy::Preserve,
            OperationWaitPolicy::Immediate,
            CapabilityFallback::CanonicalProjection,
        ),
        "rustyera.audio" => (
            OperationState::Presentation,
            TransactionPolicy::BufferedEffect,
            OperationPersistence::RuntimeOnly,
            OperationSnapshotPolicy::Included,
            OperationHotReloadPolicy::Rebuild,
            OperationWaitPolicy::Immediate,
            CapabilityFallback::IntentNoOp,
        ),
        "rustyera.graphics" if matches!(name, "GLOAD" | "GSAVE" | "GCREATEFROMFILE") => (
            OperationState::External,
            TransactionPolicy::Forbidden,
            OperationPersistence::ProjectDerived,
            OperationSnapshotPolicy::PendingBlocks,
            OperationHotReloadPolicy::ActiveBlocks,
            OperationWaitPolicy::TransientExternal,
            CapabilityFallback::Unsupported,
        ),
        "rustyera.graphics" => (
            OperationState::Presentation,
            TransactionPolicy::CloneCommit,
            OperationPersistence::RuntimeOnly,
            OperationSnapshotPolicy::Included,
            OperationHotReloadPolicy::Rebuild,
            OperationWaitPolicy::Immediate,
            CapabilityFallback::ScriptResult,
        ),
        "rustyera.input"
            if matches!(
                name,
                "GETKEY" | "GETKEYTRIGGERED" | "MOUSEX" | "MOUSEY" | "MOUSEB" | "AWAIT"
            ) =>
        {
            (
                OperationState::Controller,
                TransactionPolicy::Forbidden,
                OperationPersistence::RuntimeOnly,
                OperationSnapshotPolicy::PendingBlocks,
                OperationHotReloadPolicy::ActiveBlocks,
                OperationWaitPolicy::TransientExternal,
                CapabilityFallback::ScriptResult,
            )
        }
        "rustyera.input"
            if matches!(
                name,
                "GETTEXTBOX"
                    | "SETTEXTBOX"
                    | "CLEARTEXTBOX"
                    | "HOTKEY_STATE"
                    | "HOTKEY_STATE_INIT"
                    | "FLOWINPUT"
                    | "FLOWINPUTS"
                    | "BREAKBUTTON"
                    | "ISACTIVE"
            ) =>
        {
            (
                OperationState::Controller,
                TransactionPolicy::Forbidden,
                OperationPersistence::RuntimeOnly,
                OperationSnapshotPolicy::Included,
                OperationHotReloadPolicy::Preserve,
                OperationWaitPolicy::Immediate,
                CapabilityFallback::ScriptResult,
            )
        }
        "rustyera.input" => (
            OperationState::Controller,
            TransactionPolicy::Forbidden,
            OperationPersistence::RuntimeOnly,
            OperationSnapshotPolicy::Included,
            OperationHotReloadPolicy::Preserve,
            OperationWaitPolicy::StableInput,
            CapabilityFallback::ScriptResult,
        ),
        "rustyera.clock" | "rustyera.network" => (
            OperationState::External,
            TransactionPolicy::Forbidden,
            OperationPersistence::None,
            OperationSnapshotPolicy::PendingBlocks,
            OperationHotReloadPolicy::ActiveBlocks,
            OperationWaitPolicy::TransientExternal,
            CapabilityFallback::ScriptResult,
        ),
        "rustyera.storage" if name == "PUTFORM" => (
            OperationState::Vm,
            TransactionPolicy::CloneCommit,
            OperationPersistence::Ordinary,
            OperationSnapshotPolicy::Included,
            OperationHotReloadPolicy::Preserve,
            OperationWaitPolicy::Immediate,
            CapabilityFallback::NotApplicable,
        ),
        "rustyera.storage" if name == "SAVENOS" => (
            OperationState::Vm,
            TransactionPolicy::ReadOnly,
            OperationPersistence::None,
            OperationSnapshotPolicy::Included,
            OperationHotReloadPolicy::Preserve,
            OperationWaitPolicy::Immediate,
            CapabilityFallback::NotApplicable,
        ),
        "rustyera.storage" => (
            OperationState::External,
            TransactionPolicy::Forbidden,
            OperationPersistence::Ordinary,
            OperationSnapshotPolicy::PendingBlocks,
            OperationHotReloadPolicy::ActiveBlocks,
            OperationWaitPolicy::TransientExternal,
            CapabilityFallback::Unsupported,
        ),
        "rustyera.system" if matches!(name, "SAVEGAME" | "LOADGAME") => (
            OperationState::Controller,
            TransactionPolicy::Forbidden,
            OperationPersistence::RuntimeOnly,
            OperationSnapshotPolicy::Included,
            OperationHotReloadPolicy::Preserve,
            OperationWaitPolicy::StableInput,
            CapabilityFallback::NotApplicable,
        ),
        "rustyera.system"
            if matches!(
                name,
                "ENUMFUNCBEGINSWITH"
                    | "ENUMFUNCENDSWITH"
                    | "ENUMFUNCWITH"
                    | "ENUMVARBEGINSWITH"
                    | "ENUMVARENDSWITH"
                    | "ENUMVARWITH"
            ) =>
        {
            (
                OperationState::Vm,
                TransactionPolicy::CloneCommit,
                OperationPersistence::VariableScoped,
                OperationSnapshotPolicy::Included,
                OperationHotReloadPolicy::Preserve,
                OperationWaitPolicy::Immediate,
                CapabilityFallback::NotApplicable,
            )
        }
        "rustyera.system"
            if matches!(
                name,
                "GETCONFIG"
                    | "GETCONFIGS"
                    | "VARSIZE"
                    | "EXISTFUNCTION"
                    | "EXISTVAR"
                    | "GETDOINGFUNCTION"
            ) =>
        {
            (
                OperationState::Controller,
                TransactionPolicy::ReadOnly,
                OperationPersistence::ProjectDerived,
                OperationSnapshotPolicy::Included,
                OperationHotReloadPolicy::Rebuild,
                OperationWaitPolicy::Immediate,
                CapabilityFallback::CanonicalProjection,
            )
        }
        "rustyera.system" => (
            OperationState::Controller,
            TransactionPolicy::CloneCommit,
            OperationPersistence::RuntimeOnly,
            OperationSnapshotPolicy::Included,
            OperationHotReloadPolicy::Preserve,
            OperationWaitPolicy::Immediate,
            CapabilityFallback::NotApplicable,
        ),
        _ => (
            OperationState::External,
            TransactionPolicy::Forbidden,
            OperationPersistence::RuntimeOnly,
            OperationSnapshotPolicy::PendingBlocks,
            OperationHotReloadPolicy::ActiveBlocks,
            OperationWaitPolicy::TransientExternal,
            CapabilityFallback::Unsupported,
        ),
    };
    OperationContract {
        state,
        transaction,
        candidate: match (namespace, wait, transaction) {
            ("rustyera.clock", _, TransactionPolicy::Forbidden) => CandidatePolicy::FrozenClock,
            (_, OperationWaitPolicy::StableInput | OperationWaitPolicy::TransientExternal, _) => {
                CandidatePolicy::Forbidden
            }
            (_, _, TransactionPolicy::ReadOnly) => CandidatePolicy::ReadOnly,
            (_, _, TransactionPolicy::CloneCommit) => CandidatePolicy::CloneCommit,
            (_, _, TransactionPolicy::BufferedEffect) => CandidatePolicy::BufferedEffect,
            (_, _, TransactionPolicy::Forbidden) => CandidatePolicy::Forbidden,
        },
        persistence,
        snapshot,
        hot_reload,
        wait,
        capability_fallback: fallback,
        // The debugger deliberately rejects every Host import, including reference
        // METHOD_SAFE printing and media commands.
        debug: OperationDebugPolicy::Forbidden,
        portability: if matches!(
            name,
            "GETTEXTBOX"
                | "MOUSEX"
                | "MOUSEY"
                | "MOUSEB"
                | "GETKEY"
                | "GETKEYTRIGGERED"
                | "CLIENTWIDTH"
                | "CLIENTHEIGHT"
                | "GETLINESTR"
        ) {
            erabasic_bytecode::OperationPortability::FrontendObservation
        } else if matches!(namespace, "rustyera.audio" | "rustyera.network") {
            erabasic_bytecode::OperationPortability::PlatformIntent
        } else {
            erabasic_bytecode::OperationPortability::Portable
        },
    }
}
