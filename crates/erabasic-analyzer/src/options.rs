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
    pub compatibility: erabasic_compat::CompatibilityIdentity,
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
    /// Promote the snake non-variadic excess-argument warning to a load error.
    pub strict_user_call_arguments: bool,
    pub compatible_call_event: bool,
    pub system_save_in_binary: bool,
    pub use_erd: bool,
    /// Interpret a positive VARSIZE dimension as one-based, matching
    /// `VarsizeDimConfig`; the pinned default uses zero-based dimensions.
    pub varsize_dimension_is_one_based: bool,
    /// Portable RGB value returned by restructurable GETDEFCOLOR calls.
    pub default_foreground_color: i64,
    pub analysis_mode: bool,
    pub debug_mode: bool,
    pub allow_full_width_space: bool,
    pub debug_semicolon: bool,
    pub ignore_triple_symbols: bool,
    /// Replacement inserted between physical lines in `{ ... }` continuations.
    pub continuation_separator: String,
}

impl Default for AnalyzerOptions {
    fn default() -> Self {
        Self {
            compatibility: erabasic_compat::CompatibilityIdentity::default(),
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
            strict_user_call_arguments: false,
            compatible_call_event: false,
            system_save_in_binary: false,
            use_erd: true,
            varsize_dimension_is_one_based: false,
            default_foreground_color: 0x00c0_c0c0,
            analysis_mode: false,
            debug_mode: false,
            allow_full_width_space: true,
            debug_semicolon: false,
            ignore_triple_symbols: false,
            continuation_separator: " ".into(),
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
