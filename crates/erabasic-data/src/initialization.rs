use serde::{Deserialize, Serialize};

use crate::{NameTableKind, ProjectData};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeDefaults {
    pub preserve_globals: bool,
    pub clear_local_and_static_variables: bool,
    pub item_prices: Vec<i64>,
    pub str_values: Vec<Option<String>>,
    pub palam_levels: Vec<i64>,
    pub exp_levels: Vec<i64>,
    pub assi_0: i64,
    pub target_0: i64,
    pub pband_0: i64,
    pub ejac_0: i64,
    pub no_item_0: i64,
    pub relation_default: i64,
    pub last_load_version: i64,
    pub last_load_no: i64,
    pub last_load_text: String,
}

impl RuntimeDefaults {
    pub(crate) fn from_project(project: &ProjectData) -> Self {
        let static_data = &project.static_data;
        let str_values = static_data
            .name_tables
            .get(&NameTableKind::Str)
            .map_or_else(Vec::new, |table| table.names.clone());
        Self {
            preserve_globals: true,
            clear_local_and_static_variables: true,
            item_prices: static_data.item_prices.clone(),
            str_values,
            palam_levels: static_data.replace.palam_lv_default.clone(),
            exp_levels: static_data.replace.exp_lv_default.clone(),
            assi_0: -1,
            target_0: 1,
            pband_0: static_data.replace.pband_default,
            ejac_0: 10_000,
            // `NOITEM` itself is reset to zero. The GAMEBASE value is exposed through
            // the calculated `GAMEBASE_NOITEM` variable instead.
            no_item_0: 0,
            relation_default: static_data.replace.relation_default,
            last_load_version: -1,
            last_load_no: -1,
            last_load_text: String::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum CharacterSelection {
    CsvNumber(i64),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NewGameSeed {
    pub defaults: RuntimeDefaults,
    pub initial_characters: Vec<CharacterSelection>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SaveCompatibility {
    pub unique_code: i64,
    pub version: i64,
    pub version_defined: bool,
    pub compatible_min_version: i64,
}

impl SaveCompatibility {
    #[must_use]
    pub fn accepts(&self, saved_code: i64, saved_version: i64) -> bool {
        let code_matches = saved_code == 0 || saved_code == self.unique_code;
        let version_matches = (!self.version_defined && saved_version != 1000)
            || self.compatible_min_version <= saved_version
            || self.version == saved_version;
        code_matches && version_matches
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SaveLoadContext {
    pub defaults_before_overlay: RuntimeDefaults,
    pub clear_characters_before_overlay: bool,
    pub copy_and_truncate_arrays: bool,
    pub compatibility: SaveCompatibility,
}
