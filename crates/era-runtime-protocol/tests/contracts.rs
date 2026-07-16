use era_protocol::{
    Channel, Envelope, ProtocolBytes, ProtocolErrorCode, ProtocolVersion, SessionId,
    decode_canonical, encode_canonical,
};
use era_runtime_protocol::{
    AdvanceTime, EffectBatch, EffectEvent, EffectKind, ExitReason, ExitRequested, FrontendInput,
    GET_KEY_STATE_OPERATION, GET_KEY_STATE_OPERATION_VERSION, GetKeyStateRequest,
    GetKeyStateResponse, InputIntent, InteractionToken, PrimitiveInput, RUNTIME_PROTOCOL_VERSION,
    RuntimeMessage, ServiceKind, ServiceRequest, StateExportChunkRequest, StateExportKind,
    StateImportBegin, StorageNamespace, StorageOperation, StorageRequest, validate_relative_path,
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
fn exit_intent_is_a_persistent_versioned_runtime_message() {
    let exit = ExitRequested {
        reason: ExitReason::Restart,
        force: true,
        runtime_revision: 17,
    };
    let message = RuntimeMessage::ExitRequested(exit);
    let encoded = message.encode_payload().expect("encode exit intent");
    assert_eq!(
        RuntimeMessage::decode_payload(22, &encoded),
        Ok(RuntimeMessage::ExitRequested(exit))
    );
}

#[test]
fn input_carries_interaction_token_and_monotonic_time() {
    let input = FrontendInput {
        wait_id: 7,
        token: InteractionToken { epoch: 2, id: 3 },
        monotonic_time_ns: 99,
        intent: InputIntent::CommitText("2".into()),
        message_skip: false,
    };
    let message = RuntimeMessage::Input(input.clone());
    let encoded = message.encode_payload().expect("encode input");
    assert_eq!(
        RuntimeMessage::decode_payload(message.tag(), &encoded),
        Ok(RuntimeMessage::Input(input))
    );
}

#[test]
fn primitive_input_carries_device_fields_but_not_result_five() {
    let selection = InteractionToken { epoch: 2, id: 8 };
    let intent = InputIntent::Primitive(PrimitiveInput {
        input_type: 1,
        result_1: 10,
        result_2: 20,
        result_3: 1,
        result_4: 3,
        selection_token: Some(selection),
    });
    let bytes = encode_canonical(&intent).expect("encode primitive intent");
    assert_eq!(decode_canonical::<InputIntent>(&bytes), Ok(intent));
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
            precondition: era_runtime_protocol::StoragePrecondition::Revision("old".into()),
        },
        idempotency_key: "session-1/save-10".into(),
        deadline_ns: None,
    };
    let message = RuntimeMessage::StorageRequest(request);
    let encoded = message.encode_payload().expect("encode storage request");
    assert_eq!(
        RuntimeMessage::decode_payload(message.tag(), &encoded),
        Ok(message)
    );
}

#[test]
fn storage_v8_expresses_create_only_stat_and_recursive_listing() {
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(8, 0));
    assert_eq!(
        StorageOperation::Write {
            data: ProtocolBytes::new(vec![1]),
            atomic_replace: true,
            precondition: era_runtime_protocol::StoragePrecondition::Missing,
        },
        StorageOperation::Write {
            data: ProtocolBytes::new(vec![1]),
            atomic_replace: true,
            precondition: era_runtime_protocol::StoragePrecondition::Missing,
        }
    );
    assert_eq!(StorageOperation::Stat, StorageOperation::Stat);
    assert_eq!(
        StorageOperation::List {
            pattern: Some("*.dat".into()),
            recursive: true,
        },
        StorageOperation::List {
            pattern: Some("*.dat".into()),
            recursive: true,
        }
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
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(8, 0));
}

#[test]
fn state_transfers_are_versioned_and_chunked() {
    let begin = RuntimeMessage::StateImportBegin(StateImportBegin {
        kind: StateExportKind::TraditionalSave,
        total_bytes: 4096,
        digest: ProtocolBytes::new([7; 32]),
        artifact_id: None,
    });
    let encoded = begin.encode_payload().expect("encode state import");
    assert_eq!(RuntimeMessage::decode_payload(62, &encoded), Ok(begin));

    let read = RuntimeMessage::StateExportChunkRequest(StateExportChunkRequest {
        transfer_id: 9,
        offset: 1024,
        maximum_bytes: 1024,
    });
    let encoded = read.encode_payload().expect("encode export chunk request");
    assert_eq!(RuntimeMessage::decode_payload(67, &encoded), Ok(read));
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
        deadline_ns: None,
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
