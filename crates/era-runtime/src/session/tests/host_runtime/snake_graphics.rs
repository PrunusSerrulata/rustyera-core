#[test]
#[allow(clippy::too_many_lines)]
fn snake_graphics_resources_execute_overloads_safe_files_and_polygon_replay() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    let mut client = capabilities();
    client.graphics = true;
    client.services.push(ServiceCapability {
        kind: ServiceKind::Image,
        operation: IMAGE_METADATA_OPERATION.into(),
        versions: VersionRange::exact(IMAGE_METADATA_OPERATION_VERSION),
    });
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "snake-graphics-resource-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: client,
            preferred_locales: vec!["en".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);

    let compatibility = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let resource_digest = *blake3::hash(&[1, 2, 3]).as_bytes();
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility,
            project_revision: 1,
            files: vec![
                profile_configuration_file(
                    erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
                ),
                SubmittedFile {
                    relative_path: "erb/sub/main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\n\
                         RESULT:0 = GCREATE(1, 4, 3)\n\
                         RESULT:1 = SPRITECREATE(\"TWO\", 1)\n\
                         RESULT:2 = SPRITECREATE(\"EIGHT\", 1, 0, 0, 2, 1, -3, 4)\n\
                         RESULT:3 = SPRITECREATE(\"TEN\", 1, 0, 0, 2, 1, -3, 4, -7, -9)\n\
                         RESULT:4 = SPRITECREATE(\"FLIP\", 1, 3, 2, -2, -1)\n\
                         RESULT:5 = SPRITECREATEFROMFILE(\"ROOT\", \"root.png\")\n\
                         RESULT:6 = SPRITECREATEFROMFILE(\"LOCAL\", \"local.png\", 1)\n\
                         RESULT:7 = SPRITECREATEFROMFILE(\"UNSAFE\", \"../root.png\", 1)\n\
                         RESULT:8 = G_POLYGON_POINT_ADD(1, 1, 1)\n\
                         RESULT:9 = G_POLYGON_POINT_ADD(1, 3, 1)\n\
                         RESULT:10 = G_POLYGON_POINT_ADD(1, 2, 2)\n\
                         RESULT:11 = G_POLYGON_DRAW(1)\n\
                         RESULT:12 = G_POLYGON_FILL(1)\n\
                         RESULT:13 = G_POLYGON_POINT_CLEAR(1)\n\
                         WAIT\nRETURN\n"
                            .into(),
                    ),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "root.png".into(),
                    category: FileCategory::Resource,
                    payload: FilePayload::Bytes(ProtocolBytes::new(vec![1, 2, 3])),
                    content_hash: Some(ProtocolBytes::new(resource_digest.to_vec())),
                },
                SubmittedFile {
                    relative_path: "erb/sub/local.png".into(),
                    category: FileCategory::Resource,
                    payload: FilePayload::Bytes(ProtocolBytes::new(vec![1, 2, 3])),
                    content_hash: Some(ProtocolBytes::new(resource_digest.to_vec())),
                },
            ],
        }),
    );

    let mut sequence = 2;
    let mut loaded = false;
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        let messages = drain(&mut session);
        if let Some(report) = messages.iter().find_map(|message| match message {
            RuntimeMessage::ProjectLoadReport(report) => Some(report),
            _ => None,
        }) {
            assert!(report.success, "{:#?}", report.diagnostics);
            loaded = true;
            break;
        }
        let request_ids = messages
            .iter()
            .filter_map(|message| match message {
                RuntimeMessage::ServiceRequest(request)
                    if request.operation == IMAGE_METADATA_OPERATION =>
                {
                    Some(request.request_id)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        if request_ids.is_empty() {
            continue;
        }
        for request_id in request_ids {
            submit(
                &mut session,
                sequence,
                RuntimeMessage::ServiceResponse(ServiceResponse {
                    request_id,
                    result: ServiceResult::Ready {
                        payload: ProtocolBytes::new(
                            encode_canonical(&ImageMetadataResponse {
                                width: 2,
                                height: 1,
                                format: "png".into(),
                                animated: false,
                            })
                            .unwrap(),
                        ),
                    },
                }),
            );
            sequence += 1;
        }
    }
    assert!(loaded, "project load did not finish within the metadata request bound");

    submit(
        &mut session,
        sequence,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..16 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    for index in 0..=13 {
        let expected = i64::from(index != 7);
        assert_eq!(
            read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[index], None).unwrap(),
            expected,
            "RESULT:{index}",
        );
    }

    let replay = session.presentation.snapshot().resources;
    let sprite = |name: &str| replay.sprites.iter().find(|sprite| sprite.name == name).unwrap();
    assert_eq!(sprite("EIGHT").position, [-3, 4]);
    assert_eq!(sprite("TEN").size, [7, 9]);
    assert_eq!(sprite("FLIP").canvas_rectangle.unwrap().width, -2);
    assert_eq!(sprite("ROOT").frames[0].resource_id, "root.png");
    assert_eq!(sprite("LOCAL").frames[0].resource_id, "erb/sub/local.png");
    assert_eq!(
        sprite("ROOT").frames[0].content_digest,
        sprite("LOCAL").frames[0].content_digest,
    );
    let canvas = replay
        .canvases
        .iter()
        .filter(|canvas| canvas.canvas_id == 1)
        .max_by_key(|canvas| canvas.revision)
        .unwrap();
    assert!(
        canvas
            .commands
            .iter()
            .any(|command| matches!(command, CanvasReplayCommand::DrawPolygon)),
        "{:#?}",
        canvas.commands
    );
    assert!(
        canvas
            .commands
            .iter()
            .any(|command| matches!(command, CanvasReplayCommand::FillPolygon)),
        "{:#?}",
        canvas.commands
    );
    assert!(matches!(
        canvas.commands.last(),
        Some(CanvasReplayCommand::PolygonPointClear)
    ));
}
