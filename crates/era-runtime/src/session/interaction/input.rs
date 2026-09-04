// This is part of the split RuntimeSession interaction implementation.
#[allow(clippy::wildcard_imports)]
use super::super::*;

const fn records_input_undo(kind: WaitKind) -> bool {
    matches!(
        kind,
        WaitKind::IntegerValue
            | WaitKind::StringValue
            | WaitKind::AnyValue
            | WaitKind::IntegerButton
            | WaitKind::StringButton
    )
}

include!("input/completion.rs");
include!("input/system_commands.rs");
include!("input/replay.rs");
include!("input/finish.rs");
include!("input/system_finish.rs");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_z_records_only_reference_scalar_and_button_waits() {
        for kind in [
            WaitKind::IntegerValue,
            WaitKind::StringValue,
            WaitKind::AnyValue,
            WaitKind::IntegerButton,
            WaitKind::StringButton,
        ] {
            assert!(records_input_undo(kind), "{kind:?}");
        }
        for kind in [
            WaitKind::EnterKey,
            WaitKind::AnyKey,
            WaitKind::Void,
            WaitKind::PrimitiveMouseKey,
        ] {
            assert!(!records_input_undo(kind), "{kind:?}");
        }
    }
}
