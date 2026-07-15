use std::collections::BTreeMap;

use erabasic_analyzer::{builtin_function_names, builtin_instruction_names};
use erabasic_bytecode::{HostCapability, HostEffect, HostSnapshotCapability};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HostBinding {
    pub namespace: String,
    pub name: String,
    pub abi_version: u32,
    pub effect: HostEffect,
    pub capability: HostCapability,
    pub snapshot_capability: HostSnapshotCapability,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ExecutionBinding {
    Native,
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
            Some(ExecutionBinding::Native | ExecutionBinding::Unsupported { .. }) | None => None,
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
        registry
            .bindings
            .entry(name)
            .or_insert(ExecutionBinding::Native);
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

fn register_hosts(
    registry: &mut HostRegistry,
    names: &[&str],
    namespace: &str,
    capability: HostCapability,
    may_suspend: bool,
) {
    for name in names {
        registry.register(
            *name,
            HostBinding {
                namespace: namespace.into(),
                name: name.to_ascii_lowercase(),
                abi_version: 1,
                effect: HostEffect {
                    pure: false,
                    may_suspend,
                    may_error: true,
                    mutates_runtime: true,
                },
                capability,
                snapshot_capability: if namespace == "rustyera.input"
                    && !matches!(*name, "AWAIT" | "GETKEY" | "GETKEYTRIGGERED")
                {
                    HostSnapshotCapability::StableWait
                } else if may_suspend {
                    HostSnapshotCapability::Never
                } else {
                    HostSnapshotCapability::StableWait
                },
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
    "OUTPUTLOG",
    "CHKDATA",
    "CHKCHARADATA",
    "SAVENOS",
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
];

const NETWORK: &[&str] = &["UPDATECHECK"];

pub(crate) fn extension_binding(name: &str) -> HostBinding {
    HostBinding {
        namespace: "rustyera.extension".into(),
        name: name.to_ascii_lowercase(),
        abi_version: 1,
        effect: HostEffect {
            pure: false,
            may_suspend: true,
            may_error: true,
            mutates_runtime: true,
        },
        capability: HostCapability::Extension,
        snapshot_capability: HostSnapshotCapability::Never,
    }
}
