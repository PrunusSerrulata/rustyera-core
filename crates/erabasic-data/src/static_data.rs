use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    CharacterSelection, DeferredIndexCatalog, NewGameSeed, ProjectSchema, RuntimeDefaults,
    SaveCompatibility, SaveLoadContext,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NameTableKind {
    Abl,
    Exp,
    Talent,
    Palam,
    Train,
    Mark,
    Item,
    Base,
    Source,
    Ex,
    Str,
    Equip,
    Tequip,
    Flag,
    Tflag,
    Cflag,
    Tcvar,
    Cstr,
    Stain,
    Cdflag1,
    Cdflag2,
    Strname,
    Tstr,
    Savestr,
    Global,
    Globals,
    Day,
    Time,
    Money,
}

impl NameTableKind {
    pub const ALL: [Self; 29] = [
        Self::Abl,
        Self::Exp,
        Self::Talent,
        Self::Palam,
        Self::Train,
        Self::Mark,
        Self::Item,
        Self::Base,
        Self::Source,
        Self::Ex,
        Self::Str,
        Self::Equip,
        Self::Tequip,
        Self::Flag,
        Self::Tflag,
        Self::Cflag,
        Self::Tcvar,
        Self::Cstr,
        Self::Stain,
        Self::Cdflag1,
        Self::Cdflag2,
        Self::Strname,
        Self::Tstr,
        Self::Savestr,
        Self::Global,
        Self::Globals,
        Self::Day,
        Self::Time,
        Self::Money,
    ];

    #[must_use]
    pub const fn variable_name(self) -> &'static str {
        match self {
            Self::Abl => "ABLNAME",
            Self::Exp => "EXPNAME",
            Self::Talent => "TALENTNAME",
            Self::Palam => "PALAMNAME",
            Self::Train => "TRAINNAME",
            Self::Mark => "MARKNAME",
            Self::Item => "ITEMNAME",
            Self::Base => "BASENAME",
            Self::Source => "SOURCENAME",
            Self::Ex => "EXNAME",
            Self::Str => "__DUMMY_STR__",
            Self::Equip => "EQUIPNAME",
            Self::Tequip => "TEQUIPNAME",
            Self::Flag => "FLAGNAME",
            Self::Tflag => "TFLAGNAME",
            Self::Cflag => "CFLAGNAME",
            Self::Tcvar => "TCVARNAME",
            Self::Cstr => "CSTRNAME",
            Self::Stain => "STAINNAME",
            Self::Cdflag1 => "CDFLAGNAME1",
            Self::Cdflag2 => "CDFLAGNAME2",
            Self::Strname => "STRNAME",
            Self::Tstr => "TSTRNAME",
            Self::Savestr => "SAVESTRNAME",
            Self::Global => "GLOBALNAME",
            Self::Globals => "GLOBALSNAME",
            Self::Day => "DAYNAME",
            Self::Time => "TIMENAME",
            Self::Money => "MONEYNAME",
        }
    }

    /// Built-in data variables whose symbolic indices use this CSV name table.
    #[must_use]
    pub const fn data_variables(self) -> &'static [&'static str] {
        match self {
            Self::Abl => &["ABL"],
            Self::Exp => &["EXP"],
            Self::Talent => &["TALENT"],
            Self::Palam => &["PALAM", "UP", "DOWN", "JUEL", "GOTJUEL", "CUP", "CDOWN"],
            Self::Train => &["TRAIN"],
            Self::Mark => &["MARK"],
            // ITEM.csv names are shared by every item-indexed built-in variable.
            Self::Item => &["ITEM", "ITEMSALES", "ITEMPRICE", "ITEMNAME"],
            Self::Base => &["BASE", "MAXBASE", "LOSEBASE", "DOWNBASE"],
            Self::Source => &["SOURCE"],
            Self::Ex => &["EX", "NOWEX"],
            // STR.CSV contains initial string values. Symbolic STR indices come from
            // STRNAME.CSV in the reference implementation.
            Self::Str => &[],
            Self::Equip => &["EQUIP"],
            Self::Tequip => &["TEQUIP"],
            Self::Flag => &["FLAG"],
            Self::Tflag => &["TFLAG"],
            Self::Cflag => &["CFLAG"],
            Self::Tcvar => &["TCVAR"],
            Self::Cstr => &["CSTR"],
            Self::Stain => &["STAIN"],
            Self::Cdflag1 | Self::Cdflag2 => &["CDFLAG"],
            Self::Strname => &["STR", "STRNAME"],
            Self::Tstr => &["TSTR"],
            Self::Savestr => &["SAVESTR"],
            Self::Global => &["GLOBAL"],
            Self::Globals => &["GLOBALS"],
            Self::Day => &["DAY"],
            Self::Time => &["TIME"],
            Self::Money => &["MONEY"],
        }
    }

    /// Zero-based data dimension to which this name table applies.
    #[must_use]
    pub const fn data_dimension(self) -> usize {
        if matches!(self, Self::Cdflag2) { 1 } else { 0 }
    }

    /// Find the CSV name table used by a built-in variable data dimension.
    #[must_use]
    pub fn for_data_variable(variable: &str, dimension: usize) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| {
            kind.data_dimension() == dimension
                && kind
                    .data_variables()
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(variable))
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NameAlias {
    pub name: String,
    pub index: i32,
}

/// Names preserve declared holes. `lookup` records Emuera's first-name-wins followed by
/// aliases-that-do-not-shadow-names rule.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NameTable {
    pub names: Vec<Option<String>>,
    pub aliases: Vec<NameAlias>,
    pub lookup: BTreeMap<String, i32>,
}

impl NameTable {
    #[must_use]
    pub fn empty(length: usize) -> Self {
        Self {
            names: vec![None; length],
            aliases: Vec::new(),
            lookup: BTreeMap::new(),
        }
    }

    pub fn rebuild_lookup(&mut self) {
        self.lookup.clear();
        for (index, name) in self.names.iter().enumerate() {
            if let Some(name) = name.as_ref().filter(|name| !name.is_empty())
                && let Ok(index) = i32::try_from(index)
            {
                self.lookup.entry(name.clone()).or_insert(index);
            }
        }
        for alias in &self.aliases {
            if !alias.name.is_empty() {
                self.lookup.entry(alias.name.clone()).or_insert(alias.index);
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GameBase {
    pub unique_code: i64,
    pub version: i64,
    pub version_defined: bool,
    pub compatible_min_version: i64,
    pub default_character: i64,
    pub no_item: i64,
    pub title: String,
    pub author: String,
    pub year: String,
    pub info: String,
    pub window_title: Option<String>,
    pub required_emuera_version: String,
    pub update_url: String,
    pub version_name: String,
}

impl Default for GameBase {
    fn default() -> Self {
        Self {
            unique_code: 0,
            version: 0,
            version_defined: false,
            compatible_min_version: -1,
            default_character: -1,
            no_item: 0,
            title: String::new(),
            author: String::new(),
            year: String::new(),
            info: String::new(),
            window_title: None,
            required_emuera_version: "0.000.0.0".into(),
            update_url: String::new(),
            version_name: String::new(),
        }
    }
}

impl GameBase {
    /// Format GAMEBASE's integer version exactly like Emuera's `ScriptVersionText`.
    #[must_use]
    pub fn script_version_text(&self) -> String {
        let fraction = self.version.rem_euclid(1000);
        if fraction % 10 != 0 {
            format!("{}.{fraction:03}", self.version / 1000)
        } else {
            format!("{}.{:02}", self.version / 1000, fraction / 10)
        }
    }

    #[must_use]
    pub fn unique_code_matches(&self, saved_code: i64) -> bool {
        self.save_compatibility().unique_code_matches(saved_code)
    }

    #[must_use]
    pub fn version_matches(&self, saved_version: i64) -> bool {
        self.save_compatibility().version_matches(saved_version)
    }

    fn save_compatibility(&self) -> SaveCompatibility {
        SaveCompatibility {
            unique_code: self.unique_code,
            version: self.version,
            version_defined: self.version_defined,
            compatible_min_version: self.compatible_min_version,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReplaceSettings {
    pub money_label: String,
    pub money_first: bool,
    pub load_label: String,
    pub max_shop_item: i32,
    pub draw_line_string: String,
    pub bar_char_1: char,
    pub bar_char_2: char,
    pub title_menu_string_0: String,
    pub title_menu_string_1: String,
    pub com_able_default: i32,
    pub stain_default: Vec<i64>,
    pub timeup_label: String,
    pub exp_lv_default: Vec<i64>,
    pub palam_lv_default: Vec<i64>,
    pub pband_default: i64,
    pub relation_default: i64,
}

impl Default for ReplaceSettings {
    fn default() -> Self {
        Self {
            money_label: "$".into(),
            money_first: true,
            load_label: "Now Loading...".into(),
            max_shop_item: 100,
            draw_line_string: "-".into(),
            bar_char_1: '*',
            bar_char_2: '.',
            title_menu_string_0: "最初からはじめる".into(),
            title_menu_string_1: "ロードしてはじめる".into(),
            com_able_default: 1,
            stain_default: vec![0, 0, 2, 1, 8],
            timeup_label: "時間切れ".into(),
            exp_lv_default: vec![0, 1, 4, 20, 50, 200],
            palam_lv_default: vec![
                0, 100, 500, 3000, 10_000, 30_000, 60_000, 100_000, 150_000, 250_000,
            ],
            pband_default: 4,
            relation_default: 0,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CharacterTemplate {
    pub no: i64,
    pub csv_no: i64,
    pub name: String,
    pub call_name: String,
    pub nick_name: String,
    pub master_name: String,
    pub is_sp_character: bool,
    pub max_base: BTreeMap<usize, i64>,
    pub mark: BTreeMap<usize, i64>,
    pub exp: BTreeMap<usize, i64>,
    pub abl: BTreeMap<usize, i64>,
    pub talent: BTreeMap<usize, i64>,
    pub relation: BTreeMap<usize, i64>,
    pub cflag: BTreeMap<usize, i64>,
    pub equip: BTreeMap<usize, i64>,
    pub juel: BTreeMap<usize, i64>,
    pub cstr: BTreeMap<usize, String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExtensionData {
    pub global_maps: BTreeSet<String>,
    pub save_maps: BTreeSet<String>,
    pub static_maps: BTreeSet<String>,
    pub global_xmls: BTreeSet<String>,
    pub save_xmls: BTreeSet<String>,
    pub static_xmls: BTreeSet<String>,
    pub global_data_tables: BTreeSet<String>,
    pub save_data_tables: BTreeSet<String>,
    pub static_data_tables: BTreeSet<String>,
}

/// Legacy multibyte encoding selected by Emuera's `useLanguage` option.
///
/// Source files remain UTF-8. This value is retained only for script-visible
/// operations that explicitly expose the selected ANSI code page, such as legacy
/// string-length evaluation. FORM padding uses portable Unicode display columns.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyEncoding {
    #[default]
    Japanese,
    Korean,
    ChineseHans,
    ChineseHant,
}

impl LegacyEncoding {
    /// Return the byte width exposed by Emuera's selected legacy ANSI encoding.
    #[must_use]
    pub fn encoded_len(self, value: &str) -> usize {
        if value.is_ascii() {
            return value.len();
        }
        value
            .chars()
            .map(|character| self.encoded_char_len(character))
            .sum()
    }

    /// Return the encoded width of one Unicode scalar, using Emuera's one-byte fallback.
    #[must_use]
    pub fn encoded_char_len(self, character: char) -> usize {
        if character.is_ascii() {
            return 1;
        }
        let encoding = match self {
            Self::Japanese => encoding_rs::SHIFT_JIS,
            Self::Korean => encoding_rs::EUC_KR,
            Self::ChineseHans => encoding_rs::GBK,
            Self::ChineseHant => encoding_rs::BIG5,
        };
        let mut utf8 = [0; 4];
        let (bytes, _, had_errors) = encoding.encode(character.encode_utf8(&mut utf8));
        if had_errors { 1 } else { bytes.len() }
    }
}

/// Exact raw character-name reverse indexes, built before CALLNAME fallback.
/// Missing name fields are absent; explicitly empty fields retain the empty key.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CharacterNameLookup {
    pub names: BTreeMap<String, i64>,
    pub call_names: BTreeMap<String, i64>,
    pub nick_names: BTreeMap<String, i64>,
    pub master_names: BTreeMap<String, i64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectStaticData {
    pub legacy_encoding: LegacyEncoding,
    pub game_base: GameBase,
    pub name_tables: BTreeMap<NameTableKind, NameTable>,
    pub item_prices: Vec<i64>,
    pub characters: Vec<CharacterTemplate>,
    pub character_name_lookup: CharacterNameLookup,
    pub relation_lookup: BTreeMap<String, i64>,
    pub extensions: ExtensionData,
    /// Keys include the `[[...]]` delimiters because that is the exact lookup form used
    /// by the reference lexer.
    pub rename: BTreeMap<String, String>,
    pub replace: ReplaceSettings,
    pub deferred_indices: DeferredIndexCatalog,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectData {
    pub format_version: u32,
    pub schema: ProjectSchema,
    pub static_data: ProjectStaticData,
}

impl ProjectData {
    #[must_use]
    pub fn new_game_seed(&self) -> NewGameSeed {
        let mut characters = vec![CharacterSelection::CsvNumber(0)];
        if self.static_data.game_base.default_character > 0 {
            characters.push(CharacterSelection::CsvNumber(
                self.static_data.game_base.default_character,
            ));
        }
        NewGameSeed {
            defaults: RuntimeDefaults::from_project(self),
            initial_characters: characters,
        }
    }

    #[must_use]
    pub fn save_load_context(&self) -> SaveLoadContext {
        SaveLoadContext {
            defaults_before_overlay: RuntimeDefaults::from_project(self),
            clear_characters_before_overlay: true,
            copy_and_truncate_arrays: true,
            compatibility: self.static_data.game_base.save_compatibility(),
        }
    }
}
