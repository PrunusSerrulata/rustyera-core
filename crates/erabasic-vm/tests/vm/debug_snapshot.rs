use super::*;

fn corrupt_arithmetic_warning_site(
    site: &mut serde_json::Value,
    corruption: &str,
    instruction_count: usize,
) {
    match corruption {
        "generation" => site[0] = serde_json::json!(999),
        "function" => {
            site[1] = serde_json::to_value(SymbolKey::derive(
                "test.snapshot",
                b"missing-arithmetic-warning-function",
            ))
            .unwrap();
        }
        "instruction" => site[2] = serde_json::json!(instruction_count),
        "tag" => site[3] = serde_json::json!(3),
        _ => unreachable!(),
    }
}

#[path = "debug_snapshot/dynamic_calls.rs"]
mod dynamic_calls;
#[path = "debug_snapshot/host_snapshot.rs"]
mod host_snapshot;
#[path = "debug_snapshot/native_variables.rs"]
mod native_variables;
#[path = "debug_snapshot/text_compatibility.rs"]
mod text_compatibility;
