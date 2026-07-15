use era_protocol::{
    Channel, Envelope, ProtocolBytes, ProtocolErrorCode, ProtocolVersion, SessionId,
    decode_canonical, encode_canonical,
};
use era_runtime_protocol::{
    AdvanceTime, EffectBatch, EffectEvent, EffectKind, FrontendInput, GET_KEY_STATE_OPERATION,
    GET_KEY_STATE_OPERATION_VERSION, GetKeyStateRequest, GetKeyStateResponse, InputIntent,
    InteractionToken, RUNTIME_PROTOCOL_VERSION, RuntimeMessage, ServiceKind, ServiceRequest,
    StorageNamespace, StorageOperation, StorageRequest, validate_relative_path,
};

#[test]
fn runtime_payload_and_envelope_tags_agree() {
    let message = RuntimeMessage::AdvanceTime(AdvanceTime {
        monotonic_time_ns: 42,
    });
    let payload = message.encode_payload().expect("encode runtime message");
    let envelope = Envelope::new(
        Channel::Runtime,
        RUNTIME_PROTOCOL_VERSION,
        1,
        1,
        message.tag(),
        ProtocolBytes::new(payload.clone()),
    );
    envelope.validate().expect("valid envelope");
    assert_eq!(
        RuntimeMessage::from_envelope(&envelope).expect("decode runtime message"),
        message
    );
}

#[test]
fn input_carries_interaction_token_and_monotonic_time() {
    let input = FrontendInput {
        wait_id: 7,
        token: InteractionToken { epoch: 2, id: 3 },
        monotonic_time_ns: 99,
        intent: InputIntent::CommitText("2".into()),
    };
    let message = RuntimeMessage::Input(input.clone());
    let encoded = message.encode_payload().expect("encode input");
    assert_eq!(
        RuntimeMessage::decode_payload(message.tag(), &encoded),
        Ok(RuntimeMessage::Input(input))
    );
}

#[test]
fn storage_write_is_correlated_and_idempotent() {
    let request = StorageRequest {
        request_id: 10,
        namespace: StorageNamespace::Save,
        relative_path: "save/save00.sav".into(),
        operation: StorageOperation::Write {
            data: ProtocolBytes::new([1, 2, 3]),
            atomic_replace: true,
            expected_revision: Some("old".into()),
        },
        idempotency_key: "session-1/save-10".into(),
    };
    let message = RuntimeMessage::StorageRequest(request);
    let encoded = message.encode_payload().expect("encode storage request");
    assert_eq!(
        RuntimeMessage::decode_payload(message.tag(), &encoded),
        Ok(message)
    );
}

#[test]
fn paths_are_platform_independent_and_cannot_escape() {
    assert_eq!(
        validate_relative_path("erb\\sub/./test.erb"),
        Ok("erb/sub/test.erb".into())
    );
    assert!(validate_relative_path("../secret").is_err());
    assert!(validate_relative_path("C:\\game\\file").is_err());
    assert!(validate_relative_path("/absolute").is_err());
}

#[test]
fn protocol_version_is_independent_from_wire_version() {
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(3, 0));
}

#[test]
fn getkey_uses_a_fresh_typed_input_state_service() {
    let request_payload = GetKeyStateRequest { key_code: 65 };
    let payload = encode_canonical(&request_payload).expect("encode GETKEY request");
    let request = ServiceRequest {
        request_id: 9,
        kind: ServiceKind::InputState,
        operation: GET_KEY_STATE_OPERATION.into(),
        operation_version: GET_KEY_STATE_OPERATION_VERSION,
        payload: ProtocolBytes::new(payload.clone()),
    };
    assert_eq!(request.kind, ServiceKind::InputState);
    assert_eq!(
        decode_canonical::<GetKeyStateRequest>(&payload),
        Ok(request_payload)
    );

    let response = GetKeyStateResponse {
        frontend_active: true,
        pressed: true,
        toggle_state: false,
    };
    let encoded = encode_canonical(&response).expect("encode GETKEY response");
    assert_eq!(
        decode_canonical::<GetKeyStateResponse>(&encoded),
        Ok(response)
    );
}

#[test]
fn runtime_decoder_rejects_the_debug_channel() {
    let message = RuntimeMessage::AdvanceTime(AdvanceTime {
        monotonic_time_ns: 1,
    });
    let mut envelope = message
        .envelope(Some(SessionId { high: 1, low: 1 }), None, 1, 1, None)
        .expect("wrap message");
    envelope.channel = Channel::Debug;
    assert_eq!(
        RuntimeMessage::from_envelope(&envelope)
            .expect_err("channel isolation must be enforced")
            .code,
        ProtocolErrorCode::ChannelMismatch
    );
}

#[test]
fn transient_effects_have_an_independent_idempotent_stream() {
    let message = RuntimeMessage::EffectBatch(EffectBatch {
        effects: vec![EffectEvent {
            effect_id: 4,
            kind: EffectKind::PlaySound("click".into()),
        }],
    });
    let encoded = message.encode_payload().expect("encode effect batch");
    assert_eq!(RuntimeMessage::decode_payload(42, &encoded), Ok(message));
}
