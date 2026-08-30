use era_protocol::{
    Channel, Envelope, ProtocolBytes, ProtocolErrorCode, ProtocolVersion, SessionId,
    decode_canonical, encode_canonical,
};
use era_runtime_protocol as runtime_protocol;
use era_runtime_protocol::{
    AdvanceTime, AudioEffect, AudioEffectAction, CanvasPixelRequest, CanvasPoint, CanvasReplay,
    CanvasReplayCommand, CanvasSize, CellWidthIntent, ClientPreferenceLayers, Color,
    ConfigurationApplication, ConfigurationChange, ConfigurationClientProfile,
    ConfigurationUpdateCommitted, ConfigurationUpdateOutcome, ConfigurationValueKind,
    DiagnosticNotification, DisplayRun, EffectAcknowledgement, EffectBatch, EffectEvent,
    EffectKind, EffectOutcome, EffectOutcomeStatus, ExitReason, ExitRequested,
    FinalizeConfigurationUpdate, FrontendInput, FullProjectManifest, GET_KEY_STATE_OPERATION,
    GET_KEY_STATE_OPERATION_VERSION, GET_LINE_GEOMETRY_OPERATION,
    GET_LINE_GEOMETRY_OPERATION_VERSION, GetKeyStateRequest, GetKeyStateResponse,
    GetLineGeometryV1Request, GetLineGeometryV1Response, HtmlColorMatrix, InputIntent,
    InputUndoRequest, InputUndoState, InteractionToken, KeyMacroCommand, POINTER_STATE_OPERATION,
    POINTER_STATE_OPERATION_VERSION, PointerStateRequest, PointerStateResponse,
    PrepareConfigurationUpdate, PresentationDelta, PresentationOperation, PrimitiveInput,
    ProjectConfigurationEntry, ProjectConfigurationSnapshot, ProjectLoadRequest, ProjectManifest,
    ProjectionLength, ProjectionObservation, ProjectionQueryContext, ProjectionSize,
    ProjectionTransform, ProtocolDiagnostic, RUNTIME_PROTOCOL_VERSION, RedrawState, ResourceReplay,
    ReturnToTitleRequest, RuntimeFault, RuntimeLimits, RuntimeLog, RuntimeLogLevel, RuntimeMessage,
    RuntimeVmFault, RuntimeVmFaultCategory, RuntimeVmFaultCode, RuntimeVmFaultDetail,
    SAMPLE_CANVAS_PIXEL_OPERATION, SceneAnchorV1, SceneDeltaV1, SceneInteractionV1, SceneLayerV1,
    SceneOffsetV1, SceneOperationV1, SceneScrollPolicyV1, SceneSizeV1, SceneSourceV1, SceneStateV1,
    SeparatorRole, ServiceKind, ServiceRequest, SnapshotExportPurpose, SpriteFrameReplay,
    SpriteReplay, StateExportCancel, StateExportChunkRequest, StateExportKind, StateExportRequest,
    StateImportBegin, StateImportCommit, StorageNamespace, StorageOperation, StorageRequest,
    TextExtentRequest, TextStyle, parse_document, validate_relative_path,
};

#[test]
fn protocol_21_carries_parsed_html_instead_of_opaque_markup() {
    let run = DisplayRun::HtmlDocument {
        document: parse_document("<div width='50' height='10'><b>text</b><br></div>").unwrap(),
    };
    let bytes = encode_canonical(&run).unwrap();
    assert_eq!(decode_canonical::<DisplayRun>(&bytes), Ok(run));
}

fn scene_golden_layer() -> SceneLayerV1 {
    SceneLayerV1 {
        layer_id: 1,
        sequence: 2,
        source: SceneSourceV1::Resource {
            resource_id: "R".into(),
            resource_revision: 3,
        },
        depth: 0,
        anchor: SceneAnchorV1::Viewport,
        offset: SceneOffsetV1 {
            x: runtime_protocol::LogicalLength(0),
            y: runtime_protocol::LogicalLength(0),
        },
        size: SceneSizeV1 {
            width: runtime_protocol::LogicalLength(0),
            height: runtime_protocol::LogicalLength(0),
        },
        opacity: 255,
        color_matrix: None,
        scroll_policy: SceneScrollPolicyV1::Fixed,
        interaction: None,
        scene_revision: 4,
    }
}

#[test]
fn protocol_42_scene_and_cell_intents_have_stable_json_cbor_and_cddl() {
    let empty_scene = SceneStateV1 {
        revision: 3,
        layers: Vec::new(),
    };
    assert_eq!(
        serde_json::to_value(&empty_scene).unwrap(),
        serde_json::json!({"revision": 3, "layers": []})
    );
    assert_eq!(
        encode_canonical(&empty_scene).unwrap(),
        vec![0xa2, 0, 3, 1, 0x80]
    );

    let layer = scene_golden_layer();
    let mut replayed = empty_scene;
    replayed
        .apply_delta(&SceneDeltaV1 {
            base_revision: 3,
            new_revision: 4,
            operations: vec![SceneOperationV1::UpsertLayer {
                layer: Box::new(layer.clone()),
            }],
        })
        .unwrap();
    assert_eq!(replayed.layers, [layer]);
    assert_eq!(
        decode_canonical::<SceneStateV1>(&encode_canonical(&replayed).unwrap()),
        Ok(replayed.clone())
    );

    let project_width = CellWidthIntent::ProjectColumns(12);
    assert_eq!(
        serde_json::to_value(project_width).unwrap(),
        serde_json::json!({"type": "project_columns", "value": 12})
    );
    assert_eq!(
        encode_canonical(&project_width).unwrap(),
        vec![0x82, 0, 0x81, 12]
    );

    let width = CellWidthIntent::LogicalPixels(40);
    assert_eq!(
        serde_json::to_value(width).unwrap(),
        serde_json::json!({"type": "logical_pixels", "value": 40})
    );
    assert_eq!(
        encode_canonical(&width).unwrap(),
        vec![0x82, 1, 0x81, 0x18, 40]
    );

    let matrix = HtmlColorMatrix::Fixed(Box::new([256; 25]));
    assert_eq!(
        decode_canonical::<HtmlColorMatrix>(&encode_canonical(&matrix).unwrap()),
        Ok(matrix)
    );

    let schema = include_str!("../schema/runtime.cddl");
    for definition in [
        "cell-width-intent = [0, [uint]] / [1, [uint]]",
        "html-color-matrix",
        "5: [* display-run]",
        "7: 0..255",
        "6: uint",
        "scene-state-v1",
        "scene-delta-v1",
    ] {
        assert!(schema.contains(definition));
    }
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(44, 0));
}

#[test]
fn protocol_42_scene_operations_and_delta_have_exact_json_and_cbor() {
    let operations = [
        (
            SceneOperationV1::UpsertLayer {
                layer: Box::new(scene_golden_layer()),
            },
            serde_json::json!({"type":"upsert_layer","layer":{
                "layer_id":1,"sequence":2,
                "source":{"type":"resource","resource_id":"R","resource_revision":3},
                "depth":0,"anchor":{"type":"viewport"},
                "offset":{"x":0,"y":0},"size":{"width":0,"height":0},
                "opacity":255,"color_matrix":null,"scroll_policy":"fixed",
                "interaction":null,"scene_revision":4
            }}),
            vec![
                0x82, 0x00, 0x81, 0xaa, 0x00, 0x01, 0x01, 0x02, 0x02, 0x82, 0x00, 0x82, 0x61, b'R',
                0x03, 0x03, 0x00, 0x04, 0x82, 0x00, 0x80, 0x05, 0xa2, 0x00, 0x00, 0x01, 0x00, 0x06,
                0xa2, 0x00, 0x00, 0x01, 0x00, 0x07, 0x18, 0xff, 0x09, 0x00, 0x0b, 0x04,
            ],
        ),
        (
            SceneOperationV1::RemoveLayer { layer_id: 1 },
            serde_json::json!({"type":"remove_layer","layer_id":1}),
            vec![0x82, 0x01, 0x81, 0x01],
        ),
        (
            SceneOperationV1::ClearDepth { depth: -1 },
            serde_json::json!({"type":"clear_depth","depth":-1}),
            vec![0x82, 0x02, 0x81, 0x20],
        ),
        (
            SceneOperationV1::ClearAnchoredLine { line_id: 2 },
            serde_json::json!({"type":"clear_anchored_line","line_id":2}),
            vec![0x82, 0x03, 0x81, 0x02],
        ),
        (
            SceneOperationV1::ReplaceScene {
                scene: SceneStateV1 {
                    revision: 4,
                    layers: Vec::new(),
                },
            },
            serde_json::json!({"type":"replace_scene","scene":{"revision":4,"layers":[]}}),
            vec![0x82, 0x04, 0x81, 0xa2, 0x00, 0x04, 0x01, 0x80],
        ),
    ];
    for (operation, json, cbor) in operations {
        assert_eq!(serde_json::to_value(&operation).unwrap(), json);
        assert_eq!(encode_canonical(&operation).unwrap(), cbor);
    }

    let delta = SceneDeltaV1 {
        base_revision: 3,
        new_revision: 4,
        operations: vec![SceneOperationV1::RemoveLayer { layer_id: 1 }],
    };
    assert_eq!(
        serde_json::to_value(&delta).unwrap(),
        serde_json::json!({"base_revision":3,"new_revision":4,"operations":[{
            "type":"remove_layer","layer_id":1
        }]})
    );
    assert_eq!(
        encode_canonical(&delta).unwrap(),
        vec![
            0xa3, 0x00, 0x03, 0x01, 0x04, 0x02, 0x81, 0x82, 0x01, 0x81, 0x01
        ]
    );
}

#[test]
fn protocol_26_round_trips_runtime_owned_text_advance() {
    let run = DisplayRun::TextLayout {
        text: "■……■".into(),
        style: TextStyle {
            foreground: Color {
                red: 255,
                green: 255,
                blue: 255,
                alpha: 255,
            },
            background: None,
            bold: false,
            italic: false,
            underline: false,
            strikeout: false,
            font_family: Some("ＭＳ ゴシック".into()),
            font_millipixels: 16_000,
        },
        system_text: None,
        columns: 8,
    };

    let bytes = encode_canonical(&run).expect("encode text layout");
    assert_eq!(decode_canonical::<DisplayRun>(&bytes), Ok(run));
}

#[test]
fn styled_separator_round_trips_canonical_protocol_field() {
    let run = DisplayRun::Separator {
        pattern: "*-".into(),
        role: SeparatorRole::Rule,
        style: TextStyle {
            foreground: Color {
                red: 18,
                green: 52,
                blue: 86,
                alpha: 255,
            },
            background: Some(Color {
                red: 101,
                green: 67,
                blue: 33,
                alpha: 255,
            }),
            bold: false,
            italic: false,
            underline: false,
            strikeout: false,
            font_family: Some("separator-font".into()),
            font_millipixels: 21_000,
        },
    };

    let bytes = encode_canonical(&run).expect("encode styled separator");
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
fn protocol_44_scene_interactions_and_line_geometry_have_stable_contracts() {
    let interaction = SceneInteractionV1 {
        token: InteractionToken { epoch: 7, id: 9 },
        value: runtime_protocol::ProtocolValue::Integer(42),
        enabled: true,
        hover_source: Some(SceneSourceV1::Sprite {
            sprite_name: "H".into(),
            resource_revision: 5,
        }),
        hit_map: Some(SceneSourceV1::Canvas {
            canvas_id: 4,
            resource_revision: 6,
        }),
        title: Some("t".into()),
    };
    assert_eq!(
        serde_json::to_value(&interaction).unwrap(),
        serde_json::json!({
            "token":{"epoch":7,"id":9},
            "value":{"type":"integer","value":42},
            "enabled":true,
            "hover_source":{"type":"sprite","sprite_name":"H","resource_revision":5},
            "hit_map":{"type":"canvas","canvas_id":4,"resource_revision":6},
            "title":"t"
        })
    );
    assert_eq!(
        encode_canonical(&interaction).unwrap(),
        vec![
            0xa6, 0x00, 0xa2, 0x00, 0x07, 0x01, 0x09, 0x01, 0x82, 0x00, 0x81, 0x18, 0x2a, 0x02,
            0xf5, 0x03, 0x82, 0x01, 0x82, 0x61, b'H', 0x05, 0x04, 0x82, 0x02, 0x82, 0x04, 0x06,
            0x05, 0x61, b't',
        ]
    );

    let context = ProjectionQueryContext {
        presentation_revision: 1,
        environment_revision: 2,
        projection_space_revision: 3,
    };
    let request = GetLineGeometryV1Request {
        context,
        line_id: 9,
    };
    assert_eq!(
        encode_canonical(&request).unwrap(),
        vec![
            0xa2, 0x00, 0xa3, 0x00, 0x01, 0x01, 0x02, 0x02, 0x03, 0x01, 0x09
        ]
    );
    let response = GetLineGeometryV1Response {
        context,
        line_id: 9,
        top: ProjectionLength(-4),
        height: ProjectionLength(5),
        viewport_height: ProjectionLength(6),
    };
    assert_eq!(
        encode_canonical(&response).unwrap(),
        vec![
            0xa5, 0x00, 0xa3, 0x00, 0x01, 0x01, 0x02, 0x02, 0x03, 0x01, 0x09, 0x02, 0x23, 0x03,
            0x05, 0x04, 0x06,
        ]
    );
    assert_eq!(
        decode_canonical::<GetLineGeometryV1Response>(&encode_canonical(&response).unwrap()),
        Ok(response)
    );
    assert_eq!(GET_LINE_GEOMETRY_OPERATION, "get_line_geometry_v1");
    assert_eq!(
        GET_LINE_GEOMETRY_OPERATION_VERSION,
        ProtocolVersion::new(1, 0)
    );
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(44, 0));
    let schema = include_str!("../schema/runtime.cddl");
    assert!(schema.contains("get-line-geometry-v1-request"));
    assert!(schema.contains("get-line-geometry-v1-response"));
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
fn protocol_40_round_trips_input_environment_wait_and_ordered_device_contracts() {
    let capability = era_runtime_protocol::EnvironmentCapability {
        name: era_runtime_protocol::INPUT_DEVICE_LATCH_CAPABILITY.into(),
        versions: era_protocol::VersionRange::exact(
            era_runtime_protocol::INPUT_ENVIRONMENT_VERSION,
        ),
    };
    assert_eq!(
        decode_canonical::<era_runtime_protocol::EnvironmentCapability>(
            &encode_canonical(&capability).unwrap(),
        )
        .unwrap(),
        capability
    );

    let wait = era_runtime_protocol::InputWait {
        wait_id: 7,
        kind: era_runtime_protocol::WaitKind::StringValue,
        stability: era_runtime_protocol::WaitStability::Transient,
        one_input: false,
        stop_message_skip: false,
        system_input: false,
        mouse_input: false,
        default_value: Some(era_runtime_protocol::ProtocolValue::String(
            "default".into(),
        )),
        deadline_ns: Some(11),
        display_time: true,
        timeout_message: Some("timeout".into()),
        submission_token: InteractionToken { epoch: 2, id: 3 },
        countdown_remaining_ms: Some(4),
        viewport_policy: era_runtime_protocol::InputViewportPolicy::PreserveUserViewport,
    };
    assert_eq!(
        decode_canonical::<era_runtime_protocol::InputWait>(&encode_canonical(&wait).unwrap())
            .unwrap(),
        wait
    );

    let event = era_runtime_protocol::DeviceStateChanged {
        event_sequence: 9,
        toggle: true,
        repeat: false,
        device: era_runtime_protocol::InputDeviceKind::Keyboard,
        code: 65,
        pressed: true,
        x: 0,
        y: 0,
        monotonic_time_ns: 12,
    };
    let message = RuntimeMessage::DeviceStateChanged(event);
    assert_eq!(
        RuntimeMessage::decode_payload(message.tag(), &message.encode_payload().unwrap()).unwrap(),
        message
    );

    let pump = era_runtime_protocol::DevicePumpRequest {
        epoch: 2,
        after_event_sequence: 9,
    };
    assert_eq!(
        decode_canonical::<era_runtime_protocol::DevicePumpRequest>(
            &encode_canonical(&pump).unwrap(),
        )
        .unwrap(),
        pump
    );
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(44, 0));
    let schema = include_str!("../schema/runtime.cddl");
    for definition in [
        "environment-capability",
        "input-wait",
        "device-state-changed",
        "client-state-changed",
    ] {
        assert!(schema.contains(definition));
    }
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
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(44, 0));
}

#[test]
fn protocol_38_carries_correlated_secondary_vm_faults() {
    let message = RuntimeMessage::Fault(RuntimeFault {
        context: None,
        code: era_runtime_protocol::FaultCode::VmFault,
        message: "original".into(),
        origin: None,
        vm: Some(Box::new(RuntimeVmFault {
            primary: RuntimeVmFaultDetail {
                correlation_id: 41,
                parent_correlation_id: None,
                category: RuntimeVmFaultCategory::ScriptAssertion,
                code: RuntimeVmFaultCode::Trap,
                message: "original".into(),
                origin: None,
            },
            secondary: Some(Box::new(RuntimeVmFaultDetail {
                correlation_id: 42,
                parent_correlation_id: Some(41),
                category: RuntimeVmFaultCategory::ResourceLimit,
                code: RuntimeVmFaultCode::RunawayExecution,
                message: "hook failed".into(),
                origin: None,
            })),
        })),
    });
    assert_eq!(
        RuntimeMessage::decode_payload(message.tag(), &message.encode_payload().unwrap()).unwrap(),
        message
    );
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(44, 0));
    let schema = include_str!("../schema/runtime.cddl");
    assert!(schema.contains("runtime-vm-fault-detail"));
    assert_eq!(
        encode_canonical(&RuntimeVmFaultCategory::ScriptAssertion).unwrap(),
        vec![5]
    );
    assert_eq!(
        encode_canonical(&RuntimeVmFaultCode::RunawayExecution).unwrap(),
        vec![10]
    );
}

#[test]
fn protocol_34_carries_diagnostic_notification_guidance() {
    assert_eq!(
        encode_canonical(&DiagnosticNotification::LogOnly).unwrap(),
        vec![1]
    );
    let message = RuntimeMessage::Diagnostic(ProtocolDiagnostic {
        context: None,
        code: "vm.control_flow.goto_into_structured_block".into(),
        level: RuntimeLogLevel::Warning,
        message: "structured GOTO".into(),
        source: None,
        notification: DiagnosticNotification::LogOnly,
    });
    assert_eq!(
        RuntimeMessage::decode_payload(message.tag(), &message.encode_payload().unwrap()).unwrap(),
        message
    );
    assert_eq!(
        serde_json::to_value(&message).unwrap()["value"]["notification"],
        "log_only"
    );
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(44, 0));
}

#[test]
fn protocol_35_carries_the_encoded_journal_byte_limit_at_map_key_six() {
    let limits = RuntimeLimits {
        maximum_envelope_bytes: 1,
        maximum_payload_bytes: 2,
        maximum_pending_requests: 3,
        maximum_journal_entries: 4,
        maximum_drive_instructions: 5,
        maximum_transfer_bytes: 6,
        maximum_journal_bytes: 7,
    };
    let encoded = encode_canonical(&limits).expect("encode runtime limits");
    assert_eq!(
        encoded,
        vec![0xa7, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7]
    );
    assert_eq!(decode_canonical::<RuntimeLimits>(&encoded), Ok(limits));
    assert!(include_str!("../schema/runtime.cddl").contains(
        "runtime-limits = { 0: uint, 1: uint, 2: uint, 3: uint, 4: uint, 5: uint, 6: uint }"
    ));
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(44, 0));
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
            compatibility: erabasic_compat::CompatibilityIdentity::default(),
            configuration_digest: None,
            project_revision: 7,
            source_digest: ProtocolBytes::new(vec![1; 32]),
        },
        manifest: Some(ProjectManifest {
            compatibility: erabasic_compat::CompatibilityIdentity::default(),
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
    assert_eq!(StateExportKind::FullProjectFile as u8, 3);
}

#[test]
fn protocol_33_round_trips_configuration_and_client_preference_transactions() {
    assert_eq!(
        decode_canonical::<ConfigurationClientProfile>(
            &encode_canonical(&ConfigurationClientProfile::Tui).unwrap()
        ),
        Ok(ConfigurationClientProfile::Tui)
    );
    let digest = ProtocolBytes::new(vec![7; 32]);
    let prepare = RuntimeMessage::PrepareConfigurationUpdate(PrepareConfigurationUpdate {
        project_revision: 9,
        expected_source_digest: digest.clone(),
        changes: vec![ConfigurationChange {
            code: "MaxLog".into(),
            value: "1200".into(),
        }],
    });
    assert_eq!(prepare.tag(), 24);
    assert_eq!(
        RuntimeMessage::decode_payload(24, &prepare.encode_payload().unwrap()),
        Ok(prepare)
    );

    let finalize = RuntimeMessage::FinalizeConfigurationUpdate(FinalizeConfigurationUpdate {
        preparation_message_id: 10,
        outcome: ConfigurationUpdateOutcome::Commit,
    });
    assert_eq!(finalize.tag(), 26);
    assert_eq!(
        RuntimeMessage::decode_payload(26, &finalize.encode_payload().unwrap()),
        Ok(finalize)
    );

    let committed = RuntimeMessage::ConfigurationUpdateCommitted(ConfigurationUpdateCommitted {
        configuration: ProjectConfigurationSnapshot {
            compatibility: erabasic_compat::CompatibilityIdentity::default(),
            project_revision: 9,
            source_digest: digest,
            entries: vec![ProjectConfigurationEntry {
                code: "MaxLog".into(),
                japanese: "履歴ログの行数".into(),
                english: "Maximum log lines".into(),
                value: "1200".into(),
                kind: ConfigurationValueKind::Integer,
                allowed: Vec::new(),
                fixed: false,
                applicability: 3,
                default_value: "1000".into(),
                effective_value: "1200".into(),
                application: ConfigurationApplication::Hot,
                preference_eligible: false,
                client_effective_value: "1200".into(),
            }],
            restart_pending: false,
            generated_source: Some("[meta]\nschema_version = 2\n".into()),
        },
    });
    let json = serde_json::to_string(&committed).unwrap();
    assert_eq!(
        serde_json::from_str::<RuntimeMessage>(&json).unwrap(),
        committed
    );
    assert_eq!(committed.tag(), 27);
    assert_eq!(
        RuntimeMessage::decode_payload(27, &committed.encode_payload().unwrap()),
        Ok(committed)
    );

    let preferences = RuntimeMessage::ApplyClientPreferences(ClientPreferenceLayers {
        project_revision: 9,
        global: vec![ConfigurationChange {
            code: "UseMouse".into(),
            value: "NO".into(),
        }],
        project: vec![ConfigurationChange {
            code: "FontSize".into(),
            value: "22".into(),
        }],
    });
    assert_eq!(preferences.tag(), 28);
    assert_eq!(
        RuntimeMessage::decode_payload(28, &preferences.encode_payload().unwrap()),
        Ok(preferences)
    );
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
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(44, 0));
}

#[test]
fn protocol_21_publishes_semantic_history_redraw_and_textbox_layout() {
    use era_runtime_protocol::{
        PresentationHistory, PresentationSettings, RationalOpacity, RedrawState, TextBoxLayout,
    };

    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(44, 0));
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
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(44, 0));
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
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(44, 0));
    assert_eq!(StateExportKind::InputReplay as u8, 4);
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
    for tag in 0..=14 {
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

    let replay = RuntimeMessage::StateExportRequest(StateExportRequest {
        kind: StateExportKind::InputReplay,
        snapshot_purpose: SnapshotExportPurpose::Normal,
    });
    let encoded = replay.encode_payload().expect("encode input replay export");
    assert_eq!(RuntimeMessage::decode_payload(60, &encoded), Ok(replay));

    let begin = RuntimeMessage::StateImportBegin(StateImportBegin {
        kind: StateExportKind::TraditionalSave,
        total_bytes: 4096,
        digest: Some(ProtocolBytes::new([7; 32])),
        artifact_id: None,
    });
    let encoded = begin.encode_payload().expect("encode state import");
    assert_eq!(RuntimeMessage::decode_payload(62, &encoded), Ok(begin));

    let streamed_begin = RuntimeMessage::StateImportBegin(StateImportBegin {
        kind: StateExportKind::FullProjectManifest,
        total_bytes: 8192,
        digest: None,
        artifact_id: None,
    });
    let encoded = streamed_begin
        .encode_payload()
        .expect("encode streamed import");
    assert_eq!(
        RuntimeMessage::decode_payload(62, &encoded),
        Ok(streamed_begin)
    );
    let commit = RuntimeMessage::StateImportCommit(StateImportCommit {
        transfer_id: 9,
        digest: Some(ProtocolBytes::new([8; 32])),
    });
    let encoded = commit
        .encode_payload()
        .expect("encode streamed import commit");
    assert_eq!(RuntimeMessage::decode_payload(65, &encoded), Ok(commit));

    let read = RuntimeMessage::StateExportChunkRequest(StateExportChunkRequest {
        transfer_id: 9,
        offset: 1024,
        maximum_bytes: 1024,
    });
    let encoded = read.encode_payload().expect("encode export chunk request");
    assert_eq!(RuntimeMessage::decode_payload(67, &encoded), Ok(read));

    let manifest = RuntimeMessage::FullProjectManifest(FullProjectManifest {
        manifest: ProjectManifest {
            compatibility: erabasic_compat::CompatibilityIdentity::default(),
            project_revision: 1,
            files: Vec::new(),
        },
    });
    let encoded = manifest
        .encode_payload()
        .expect("encode full project manifest");
    assert_eq!(RuntimeMessage::decode_payload(70, &encoded), Ok(manifest));

    let cancel = RuntimeMessage::StateExportCancel(StateExportCancel {
        kind: StateExportKind::FullProjectFile,
    });
    let encoded = cancel
        .encode_payload()
        .expect("encode state export cancellation");
    assert_eq!(RuntimeMessage::decode_payload(71, &encoded), Ok(cancel));

    let schema = include_str!("../schema/runtime.cddl");
    assert!(schema.contains("state-export-kind = 0..5"));
    assert!(schema.contains("state-import-begin = { 0: state-export-kind"));
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
    let encoded_image = CanvasReplayCommand::LoadEncodedImage {
        content_digest: ProtocolBytes::new(vec![1, 2]),
        encoded: ProtocolBytes::new(vec![3]),
    };
    assert_eq!(
        serde_json::to_value(&encoded_image).unwrap(),
        serde_json::json!({
            "type": "load_encoded_image",
            "content_digest": [1, 2],
            "encoded": [3]
        })
    );
    assert_eq!(
        encode_canonical(&encoded_image).unwrap(),
        vec![0x82, 0x0b, 0x82, 0x42, 0x01, 0x02, 0x41, 0x03]
    );

    let replay = ResourceReplay {
        sprites: vec![SpriteReplay {
            name: "FILE".into(),
            size: [2, 1],
            position: [0, 0],
            frames: vec![SpriteFrameReplay {
                resource_id: "erb/image.png".into(),
                source_rectangle: [0, 0, 2, 1],
                offset: [0, 0],
                delay_ms: 1_000,
                destination_size: None,
                canvas_id: None,
                content_digest: Some(ProtocolBytes::new(vec![7; 32])),
            }],
            canvas_id: None,
            canvas_rectangle: None,
            revision: 4,
        }],
        canvases: vec![CanvasReplay {
            canvas_id: 3,
            size: CanvasSize {
                width: 64,
                height: 32,
            },
            commands: vec![
                CanvasReplayCommand::Clear {
                    argb: 0xff00_ff00,
                    rectangle: None,
                },
                CanvasReplayCommand::PolygonPointAdd {
                    point: CanvasPoint { x: 1, y: 2 },
                },
                CanvasReplayCommand::DrawPolygon,
                CanvasReplayCommand::PolygonPointClear,
            ],
            revision: 4,
        }],
        animation_timer_ms: 55,
    };
    let encoded = encode_canonical(&replay).expect("encode resource replay");
    assert_eq!(decode_canonical(&encoded), Ok(replay));
}

#[test]
fn protocol_36_resolves_versioned_profile_identity_before_project_load() {
    use era_runtime_protocol::{
        CompatibilityIdentity, CompatibilityProfileId, ProjectCompatibilityResolved,
        ResolveProjectCompatibility,
    };
    let identity = CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake);
    let request = RuntimeMessage::ResolveProjectCompatibility(ResolveProjectCompatibility {
        request_id: 3,
        configuration: None,
    });
    let response = RuntimeMessage::ProjectCompatibilityResolved(ProjectCompatibilityResolved {
        request_id: 3,
        identity: Some(identity.clone()),
        configuration_digest: None,
        diagnostics: Vec::new(),
    });
    for (message, tag) in [(request, 72), (response, 73)] {
        assert_eq!(message.tag(), tag);
        assert_eq!(
            RuntimeMessage::decode_payload(tag, &message.encode_payload().unwrap()).unwrap(),
            message
        );
    }
    let encoded = encode_canonical(&identity).unwrap();
    assert_eq!(
        decode_canonical::<CompatibilityIdentity>(&encoded).unwrap(),
        identity
    );
    let manifest = ProjectManifest {
        project_revision: 1,
        files: Vec::new(),
        compatibility: identity,
    };
    let bytes = encode_canonical(&manifest).unwrap();
    assert_eq!(
        decode_canonical::<ProjectManifest>(&bytes).unwrap(),
        manifest
    );
}

#[test]
fn protocol_37_round_trips_index_inputs_without_reclassifying_them_as_scripts() {
    use era_runtime_protocol::{FileCategory, FilePayload, SubmittedFile};
    let message = RuntimeMessage::ProjectManifest(ProjectManifest {
        project_revision: 7,
        compatibility: erabasic_compat::CompatibilityIdentity::default(),
        files: [
            ("ERB/BUFF.erd", FileCategory::Erd, "10,main\n"),
            ("ERB/BUFF.als", FileCategory::Als, "10,alias\n"),
        ]
        .into_iter()
        .map(|(path, category, text)| SubmittedFile {
            relative_path: path.into(),
            category,
            payload: FilePayload::Utf8(text.into()),
            content_hash: Some(ProtocolBytes::new(vec![1; 32])),
        })
        .collect(),
    });
    assert_eq!(
        RuntimeMessage::decode_payload(message.tag(), &message.encode_payload().unwrap()).unwrap(),
        message
    );
    assert_eq!(encode_canonical(&FileCategory::Als).unwrap(), [6]);
    assert_eq!(encode_canonical(&FileCategory::Erd).unwrap(), [7]);
}

#[test]
fn project_diagnostic_scope_round_trips_without_requiring_a_vm_generation() {
    for generation in [None, Some(2)] {
        let context = era_runtime_protocol::CompatibilityDiagnosticContext {
            identity: None,
            stage: "compat".into(),
            api: Some("user_call".into()),
            required_capability: None,
            artifact: Some(ProtocolBytes::new(vec![7; 32])),
            project_load_id: Some(3),
            runtime_epoch: Some(5),
            generation,
        };
        let bytes = encode_canonical(&context).unwrap();
        assert_eq!(
            decode_canonical::<era_runtime_protocol::CompatibilityDiagnosticContext>(&bytes)
                .unwrap(),
            context
        );
    }
    // Existing four-field CBOR context remains a valid unbound template.
    let bytes = [
        0xa4, 0x00, 0xf6, 0x01, 0x66, b'c', b'o', b'm', b'p', b'a', b't', 0x02, 0xf6, 0x03, 0xf6,
    ];
    let context: era_runtime_protocol::CompatibilityDiagnosticContext =
        decode_canonical(&bytes).unwrap();
    assert_eq!(context.stage, "compat");
    assert!(context.artifact.is_none());
    assert_eq!(
        (
            context.project_load_id,
            context.runtime_epoch,
            context.generation
        ),
        (None, None, None)
    );
}

#[test]
fn protocol_41_carries_safe_sql_v1_without_native_paths_or_handles() {
    use runtime_protocol::{
        SQL_DATABASE_FORMAT_VERSION, SQL_LIMITS_POLICY_VERSION, SQL_OPERATION,
        SQL_OPERATION_VERSION, SQL_SQLITE_VERSION, ServiceKind, SqlConnectionHandleV1,
        SqlDatabaseIdentityV1, SqlDatabaseSourceV1, SqlErrorCodeV1, SqlErrorContextV1, SqlErrorV1,
        SqlLimitsV1, SqlOpenRevisionV1, SqlOperationV1, SqlProviderHandleV1, SqlReaderHandleV1,
        SqlReaderStateV1, SqlRequestV1, SqlResourceSeedV1, SqlResponseV1, SqlResultV1,
        SqlRevisionV1,
    };

    let provider = SqlProviderHandleV1 {
        service_epoch: 7,
        id: 3,
    };
    let connection = SqlConnectionHandleV1 {
        service_epoch: 7,
        id: 11,
    };
    let reader = SqlReaderHandleV1 {
        service_epoch: 7,
        id: 13,
    };
    let revision = SqlRevisionV1 {
        sha256: ProtocolBytes::new(vec![9; 32]),
    };
    let request = SqlRequestV1 {
        provider,
        operation: SqlOperationV1::Open {
            connection,
            logical_name: "qol_data".into(),
            identity: SqlDatabaseIdentityV1 {
                source: SqlDatabaseSourceV1::ResourceSeed(SqlResourceSeedV1 {
                    resource_id: "plugins/qol_data.db".into(),
                    sha256: ProtocolBytes::new(vec![4; 32]),
                }),
                sqlite_version: SQL_SQLITE_VERSION.into(),
                format_version: SQL_DATABASE_FORMAT_VERSION,
            },
            revision: SqlOpenRevisionV1::Exact(revision.clone()),
            limits: SqlLimitsV1::FIXED,
        },
    };
    let encoded = encode_canonical(&request).unwrap();
    assert_eq!(decode_canonical::<SqlRequestV1>(&encoded).unwrap(), request);
    let json = serde_json::to_string(&request).unwrap();
    assert_eq!(
        serde_json::from_str::<SqlRequestV1>(&json).unwrap(),
        request
    );

    let response = SqlResponseV1 {
        provider,
        database: Some(runtime_protocol::SqlDatabaseStateV1 {
            connection,
            connected: true,
            transaction_active: true,
            durable_revision: Some(revision),
        }),
        reader: Some(SqlReaderStateV1 {
            reader,
            status: runtime_protocol::SqlReaderStatusV1::Row,
            rows_read: 17,
        }),
        result: SqlResultV1::Error {
            error: SqlErrorV1 {
                code: SqlErrorCodeV1::Sqlite,
                operation: runtime_protocol::SqlOperationKindV1::ReaderRead,
                context: vec![SqlErrorContextV1 {
                    key: "row_index".into(),
                    value: "17".into(),
                }],
                sqlite_code: Some(19),
                sqlite_message: Some("constraint failed".into()),
            },
        },
    };
    let encoded = encode_canonical(&response).unwrap();
    assert_eq!(
        decode_canonical::<SqlResponseV1>(&encoded).unwrap(),
        response
    );
    let json = serde_json::to_string(&response).unwrap();
    assert_eq!(
        serde_json::from_str::<SqlResponseV1>(&json).unwrap(),
        response
    );

    assert_eq!(encode_canonical(&ServiceKind::Sql).unwrap(), [11]);
    assert_eq!(SQL_OPERATION, "rustyera.sql");
    assert_eq!(SQL_OPERATION_VERSION, ProtocolVersion::new(1, 0));
    assert_eq!(
        SQL_LIMITS_POLICY_VERSION,
        erabasic_compat::SQL_LIMITS_CONTRACT_VERSION
    );
    assert_eq!(SqlLimitsV1::FIXED.maximum_connections, 8);
    assert_eq!(SqlLimitsV1::FIXED.execution_budget_ms, 5_000);
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(44, 0));
    let schema = include_str!("../schema/runtime.cddl");
    assert!(schema.contains("sql-request-v1"));
    assert!(schema.contains("sql-response-v1"));
}

#[test]
#[allow(clippy::too_many_lines)]
fn safe_sql_v1_round_trips_every_operation_and_result_variant() {
    use runtime_protocol::{
        SQL_DATABASE_FORMAT_VERSION, SQL_SQLITE_VERSION, SqlConnectionHandleV1,
        SqlDatabaseIdentityV1, SqlDatabaseSourceV1, SqlErrorCodeV1, SqlErrorV1, SqlExecuteModeV1,
        SqlLimitsV1, SqlMapRowV1, SqlOpenRevisionV1, SqlOperationKindV1, SqlOperationV1,
        SqlReaderHandleV1, SqlReaderValueModeV1, SqlResultV1, SqlValueV1,
    };

    let connection = SqlConnectionHandleV1 {
        service_epoch: 2,
        id: 3,
    };
    let reader = SqlReaderHandleV1 {
        service_epoch: 2,
        id: 5,
    };
    let operations = vec![
        SqlOperationV1::Open {
            connection,
            logical_name: "memory".into(),
            identity: SqlDatabaseIdentityV1 {
                source: SqlDatabaseSourceV1::Memory,
                sqlite_version: SQL_SQLITE_VERSION.into(),
                format_version: SQL_DATABASE_FORMAT_VERSION,
            },
            revision: SqlOpenRevisionV1::Current,
            limits: SqlLimitsV1::FIXED,
        },
        SqlOperationV1::Execute {
            connection,
            mode: SqlExecuteModeV1::Reader,
            sql: "SELECT @0, @1, @2".into(),
            parameters: vec![
                SqlValueV1::Null,
                SqlValueV1::Integer(7),
                SqlValueV1::String("text".into()),
            ],
        },
        SqlOperationV1::ReaderRead { reader },
        SqlOperationV1::ReaderGet {
            reader,
            column: 1,
            mode: SqlReaderValueModeV1::Integer,
        },
        SqlOperationV1::ReaderGet {
            reader,
            column: 2,
            mode: SqlReaderValueModeV1::String,
        },
        SqlOperationV1::ReaderIsNull { reader, column: 2 },
        SqlOperationV1::ReaderClose { reader },
        SqlOperationV1::ImportMapRows {
            connection,
            table: "translations".into(),
            rows: vec![SqlMapRowV1 {
                key: "k".into(),
                value: "<b>v</b>".into(),
            }],
        },
        SqlOperationV1::Disconnect { connection },
    ];
    for operation in operations {
        let encoded = encode_canonical(&operation).unwrap();
        assert_eq!(
            decode_canonical::<SqlOperationV1>(&encoded).unwrap(),
            operation
        );
        let json = serde_json::to_value(&operation).unwrap();
        assert_eq!(
            serde_json::from_value::<SqlOperationV1>(json).unwrap(),
            operation
        );
    }

    let results = vec![
        SqlResultV1::Opened {
            sqlite_version: SQL_SQLITE_VERSION.into(),
            limits: SqlLimitsV1::FIXED,
        },
        SqlResultV1::NonQuery { affected_rows: 2 },
        SqlResultV1::Scalar {
            value: SqlValueV1::Integer(3),
        },
        SqlResultV1::ReaderOpened { reader },
        SqlResultV1::ReaderAdvanced { has_row: true },
        SqlResultV1::ReaderValue {
            value: SqlValueV1::String("v".into()),
        },
        SqlResultV1::ReaderNull { is_null: true },
        SqlResultV1::ReaderClosed,
        SqlResultV1::MapImported { rows: 1 },
        SqlResultV1::Disconnected,
        SqlResultV1::Error {
            error: SqlErrorV1 {
                code: SqlErrorCodeV1::InvalidState,
                operation: SqlOperationKindV1::Execute,
                context: Vec::new(),
                sqlite_code: None,
                sqlite_message: None,
            },
        },
    ];
    for result in results {
        let encoded = encode_canonical(&result).unwrap();
        assert_eq!(decode_canonical::<SqlResultV1>(&encoded).unwrap(), result);
    }
}
