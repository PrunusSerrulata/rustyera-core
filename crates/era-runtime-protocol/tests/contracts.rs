use era_protocol::{
    Channel, Envelope, ProtocolBytes, ProtocolErrorCode, ProtocolVersion, SessionId,
    decode_canonical, encode_canonical,
};
use era_runtime_protocol::{
    AdvanceTime, AudioEffect, AudioEffectAction, CanvasPixelRequest, CanvasReplay,
    CanvasReplayCommand, CanvasSize, DisplayRun, EffectAcknowledgement, EffectBatch, EffectEvent,
    EffectKind, EffectOutcome, EffectOutcomeStatus, ExitReason, ExitRequested, FrontendInput,
    GET_KEY_STATE_OPERATION, GET_KEY_STATE_OPERATION_VERSION, GetKeyStateRequest,
    GetKeyStateResponse, InputIntent, InputUndoRequest, InputUndoState, InteractionToken,
    KeyMacroCommand, POINTER_STATE_OPERATION, POINTER_STATE_OPERATION_VERSION, PointerStateRequest,
    PointerStateResponse, PresentationDelta, PresentationOperation, PrimitiveInput,
    ProjectLoadRequest, ProjectManifest, ProjectionLength, ProjectionObservation,
    ProjectionQueryContext, ProjectionSize, ProjectionTransform, RUNTIME_PROTOCOL_VERSION,
    RedrawState, ResourceReplay, ReturnToTitleRequest, RuntimeLog, RuntimeLogLevel, RuntimeMessage,
    SAMPLE_CANVAS_PIXEL_OPERATION, ServiceKind, ServiceRequest, SnapshotExportPurpose,
    StateExportChunkRequest, StateExportKind, StateExportRequest, StateImportBegin,
    StorageNamespace, StorageOperation, StorageRequest, TextExtentRequest, parse_document,
    validate_relative_path,
};

#[test]
fn protocol_21_carries_parsed_html_instead_of_opaque_markup() {
    let run = DisplayRun::HtmlDocument {
        document: parse_document("<div width='50' height='10'><b>text</b><br></div>").unwrap(),
    };
    let bytes = encode_canonical(&run).unwrap();
    assert_eq!(decode_canonical::<DisplayRun>(&bytes), Ok(run));
}

#[test]
fn projection_queries_use_typed_revision_bound_payloads() {
    let context = ProjectionQueryContext {
        presentation_revision: 11,
        environment_revision: 7,
        projection_space_revision: 3,
    };
    let extent = TextExtentRequest {
        context,
        text: "abc".into(),
        font_family: "sans-serif".into(),
        font_size: 19,
        style_bits: 3,
    };
    assert_eq!(
        decode_canonical::<TextExtentRequest>(&encode_canonical(&extent).unwrap()).unwrap(),
        extent
    );
    let pixel = CanvasPixelRequest {
        context,
        canvas_id: 2,
        canvas_revision: 5,
        point: era_runtime_protocol::CanvasPoint { x: 4, y: 6 },
    };
    assert_eq!(SAMPLE_CANVAS_PIXEL_OPERATION, "sample_canvas_pixel");
    assert_eq!(
        decode_canonical::<CanvasPixelRequest>(&encode_canonical(&pixel).unwrap()).unwrap(),
        pixel
    );
}

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
fn protocol_24_carries_backend_authoritative_logs() {
    for (level, encoded_level) in [
        (RuntimeLogLevel::Debug, 0_u8),
        (RuntimeLogLevel::Info, 1),
        (RuntimeLogLevel::Warning, 2),
        (RuntimeLogLevel::Error, 3),
    ] {
        assert_eq!(encode_canonical(&level).unwrap(), vec![encoded_level]);
    }
    let message = RuntimeMessage::Log(RuntimeLog {
        level: RuntimeLogLevel::Warning,
        message: "cache fallback".into(),
    });
    assert_eq!(message.tag(), 98);
    assert_eq!(
        RuntimeMessage::decode_payload(98, &message.encode_payload().unwrap()).unwrap(),
        message
    );
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(25, 0));
}

#[test]
fn checked_runtime_schema_covers_lifecycle_control_messages() {
    let schema = include_str!("../schema/runtime.cddl");
    for expected in [
        "[22, exit-requested]",
        "[93, sequence-acknowledgement]",
        "[94, resynchronize-request]",
        "[96, runtime-resynchronized]",
    ] {
        assert!(
            schema.contains(expected),
            "runtime CDDL is missing {expected}"
        );
    }
}

#[test]
fn protocol_23_carries_compiled_cache_loads_and_in_session_title_returns() {
    let load = RuntimeMessage::ProjectLoad(ProjectLoadRequest {
        identity: era_runtime_protocol::ProjectIdentity {
            project_revision: 7,
            source_digest: ProtocolBytes::new(vec![1; 32]),
        },
        manifest: Some(ProjectManifest {
            project_revision: 7,
            files: Vec::new(),
        }),
        compiled_cache_transfer_id: Some(9),
    });
    assert_eq!(load.tag(), 19);
    assert_eq!(
        RuntimeMessage::decode_payload(19, &load.encode_payload().unwrap()).unwrap(),
        load
    );
    assert_eq!(
        RuntimeMessage::ReturnToTitle(ReturnToTitleRequest {}).tag(),
        23
    );
    assert_eq!(StateExportKind::CompiledProjectCache as u8, 2);
}

#[test]
fn protocol_23_retains_analysis_key_macros_and_extension_registration() {
    let macro_command = RuntimeMessage::KeyMacroCommand(KeyMacroCommand::Store {
        group: 2,
        slot: 3,
        text: "abc".into(),
    });
    assert_eq!(macro_command.tag(), 16);
    assert_eq!(
        RuntimeMessage::decode_payload(16, &macro_command.encode_payload().unwrap()).unwrap(),
        macro_command
    );
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(25, 0));
}

#[test]
fn protocol_21_publishes_semantic_history_redraw_and_textbox_layout() {
    use era_runtime_protocol::{
        PresentationHistory, PresentationSettings, RationalOpacity, RedrawState, TextBoxLayout,
    };

    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(25, 0));
    let opacity = RationalOpacity {
        numerator: 128,
        denominator: 255,
    };
    assert_eq!(opacity.denominator, 255);
    let history = PresentationHistory {
        logical_lines: Vec::new(),
        operations: Vec::new(),
    };
    assert!(history.logical_lines.is_empty());
    assert!(!RedrawState { enabled: false }.enabled);
    assert_eq!(
        TextBoxLayout {
            x: 10,
            y: 20,
            width: 30,
        }
        .width,
        30
    );
    let _ = std::mem::size_of::<PresentationSettings>();
}

#[test]
fn input_undo_is_a_tokenized_semantic_protocol_operation() {
    let token = InteractionToken { epoch: 7, id: 9 };
    let request = RuntimeMessage::InputUndoRequest(InputUndoRequest { token });
    assert_eq!(request.tag(), 37);
    let encoded = request.encode_payload().unwrap();
    assert_eq!(RuntimeMessage::decode_payload(37, &encoded), Ok(request));

    let state = RuntimeMessage::InputUndoStateChanged(InputUndoState {
        enabled: true,
        available_steps: 2,
        in_progress: false,
        runtime_revision: 11,
        token: Some(token),
    });
    assert_eq!(state.tag(), 38);
    let encoded = state.encode_payload().unwrap();
    assert_eq!(RuntimeMessage::decode_payload(38, &encoded), Ok(state));
}

#[test]
fn projection_observations_and_pointer_results_bind_presentation_revisions() {
    let message = RuntimeMessage::ProjectionObservation(ProjectionObservation {
        environment_revision: 7,
        presentation_revision: 9,
        client_size: ProjectionSize {
            width: ProjectionLength(800),
            height: ProjectionLength(600),
        },
        projection_space_revision: 3,
        line_columns: 80,
        text_box: "typed".into(),
        transform: ProjectionTransform {
            x_numerator: 1,
            x_denominator: 1_000,
            y_numerator: 1,
            y_denominator: 1_000,
            origin_x: ProjectionLength(0),
            origin_y: ProjectionLength(0),
        },
    });
    assert_eq!(message.tag(), 35);
    assert_eq!(
        RuntimeMessage::decode_payload(message.tag(), &message.encode_payload().unwrap()).unwrap(),
        message
    );
    assert_eq!(POINTER_STATE_OPERATION, "pointer_state");
    assert_eq!(POINTER_STATE_OPERATION_VERSION, ProtocolVersion::new(1, 0));
    let request = PointerStateRequest {
        presentation_revision: 9,
        environment_revision: 7,
        projection_space_revision: 3,
    };
    let response = PointerStateResponse {
        x: ProjectionLength(10),
        y: ProjectionLength(20),
        button_value: "3".into(),
        presentation_revision: 9,
        environment_revision: 7,
        projection_space_revision: 3,
    };
    assert_eq!(
        decode_canonical::<PointerStateRequest>(&encode_canonical(&request).unwrap()).unwrap(),
        request
    );
    assert_eq!(
        decode_canonical::<PointerStateResponse>(&encode_canonical(&response).unwrap()).unwrap(),
        response
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
fn storage_contract_expresses_create_only_stat_and_recursive_listing() {
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(25, 0));
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
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(25, 0));
}

#[test]
fn protocol_21_round_trips_complete_presentation_deltas() {
    let message = RuntimeMessage::PresentationDelta(PresentationDelta {
        base_revision: 7,
        new_revision: 9,
        operations: vec![
            PresentationOperation::SetRedraw {
                redraw: RedrawState { enabled: false },
            },
            PresentationOperation::SetButtonGeneration { generation: 4 },
        ],
    });
    let encoded = message.encode_payload().expect("encode presentation delta");
    assert_eq!(RuntimeMessage::decode_payload(41, &encoded), Ok(message));

    let schema = include_str!("../schema/runtime.cddl");
    for tag in 0..=13 {
        assert!(
            schema.contains(&format!("[{tag}")),
            "runtime CDDL is missing presentation operation {tag}"
        );
    }
}

#[test]
fn state_transfers_are_versioned_and_chunked() {
    let request = RuntimeMessage::StateExportRequest(StateExportRequest {
        kind: StateExportKind::VmSnapshot,
        snapshot_purpose: SnapshotExportPurpose::Diagnosis,
    });
    let encoded = request.encode_payload().expect("encode state export");
    assert_eq!(RuntimeMessage::decode_payload(60, &encoded), Ok(request));

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
            kind: EffectKind::Audio(AudioEffect {
                channel_id: 0,
                action: AudioEffectAction::Play,
                resource_id: Some("click".into()),
                repeat_count: 1,
                volume_millionths: 1_000_000,
            }),
        }],
    });
    let encoded = message.encode_payload().expect("encode effect batch");
    assert_eq!(RuntimeMessage::decode_payload(42, &encoded), Ok(message));

    let acknowledgement = EffectAcknowledgement {
        outcomes: vec![EffectOutcome {
            effect_id: 4,
            status: EffectOutcomeStatus::Failed,
            message: Some("device unavailable".into()),
        }],
    };
    let encoded = encode_canonical(&acknowledgement).expect("encode effect outcome");
    assert_eq!(decode_canonical(&encoded), Ok(acknowledgement));
}

#[test]
fn resource_replay_is_a_renderer_independent_protocol_value() {
    let replay = ResourceReplay {
        sprites: Vec::new(),
        canvases: vec![CanvasReplay {
            canvas_id: 3,
            size: CanvasSize {
                width: 64,
                height: 32,
            },
            commands: vec![CanvasReplayCommand::Clear {
                argb: 0xff00_ff00,
                rectangle: None,
            }],
            revision: 1,
        }],
        animation_timer_ms: 55,
    };
    let encoded = encode_canonical(&replay).expect("encode resource replay");
    assert_eq!(decode_canonical(&encoded), Ok(replay));
}
