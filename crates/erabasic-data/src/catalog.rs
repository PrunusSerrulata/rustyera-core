use std::collections::BTreeMap;

use crate::{
    IndexSpaceSchema, NameTableKind, Persistence, ProjectSchema, StorageScope, ValueType,
    VariableId, VariableSchema,
};

#[allow(clippy::too_many_arguments)]
fn add(
    variables: &mut BTreeMap<String, VariableSchema>,
    name: &str,
    value_type: ValueType,
    storage: StorageScope,
    dimensions: &[usize],
    mutable: bool,
    persistence: Persistence,
    can_forbid: bool,
) {
    variables.insert(
        name.to_owned(),
        VariableSchema {
            id: VariableId::builtin(name),
            value_type,
            storage,
            dimensions: dimensions.to_vec(),
            mutable,
            persistence,
            can_forbid,
        },
    );
}

/// Build the fixed variable catalog from the pinned `VariableCode.cs` reference.
///
/// Keeping the declaration in Rust (rather than parsing the ignored C# tree at build
/// time) makes release artifacts reproducible and exposes changes to ordinary review.
#[must_use]
#[allow(clippy::items_after_statements, clippy::too_many_lines)]
pub fn builtin_schema() -> ProjectSchema {
    let mut variables = BTreeMap::new();

    const GAME_INT_1D: &[&str] = &[
        "DAY",
        "MONEY",
        "ITEM",
        "FLAG",
        "TFLAG",
        "UP",
        "PALAMLV",
        "EXPLV",
        "EJAC",
        "DOWN",
        "RESULT",
        "COUNT",
        "TARGET",
        "ASSI",
        "MASTER",
        "NOITEM",
        "LOSEBASE",
        "SELECTCOM",
        "ASSIPLAY",
        "PREVCOM",
        "NOTUSE_14",
        "NOTUSE_15",
        "TIME",
        "ITEMSALES",
        "PLAYER",
        "NEXTCOM",
        "PBAND",
        "BOUGHT",
        "NOTUSE_1C",
        "NOTUSE_1D",
        "A",
        "B",
        "C",
        "D",
        "E",
        "F",
        "G",
        "H",
        "I",
        "J",
        "K",
        "L",
        "M",
        "N",
        "O",
        "P",
        "Q",
        "R",
        "S",
        "T",
        "U",
        "V",
        "W",
        "X",
        "Y",
        "Z",
        "NOTUSE_38",
        "NOTUSE_39",
        "NOTUSE_3A",
        "NOTUSE_3B",
    ];
    const NOT_FORBIDDABLE_GAME_ARRAYS: &[&str] = &[
        "PALAMLV",
        "EXPLV",
        "RESULT",
        "TARGET",
        "SELECTCOM",
        "NOTUSE_1C",
        "NOTUSE_1D",
        "NOTUSE_38",
        "NOTUSE_39",
        "NOTUSE_3A",
        "NOTUSE_3B",
    ];
    for name in GAME_INT_1D {
        let length = if *name == "FLAG" { 10_000 } else { 1_000 };
        let persistence = if name.starts_with("NOTUSE_") {
            Persistence::None
        } else {
            Persistence::GameSave
        };
        add(
            &mut variables,
            name,
            ValueType::Integer,
            StorageScope::Normal,
            &[length],
            true,
            persistence,
            !NOT_FORBIDDABLE_GAME_ARRAYS.contains(name),
        );
    }

    add(
        &mut variables,
        "ITEMPRICE",
        ValueType::Integer,
        StorageScope::Normal,
        &[1_000],
        false,
        Persistence::None,
        true,
    );
    for name in ["LOCAL", "ARG"] {
        add(
            &mut variables,
            name,
            ValueType::Integer,
            StorageScope::Local,
            &[1_000],
            true,
            Persistence::None,
            true,
        );
    }
    add(
        &mut variables,
        "GLOBAL",
        ValueType::Integer,
        StorageScope::Global,
        &[1_000],
        true,
        Persistence::GlobalSave,
        true,
    );
    add(
        &mut variables,
        "RANDDATA",
        ValueType::Integer,
        StorageScope::Normal,
        &[625],
        true,
        Persistence::ExtendedSave,
        false,
    );

    add(
        &mut variables,
        "SAVESTR",
        ValueType::String,
        StorageScope::Normal,
        &[100],
        true,
        Persistence::GameSave,
        true,
    );
    add(
        &mut variables,
        "STR",
        ValueType::String,
        StorageScope::Normal,
        &[20_000],
        true,
        Persistence::None,
        true,
    );
    add(
        &mut variables,
        "RESULTS",
        ValueType::String,
        StorageScope::Normal,
        &[100],
        true,
        Persistence::None,
        false,
    );
    for name in ["LOCALS", "ARGS"] {
        add(
            &mut variables,
            name,
            ValueType::String,
            StorageScope::Local,
            &[100],
            true,
            Persistence::None,
            true,
        );
    }
    add(
        &mut variables,
        "GLOBALS",
        ValueType::String,
        StorageScope::Global,
        &[100],
        true,
        Persistence::GlobalSave,
        true,
    );
    add(
        &mut variables,
        "TSTR",
        ValueType::String,
        StorageScope::Normal,
        &[100],
        true,
        Persistence::ExtendedSave,
        true,
    );
    add(
        &mut variables,
        "SAVEDATA_TEXT",
        ValueType::String,
        StorageScope::Normal,
        &[],
        true,
        Persistence::None,
        false,
    );

    for name in ["ISASSI", "NO"] {
        add(
            &mut variables,
            name,
            ValueType::Integer,
            StorageScope::Character,
            &[],
            true,
            Persistence::GameSave,
            false,
        );
    }
    const CHARA_GAME_ARRAYS: &[&str] = &[
        "BASE", "MAXBASE", "ABL", "TALENT", "EXP", "MARK", "PALAM", "SOURCE", "EX", "CFLAG",
        "JUEL", "RELATION", "EQUIP", "TEQUIP", "STAIN", "GOTJUEL", "NOWEX",
    ];
    for name in CHARA_GAME_ARRAYS {
        let length = match *name {
            "TALENT" | "CFLAG" => 1_000,
            "JUEL" | "GOTJUEL" => 200,
            _ => 100,
        };
        add(
            &mut variables,
            name,
            ValueType::Integer,
            StorageScope::Character,
            &[length],
            true,
            Persistence::GameSave,
            *name != "STAIN",
        );
    }
    for name in ["DOWNBASE", "CUP", "CDOWN", "TCVAR"] {
        add(
            &mut variables,
            name,
            ValueType::Integer,
            StorageScope::Character,
            &[100],
            true,
            Persistence::ExtendedSave,
            true,
        );
    }
    for (name, persistence) in [
        ("NAME", Persistence::GameSave),
        ("CALLNAME", Persistence::GameSave),
        ("NICKNAME", Persistence::ExtendedSave),
        ("MASTERNAME", Persistence::ExtendedSave),
    ] {
        add(
            &mut variables,
            name,
            ValueType::String,
            StorageScope::Character,
            &[],
            true,
            persistence,
            false,
        );
    }
    add(
        &mut variables,
        "CSTR",
        ValueType::String,
        StorageScope::Character,
        &[100],
        true,
        Persistence::ExtendedSave,
        true,
    );
    add(
        &mut variables,
        "CDFLAG",
        ValueType::Integer,
        StorageScope::Character,
        &[1, 1],
        true,
        Persistence::ExtendedSave,
        true,
    );
    for name in ["DITEMTYPE", "DA", "DB", "DC", "DD", "DE"] {
        add(
            &mut variables,
            name,
            ValueType::Integer,
            StorageScope::Normal,
            &[100, 100],
            true,
            Persistence::ExtendedSave,
            true,
        );
    }
    for name in ["TA", "TB"] {
        add(
            &mut variables,
            name,
            ValueType::Integer,
            StorageScope::Normal,
            &[100, 100, 100],
            true,
            Persistence::ExtendedSave,
            true,
        );
    }

    add(
        &mut variables,
        "RAND",
        ValueType::Integer,
        StorageScope::Calculated,
        &[0],
        false,
        Persistence::None,
        false,
    );
    const CALC_INTS: &[&str] = &[
        "CHARANUM",
        "GAMEBASE_GAMECODE",
        "GAMEBASE_VERSION",
        "GAMEBASE_ALLOWVERSION",
        "GAMEBASE_DEFAULTCHARA",
        "GAMEBASE_NOITEM",
        "LASTLOAD_VERSION",
        "LASTLOAD_NO",
        "__LINE__",
        "LINECOUNT",
        "ISTIMEOUT",
        "__INT_MAX__",
        "__INT_MIN__",
    ];
    for name in CALC_INTS {
        add(
            &mut variables,
            name,
            ValueType::Integer,
            StorageScope::Calculated,
            &[],
            *name == "LINECOUNT",
            Persistence::None,
            false,
        );
    }
    const CALC_STRINGS: &[&str] = &[
        "GAMEBASE_AUTHER",
        "GAMEBASE_AUTHOR",
        "GAMEBASE_INFO",
        "GAMEBASE_YEAR",
        "GAMEBASE_TITLE",
        "GAMEBASE_URL",
        "GAMEBASE_VERSIONNAME",
        "WINDOW_TITLE",
        "__FILE__",
        "__FUNCTION__",
        "MONEYLABEL",
        "DRAWLINESTR",
        "EMUERA_VERSION",
        "LASTLOAD_TEXT",
    ];
    for name in CALC_STRINGS {
        add(
            &mut variables,
            name,
            ValueType::String,
            StorageScope::Calculated,
            &[],
            *name == "WINDOW_TITLE",
            Persistence::None,
            false,
        );
    }

    let name_lengths = [
        (NameTableKind::Abl, 100),
        (NameTableKind::Exp, 100),
        (NameTableKind::Talent, 1_000),
        (NameTableKind::Palam, 200),
        (NameTableKind::Train, 1_000),
        (NameTableKind::Mark, 100),
        (NameTableKind::Item, 1_000),
        (NameTableKind::Base, 100),
        (NameTableKind::Source, 1_000),
        (NameTableKind::Ex, 100),
        (NameTableKind::Str, 20_000),
        (NameTableKind::Equip, 100),
        (NameTableKind::Tequip, 100),
        (NameTableKind::Flag, 10_000),
        (NameTableKind::Tflag, 1_000),
        (NameTableKind::Cflag, 1_000),
        (NameTableKind::Tcvar, 100),
        (NameTableKind::Cstr, 100),
        (NameTableKind::Stain, 1_000),
        (NameTableKind::Cdflag1, 1),
        (NameTableKind::Cdflag2, 1),
        (NameTableKind::Strname, 20_000),
        (NameTableKind::Tstr, 100),
        (NameTableKind::Savestr, 100),
        (NameTableKind::Global, 1_000),
        (NameTableKind::Globals, 100),
        (NameTableKind::Day, 100),
        (NameTableKind::Time, 100),
        (NameTableKind::Money, 100),
    ];
    let mut index_spaces = BTreeMap::new();
    for (kind, length) in name_lengths {
        index_spaces.insert(kind, IndexSpaceSchema { kind, length });
        add(
            &mut variables,
            kind.variable_name(),
            ValueType::String,
            StorageScope::Constant,
            &[length],
            false,
            Persistence::None,
            kind != NameTableKind::Str,
        );
    }

    ProjectSchema {
        variables,
        user_variable_order: Vec::new(),
        index_spaces,
    }
}
