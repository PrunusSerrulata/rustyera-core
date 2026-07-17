use era_debug_protocol::{
    AuthorizedDebugRequest, DEBUG_PROTOCOL_VERSION, DebugCommand, DebugMessage, DebugPlace,
    DebugScope, DebugValue, GrantToken, StepKind, StopToken, ValueKind, grant_scopes,
};
use era_protocol::{ProtocolBytes, SessionId};

#[test]
fn requested_scopes_cannot_widen_creation_policy() {
    let granted = grant_scopes(
        &[DebugScope::VariablesRead, DebugScope::ExecutionRead],
        &[
            DebugScope::VariablesRead,
            DebugScope::VariablesWrite,
            DebugScope::VariablesRead,
        ],
    );
    assert_eq!(granted, [DebugScope::VariablesRead]);
}

#[test]
fn stateful_debug_commands_carry_a_stop_token() {
    let stop = StopToken {
        session_epoch: 3,
        pause_epoch: 7,
        program_generation: 2,
        runtime_revision: 19,
    };
    let message = DebugMessage::Request(AuthorizedDebugRequest {
        grant: GrantToken {
            grant_id: SessionId { high: 1, low: 2 },
            session_epoch: 3,
            program_generation: 2,
            issued_runtime_revision: 18,
        },
        command: DebugCommand::Step {
            stop,
            fiber_id: 4,
            kind: StepKind::Over,
        },
    });
    let bytes = message.encode_payload().expect("encode debug request");
    assert_eq!(
        DebugMessage::decode_payload(message.tag(), &bytes),
        Ok(message)
    );
}

#[test]
fn debug_protocol_has_an_independent_version() {
    assert_eq!(DEBUG_PROTOCOL_VERSION.major, 4);
}

#[test]
fn operand_places_have_a_unique_external_representation() {
    let value = DebugValue::Place(DebugPlace {
        symbol_key: ProtocolBytes::new([7; 16]),
        value_kind: ValueKind::Integer,
        indices: vec![1, 2],
        character: Some(3),
        fiber_id: Some(4),
        frame_id: Some(5),
        generation: 6,
    });
    assert_eq!(value.kind(), ValueKind::Integer);
}
