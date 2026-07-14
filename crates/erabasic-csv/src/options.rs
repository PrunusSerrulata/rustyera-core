use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct CsvLoadOptions {
    pub ignore_case: bool,
    pub use_rename_file: bool,
    pub use_replace_file: bool,
    pub search_subdirectories: bool,
    pub sort_with_filename: bool,
    pub compatible_call_name: bool,
    pub compatible_sp_character: bool,
    pub use_erd: bool,
    pub debug_mode: bool,
    pub allow_full_width_space: bool,
    pub continuation_separator: String,
    pub current_emuera_version: String,
}

impl Default for CsvLoadOptions {
    fn default() -> Self {
        Self {
            ignore_case: true,
            use_rename_file: false,
            use_replace_file: true,
            search_subdirectories: false,
            sort_with_filename: false,
            compatible_call_name: false,
            compatible_sp_character: false,
            use_erd: true,
            debug_mode: false,
            allow_full_width_space: true,
            continuation_separator: " ".into(),
            current_emuera_version: "1.824.0.0".into(),
        }
    }
}
