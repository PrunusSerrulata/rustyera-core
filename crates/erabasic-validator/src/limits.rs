use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValidationLimits {
    pub maximum_functions: usize,
    pub maximum_globals: usize,
    pub maximum_instructions_per_function: usize,
    pub maximum_stack: u32,
    pub maximum_imports_per_function: usize,
    pub maximum_source_map_entries: usize,
}

impl Default for ValidationLimits {
    fn default() -> Self {
        Self {
            maximum_functions: 1_000_000,
            maximum_globals: 1_000_000,
            maximum_instructions_per_function: 10_000_000,
            maximum_stack: 1_000_000,
            maximum_imports_per_function: 1_000_000,
            maximum_source_map_entries: 100_000_000,
        }
    }
}
