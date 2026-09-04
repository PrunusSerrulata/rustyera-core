use super::*;

type IntegerMutationBoundary = (i64, i64, i64, i64, i64);

fn integer_mutation_boundary_cases() -> [(&'static str, [IntegerMutationBoundary; 3]); 4] {
    // Each row contains input, original return/storage, then snake return/storage.
    [
        (
            "++FLAG:0",
            [
                (42, 43, 43, 43, 43),
                (
                    i64::MIN,
                    i64::MIN + 1,
                    i64::MIN + 1,
                    i64::MIN + 1,
                    i64::MIN + 1,
                ),
                (i64::MAX, i64::MIN, i64::MIN, i64::MAX, i64::MAX),
            ],
        ),
        (
            "--FLAG:0",
            [
                (42, 41, 41, 41, 41),
                (i64::MIN, i64::MAX, i64::MAX, i64::MIN, i64::MIN),
                (
                    i64::MAX,
                    i64::MAX - 1,
                    i64::MAX - 1,
                    i64::MAX - 1,
                    i64::MAX - 1,
                ),
            ],
        ),
        (
            "FLAG:0++",
            [
                (42, 42, 43, 42, 43),
                (i64::MIN, i64::MIN, i64::MIN + 1, i64::MIN, i64::MIN + 1),
                (i64::MAX, i64::MAX, i64::MIN, i64::MAX - 1, i64::MAX),
            ],
        ),
        (
            "FLAG:0--",
            [
                (42, 42, 41, 42, 41),
                (i64::MIN, i64::MIN, i64::MAX, i64::MIN + 1, i64::MIN),
                (i64::MAX, i64::MAX, i64::MAX - 1, i64::MAX, i64::MAX - 1),
            ],
        ),
    ]
}

#[path = "language/control_flow.rs"]
mod control_flow;
#[path = "language/expressions_and_calls.rs"]
mod expressions_and_calls;
#[path = "language/text_and_arrays.rs"]
mod text_and_arrays;

pub(super) use text_and_arrays::CHARACTER_SHADOW_SOURCE;
