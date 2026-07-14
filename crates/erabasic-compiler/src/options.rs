use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptimizationLevel {
    None,
    #[default]
    Basic,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompilerOptions {
    pub optimization: OptimizationLevel,
    /// Execution-only setting. It is intentionally excluded from artifact identity.
    pub jobs: Option<usize>,
}

impl Default for CompilerOptions {
    fn default() -> Self {
        Self {
            optimization: OptimizationLevel::Basic,
            jobs: None,
        }
    }
}
