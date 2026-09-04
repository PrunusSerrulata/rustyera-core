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
        document_origin_y: runtime_protocol::LogicalLength(0),
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn protocol_45_scene_and_cell_intents_have_stable_json_cbor_and_cddl() {
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
    let mut following = scene_golden_layer();
    following.scroll_policy = SceneScrollPolicyV1::FollowContent;
    following.document_origin_y = runtime_protocol::LogicalLength(19_000);
    let following = SceneStateV1 {
        revision: 4,
        layers: vec![following],
    };
    assert_eq!(
        serde_json::to_value(&following).unwrap()["layers"][0]["document_origin_y"],
        serde_json::json!(19_000)
    );
    assert_eq!(
        decode_canonical::<SceneStateV1>(&encode_canonical(&following).unwrap()),
        Ok(following)
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
        serde_json::to_value(&matrix).unwrap(),
        serde_json::json!({"type": "fixed", "value": vec![256; 25]})
    );
    assert_eq!(encode_canonical(&matrix).unwrap(), {
        let mut bytes = vec![0x82, 0x01, 0x81, 0x98, 0x19];
        for _ in 0..25 {
            bytes.extend([0x19, 0x01, 0x00]);
        }
        bytes
    });
    let variable = HtmlColorMatrix::Variable {
        name: "MATRIX".into(),
        indices: [1, 2, 3],
    };
    assert_eq!(
        serde_json::to_value(&variable).unwrap(),
        serde_json::json!({
            "type": "variable",
            "value": {"name": "MATRIX", "indices": [1, 2, 3]}
        })
    );
    assert_eq!(
        encode_canonical(&variable).unwrap(),
        vec![
            0x82, 0x00, 0x82, 0x66, b'M', b'A', b'T', b'R', b'I', b'X', 0x83, 0x01, 0x02, 0x03,
        ]
    );

    let schema = include_str!("../../schema/runtime.cddl");
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
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(46, 0));
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
                "interaction":null,"scene_revision":4,"document_origin_y":0
            }}),
            vec![
                0x82, 0x00, 0x81, 0xab, 0x00, 0x01, 0x01, 0x02, 0x02, 0x82, 0x00, 0x82, 0x61, b'R',
                0x03, 0x03, 0x00, 0x04, 0x82, 0x00, 0x80, 0x05, 0xa2, 0x00, 0x00, 0x01, 0x00, 0x06,
                0xa2, 0x00, 0x00, 0x01, 0x00, 0x07, 0x18, 0xff, 0x09, 0x00, 0x0b, 0x04, 0x0c, 0x00,
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
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(46, 0));
    let schema = include_str!("../../schema/runtime.cddl");
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
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(46, 0));
    let schema = include_str!("../../schema/runtime.cddl");
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
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(46, 0));
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
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(46, 0));
    let schema = include_str!("../../schema/runtime.cddl");
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
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(46, 0));
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
    assert!(include_str!("../../schema/runtime.cddl").contains(
        "runtime-limits = { 0: uint, 1: uint, 2: uint, 3: uint, 4: uint, 5: uint, 6: uint }"
    ));
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(46, 0));
}

