use super::*;

#[path = "arrays/basic.rs"]
mod basic;
#[path = "arrays/bit_calls.rs"]
mod bit_calls;
#[path = "arrays/dynamic_variables.rs"]
mod dynamic_variables;
#[path = "arrays/lifecycle.rs"]
mod lifecycle;
#[path = "arrays/matching.rs"]
mod matching;
use bit_calls::bit_options;
use matching::{match_options, match_result, run_match_source};
