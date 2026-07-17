use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningPolicy {
    Ignore,
    Display,
    OncePerFile,
    Later,
}

/// Semantic options whose defaults match the pinned Emuera configuration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct AnalyzerOptions {
    pub ignore_case: bool,
    pub sort_with_filename: bool,
    pub allow_function_overloading: bool,
    pub warn_function_overloading: bool,
    pub display_warning_level: u8,
    pub ignore_uncalled_functions: bool,
    pub function_not_found: WarningPolicy,
    pub function_not_called: WarningPolicy,
    pub compatible_function_argument_auto_convert: bool,
    pub compatible_function_argument_optional: bool,
    pub compatible_call_event: bool,
    pub system_save_in_binary: bool,
    pub use_erd: bool,
    pub analysis_mode: bool,
    pub debug_mode: bool,
    pub allow_full_width_space: bool,
    pub debug_semicolon: bool,
}

impl Default for AnalyzerOptions {
    fn default() -> Self {
        Self {
            ignore_case: true,
            sort_with_filename: false,
            allow_function_overloading: true,
            warn_function_overloading: true,
            display_warning_level: 1,
            ignore_uncalled_functions: true,
            function_not_found: WarningPolicy::Ignore,
            function_not_called: WarningPolicy::Ignore,
            compatible_function_argument_auto_convert: false,
            compatible_function_argument_optional: false,
            compatible_call_event: false,
            system_save_in_binary: false,
            use_erd: true,
            analysis_mode: false,
            debug_mode: false,
            allow_full_width_space: true,
            debug_semicolon: false,
        }
    }
}

impl AnalyzerOptions {
    /// Reference analysis mode checks otherwise unreachable functions too.
    #[must_use]
    pub fn analysis_mode() -> Self {
        Self {
            analysis_mode: true,
            ignore_uncalled_functions: false,
            function_not_found: WarningPolicy::Display,
            function_not_called: WarningPolicy::Display,
            ..Self::default()
        }
    }
}
