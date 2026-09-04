#[test]
#[allow(clippy::too_many_lines)]
fn protocol_46_audio_targets_effects_and_observations_are_exact() {
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(46, 0));
    assert_eq!(AUDIO_OBSERVATION_OPERATION, "audio_observation");
    assert_eq!(
        AUDIO_OBSERVATION_OPERATION_VERSION,
        ProtocolVersion::new(1, 0)
    );
    assert_eq!(AudioChannelV1::sound(9), Some(AudioChannelV1::Sound(9)));
    assert_eq!(AudioChannelV1::sound(10), None);
    assert!(!AudioChannelV1::Sound(10).is_valid());
    let current = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    assert_eq!((current.semantic_version, current.policy_version), (12, 12));
    assert_eq!(
        current.save_codec,
        erabasic_compat::SNAKE_INTEROP_SAVE_CODEC
    );
    assert!(current.services.iter().any(|service| {
        service.name == erabasic_compat::AUDIO_SERVICE_CONTRACT_NAME && service.version == 1
    }));
    let encoded = encode_canonical(&current).unwrap();
    assert_eq!(
        decode_canonical::<erabasic_compat::CompatibilityIdentity>(&encoded),
        Ok(current)
    );

    let request = AudioObservationRequestV1 {
        channel: AudioChannelV1::Sound(3),
        expected_revision: 7,
    };
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        serde_json::json!({"channel":{"type":"sound","channel":3},"expected_revision":7})
    );
    let encoded = encode_canonical(&request).unwrap();
    assert_eq!(
        encoded,
        vec![0xa2, 0x00, 0x82, 0x00, 0x81, 0x03, 0x01, 0x07]
    );
    assert_eq!(decode_canonical(&encoded), Ok(request));

    let response = AudioObservationResponseV1 {
        channel: AudioChannelV1::Bgm,
        revision: 7,
        duration_ms: 2_500,
        position_ms: 1_234,
        state: AudioPlaybackStateV1::Paused,
        volume_millionths: 500_000,
        rate_millionths: 2_500_000,
        preserve_pitch: false,
        frontend_monotonic_time_ns: 999,
    };
    let bgm_request = AudioObservationRequestV1 {
        channel: AudioChannelV1::Bgm,
        expected_revision: 7,
    };
    assert!(response.is_fresh_for(bgm_request));
    assert!(
        !AudioObservationResponseV1 {
            revision: 8,
            ..response
        }
        .is_fresh_for(bgm_request)
    );
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        serde_json::json!({
            "channel":{"type":"bgm"},
            "revision":7,
            "duration_ms":2500,
            "position_ms":1234,
            "state":"paused",
            "volume_millionths":500_000,
            "rate_millionths":2_500_000,
            "preserve_pitch":false,
            "frontend_monotonic_time_ns":999
        })
    );
    let encoded = encode_canonical(&response).unwrap();
    assert_eq!(
        encoded,
        vec![
            0xa9, 0x00, 0x82, 0x01, 0x80, 0x01, 0x07, 0x02, 0x19, 0x09, 0xc4, 0x03, 0x19, 0x04,
            0xd2, 0x04, 0x02, 0x05, 0x1a, 0x00, 0x07, 0xa1, 0x20, 0x06, 0x1a, 0x00, 0x26, 0x25,
            0xa0, 0x07, 0xf4, 0x08, 0x19, 0x03, 0xe7,
        ]
    );
    assert_eq!(decode_canonical(&encoded), Ok(response));

    let effect = AudioEffect {
        channel: AudioChannelV1::Sound(3),
        action: AudioEffectAction::Pause,
        resource_id: Some("tone".into()),
        repeat_count: 1,
        volume_millionths: 500_000,
        revision: 9,
        rate_millionths: 1_500_000,
        preserve_pitch: false,
    };
    assert_eq!(
        serde_json::to_value(&effect).unwrap(),
        serde_json::json!({
            "channel":{"type":"sound","channel":3},
            "action":"pause",
            "resource_id":"tone",
            "repeat_count":1,
            "volume_millionths":500_000,
            "revision":9,
            "rate_millionths":1_500_000,
            "preserve_pitch":false
        })
    );
    let encoded = encode_canonical(&effect).unwrap();
    assert_eq!(
        encoded,
        vec![
            0xa8, 0x00, 0x82, 0x00, 0x81, 0x03, 0x01, 0x03, 0x02, 0x64, b't', b'o', b'n', b'e',
            0x03, 0x01, 0x04, 0x1a, 0x00, 0x07, 0xa1, 0x20, 0x05, 0x09, 0x06, 0x1a, 0x00, 0x16,
            0xe3, 0x60, 0x07, 0xf4,
        ]
    );
    assert_eq!(decode_canonical(&encoded), Ok(effect));

    let state = AudioState {
        channel: AudioChannelV1::Bgm,
        resource_id: "theme".into(),
        repeat_count: -1,
        volume_millionths: 500_000,
        state: AudioPlaybackStateV1::Playing,
        revision: 9,
        rate_millionths: 1_000_000,
        preserve_pitch: true,
    };
    assert_eq!(
        serde_json::to_value(&state).unwrap(),
        serde_json::json!({
            "channel":{"type":"bgm"},
            "resource_id":"theme",
            "repeat_count":-1,
            "volume_millionths":500_000,
            "state":"playing",
            "revision":9,
            "rate_millionths":1_000_000,
            "preserve_pitch":true
        })
    );
    let encoded = encode_canonical(&state).unwrap();
    assert_eq!(
        encoded,
        vec![
            0xa8, 0x00, 0x82, 0x01, 0x80, 0x01, 0x65, b't', b'h', b'e', b'm', b'e', 0x02, 0x20,
            0x03, 0x1a, 0x00, 0x07, 0xa1, 0x20, 0x04, 0x01, 0x05, 0x09, 0x06, 0x1a, 0x00, 0x0f,
            0x42, 0x40, 0x07, 0xf5,
        ]
    );
    assert_eq!(decode_canonical(&encoded), Ok(state));

    let schema = include_str!("../../schema/runtime.cddl");
    for definition in [
        "audio-channel-v1 = [0, [0..9]] / [1, []]",
        "audio-state = {",
        "audio-effect-action = 0..5",
        "audio-observation-request-v1",
        "audio-observation-response-v1",
        "storage-namespace = 0..5",
    ] {
        assert!(schema.contains(definition), "CDDL omitted {definition}");
    }

    for (tag, action) in [
        AudioEffectAction::Play,
        AudioEffectAction::Stop,
        AudioEffectAction::SetVolume,
        AudioEffectAction::Pause,
        AudioEffectAction::Resume,
        AudioEffectAction::SetRate,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            encode_canonical(&action).unwrap(),
            vec![u8::try_from(tag).unwrap()]
        );
        assert_eq!(
            decode_canonical::<AudioEffectAction>(&encode_canonical(&action).unwrap()),
            Ok(action)
        );
    }
}

#[test]
#[allow(clippy::too_many_lines)]
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
                canvas_revision: None,
            }],
            canvas_id: None,
            canvas_rectangle: None,
            revision: 4,
            canvas_revision: None,
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

    let exact_canvas_edge = CanvasReplayCommand::DrawCanvas {
        source_canvas_id: 9,
        source_revision: u64::MAX,
        source: runtime_protocol::CanvasRect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        },
        destination: runtime_protocol::CanvasRect {
            x: 2,
            y: 3,
            width: 4,
            height: 5,
        },
        color_matrix: None,
        mask_canvas_id: Some(10),
        rotation_millidegrees: 0,
        rotation_center: None,
        mask_revision: Some(u64::MAX - 1),
    };
    assert_eq!(
        serde_json::to_value(&exact_canvas_edge).unwrap(),
        serde_json::json!({
            "type": "draw_canvas",
            "source_canvas_id": 9,
            "source_revision": u64::MAX,
            "source": {"x":0,"y":0,"width":1,"height":1},
            "destination": {"x":2,"y":3,"width":4,"height":5},
            "color_matrix": null,
            "mask_canvas_id": 10,
            "rotation_millidegrees": 0,
            "rotation_center": null,
            "mask_revision": u64::MAX - 1
        })
    );
    assert_eq!(
        decode_canonical::<CanvasReplayCommand>(&encode_canonical(&exact_canvas_edge).unwrap()),
        Ok(exact_canvas_edge)
    );

    let frame = SpriteFrameReplay {
        resource_id: String::new(),
        source_rectangle: [0, 0, 1, 1],
        offset: [0, 0],
        delay_ms: 1,
        destination_size: None,
        canvas_id: Some(2),
        content_digest: None,
        canvas_revision: Some(3),
    };
    assert_eq!(
        serde_json::to_value(&frame).unwrap(),
        serde_json::json!({
            "resource_id":"", "source_rectangle":[0,0,1,1], "offset":[0,0],
            "delay_ms":1, "destination_size":null, "canvas_id":2,
            "content_digest":null, "canvas_revision":3
        })
    );
    assert_eq!(
        encode_canonical(&frame).unwrap(),
        vec![
            0xa6, 0x00, 0x60, 0x01, 0x84, 0x00, 0x00, 0x01, 0x01, 0x02, 0x82, 0x00, 0x00, 0x03,
            0x01, 0x05, 0x02, 0x07, 0x03,
        ]
    );
    let sprite = SpriteReplay {
        name: "S".into(),
        size: [1, 1],
        position: [0, 0],
        frames: Vec::new(),
        canvas_id: Some(2),
        canvas_rectangle: None,
        revision: 4,
        canvas_revision: Some(3),
    };
    assert_eq!(
        serde_json::to_value(&sprite).unwrap(),
        serde_json::json!({
            "name":"S", "size":[1,1], "position":[0,0], "frames":[],
            "canvas_id":2, "canvas_rectangle":null, "revision":4, "canvas_revision":3
        })
    );
    assert_eq!(
        encode_canonical(&sprite).unwrap(),
        vec![
            0xa7, 0x00, 0x61, b'S', 0x01, 0x82, 0x01, 0x01, 0x02, 0x82, 0x00, 0x00, 0x03, 0x80,
            0x04, 0x02, 0x06, 0x04, 0x07, 0x03,
        ]
    );
    let draw_sprite = CanvasReplayCommand::DrawSprite {
        name: "S".into(),
        destination: runtime_protocol::CanvasRect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        },
        color_matrix: None,
        resource_revision: 4,
    };
    assert_eq!(
        serde_json::to_value(&draw_sprite).unwrap(),
        serde_json::json!({
            "type":"draw_sprite", "name":"S",
            "destination":{"x":0,"y":0,"width":1,"height":1},
            "color_matrix":null, "resource_revision":4
        })
    );
    assert_eq!(
        encode_canonical(&draw_sprite).unwrap(),
        vec![
            0x82, 0x01, 0x84, 0x61, b'S', 0xa4, 0x00, 0x00, 0x01, 0x00, 0x02, 0x01, 0x03, 0x01,
            0xf6, 0x04,
        ]
    );
    let draw_canvas = CanvasReplayCommand::DrawCanvas {
        source_canvas_id: 1,
        source_revision: 2,
        source: runtime_protocol::CanvasRect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        },
        destination: runtime_protocol::CanvasRect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        },
        color_matrix: None,
        mask_canvas_id: Some(3),
        rotation_millidegrees: 0,
        rotation_center: None,
        mask_revision: Some(4),
    };
    assert_eq!(
        encode_canonical(&draw_canvas).unwrap(),
        vec![
            0x82, 0x0a, 0x89, 0x01, 0x02, 0xa4, 0x00, 0x00, 0x01, 0x00, 0x02, 0x01, 0x03, 0x01,
            0xa4, 0x00, 0x00, 0x01, 0x00, 0x02, 0x01, 0x03, 0x01, 0xf6, 0x03, 0x00, 0xf6, 0x04,
        ]
    );

    let valid = ResourceReplay {
        sprites: vec![sprite],
        canvases: vec![
            CanvasReplay {
                canvas_id: 1,
                size: CanvasSize {
                    width: 1,
                    height: 1,
                },
                commands: vec![draw_canvas, draw_sprite],
                revision: 2,
            },
            CanvasReplay {
                canvas_id: 2,
                size: CanvasSize {
                    width: 1,
                    height: 1,
                },
                commands: vec![],
                revision: 3,
            },
            CanvasReplay {
                canvas_id: 3,
                size: CanvasSize {
                    width: 1,
                    height: 1,
                },
                commands: vec![],
                revision: 4,
            },
        ],
        animation_timer_ms: 0,
    };
    assert_eq!(valid.validate_exact_references(), Ok(()));
    let mut partial = valid;
    partial.sprites[0].canvas_revision = None;
    assert!(partial.validate_exact_references().is_err());
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
    assert_eq!(RUNTIME_PROTOCOL_VERSION, ProtocolVersion::new(46, 0));
    let schema = include_str!("../../schema/runtime.cddl");
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
