fn complete_projection_reply(
    session: &mut RuntimeSession,
    request: &ServiceRequest,
    payload: Vec<u8>,
) {
    submit(
        session,
        3,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: request.request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(payload),
            },
        }),
    );
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if matches!(
            session.phase(),
            RuntimePhase::WaitingInput | RuntimePhase::Faulted
        ) {
            break;
        }
    }
}

#[test]
fn pointer_query_flushes_prints_and_returns_each_canonical_value() {
    for (expression, integer, string) in [
        ("MOUSEX()", Some(37), None),
        ("MOUSEY()", Some(-91), None),
        ("MOUSEB()", None, Some("script-value")),
    ] {
        let assignment = if integer.is_some() {
            format!("RESULT = {expression}")
        } else {
            format!("RESULTS '= {expression}")
        };
        let source =
            format!("@SYSTEM_TITLE\nREDRAW 0\nPRINTL before-pointer\n{assignment}\nWAIT\nRETURN\n");
        let (mut session, request, messages) = start_projection_service_with_messages(
            &source,
            ServiceKind::InputState,
            POINTER_STATE_OPERATION,
            POINTER_STATE_OPERATION_VERSION,
        );
        let query: PointerStateRequest = decode_canonical(request.payload.as_slice()).unwrap();
        let service_index = messages.iter().position(|message| matches!(message, RuntimeMessage::ServiceRequest(value) if value.request_id == request.request_id)).unwrap();
        assert!(
            messages[..service_index].iter().any(|message| matches!(
                message,
                RuntimeMessage::PresentationDelta(_) | RuntimeMessage::PresentationSnapshot(_)
            )),
            "{messages:?}"
        );
        assert_eq!(query.presentation_revision, session.presentation.revision());
        complete_projection_reply(
            &mut session,
            &request,
            encode_canonical(&PointerStateResponse {
                x: ProjectionLength(37),
                y: ProjectionLength(-91),
                button_value: "script-value".into(),
                presentation_revision: query.presentation_revision,
                environment_revision: query.environment_revision,
                projection_space_revision: query.projection_space_revision,
            })
            .unwrap(),
        );
        assert_eq!(session.phase(), RuntimePhase::WaitingInput);
        let vm = session.vm.as_ref().unwrap();
        if let Some(value) = integer {
            assert_eq!(
                read_runtime_integer(vm, "RESULT", &[], None).unwrap(),
                value
            );
        }
        if let Some(value) = string {
            assert_eq!(
                vm.vm()
                    .read_variable(runtime_variable_key(vm, "RESULTS").unwrap(), &[0], None)
                    .unwrap(),
                VmValue::String(value.into())
            );
        }
    }
}

#[test]
fn pointer_query_commits_its_captured_projection_after_environment_advances() {
    let (mut session, request, _) = start_projection_service_with_messages(
        "@SYSTEM_TITLE\nRESULT = MOUSEX()\nWAIT\nRETURN\n",
        ServiceKind::InputState,
        POINTER_STATE_OPERATION,
        POINTER_STATE_OPERATION_VERSION,
    );
    let query: PointerStateRequest = decode_canonical(request.payload.as_slice()).unwrap();
    submit_projection_resize(
        &mut session,
        3,
        ProjectionQueryContext {
            presentation_revision: query.presentation_revision,
            environment_revision: query.environment_revision,
            projection_space_revision: query.projection_space_revision,
        },
    );
    submit(
        &mut session,
        4,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: request.request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&PointerStateResponse {
                        x: ProjectionLength(37),
                        y: ProjectionLength(-91),
                        button_value: "script-value".into(),
                        presentation_revision: query.presentation_revision,
                        environment_revision: query.environment_revision,
                        projection_space_revision: query.projection_space_revision,
                    })
                    .unwrap(),
                ),
            },
        }),
    );
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if matches!(
            session.phase(),
            RuntimePhase::WaitingInput | RuntimePhase::Faulted
        ) {
            break;
        }
    }
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
        37
    );
}

#[test]
fn pointer_query_rejects_a_response_for_another_projection() {
    let (mut session, request, _) = start_projection_service_with_messages(
        "@SYSTEM_TITLE\nRESULT = 77\nRESULT = MOUSEX()\nWAIT\nRETURN\n",
        ServiceKind::InputState,
        POINTER_STATE_OPERATION,
        POINTER_STATE_OPERATION_VERSION,
    );
    let query: PointerStateRequest = decode_canonical(request.payload.as_slice()).unwrap();
    complete_projection_reply(
        &mut session,
        &request,
        encode_canonical(&PointerStateResponse {
            x: ProjectionLength(37),
            y: ProjectionLength(-91),
            button_value: "script-value".into(),
            presentation_revision: query.presentation_revision,
            environment_revision: query.environment_revision.saturating_add(1),
            projection_space_revision: query.projection_space_revision,
        })
        .unwrap(),
    );
    assert_eq!(session.phase(), RuntimePhase::Faulted);
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
        77
    );
}

#[test]
fn snake_getliney_resolves_a_display_index_to_revision_bound_stable_geometry() {
    let snake = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let (mut session, request, messages) = start_projection_service_with_profile(
        "@SYSTEM_TITLE\nPRINTL first\nPRINTL second\nRESULT = GETLINEY(0)\nWAIT\nRETURN\n",
        ServiceKind::PresentationQuery,
        GET_LINE_GEOMETRY_OPERATION,
        GET_LINE_GEOMETRY_OPERATION_VERSION,
        snake,
    );
    let query: GetLineGeometryV1Request =
        decode_canonical(request.payload.as_slice()).unwrap();
    assert_eq!(query.line_id, 1);
    assert_eq!(query.context.presentation_revision, session.presentation.revision());
    let request_index = messages
        .iter()
        .position(|message| matches!(
            message,
            RuntimeMessage::ServiceRequest(value) if value.request_id == request.request_id
        ))
        .unwrap();
    assert!(messages[..request_index].iter().any(|message| matches!(
        message,
        RuntimeMessage::PresentationDelta(_) | RuntimeMessage::PresentationSnapshot(_)
    )));

    complete_projection_reply(
        &mut session,
        &request,
        encode_canonical(&GetLineGeometryV1Response {
            context: query.context,
            line_id: query.line_id,
            top: ProjectionLength(100),
            height: ProjectionLength(20),
            viewport_height: ProjectionLength(80),
        })
        .unwrap(),
    );
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
        40
    );
}

#[test]
fn snake_getliney_rejects_stale_geometry_without_committing_the_assignment() {
    let snake = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let (mut session, request, _) = start_projection_service_with_profile(
        "@SYSTEM_TITLE\nPRINTL first\nRESULT = 77\nRESULT = GETLINEY(0)\nWAIT\nRETURN\n",
        ServiceKind::PresentationQuery,
        GET_LINE_GEOMETRY_OPERATION,
        GET_LINE_GEOMETRY_OPERATION_VERSION,
        snake,
    );
    let query: GetLineGeometryV1Request =
        decode_canonical(request.payload.as_slice()).unwrap();
    let mut stale = query.context;
    stale.projection_space_revision = stale.projection_space_revision.saturating_add(1);
    complete_projection_reply(
        &mut session,
        &request,
        encode_canonical(&GetLineGeometryV1Response {
            context: stale,
            line_id: query.line_id,
            top: ProjectionLength(100),
            height: ProjectionLength(20),
            viewport_height: ProjectionLength(80),
        })
        .unwrap(),
    );
    assert_eq!(session.phase(), RuntimePhase::Faulted);
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
        77
    );
}

#[test]
fn canvas_sampling_flushes_new_draws_before_query_and_returns_argb() {
    let (mut session, request, messages) = start_projection_service_with_messages(
        "@SYSTEM_TITLE\nREDRAW 0\nRESULT = GCREATE(1, 2, 2)\nRESULT = GCLEAR(1, 4279312947)\nRESULT = GGETCOLOR(1, 0, 0)\nWAIT\nRETURN\n",
        ServiceKind::Canvas,
        SAMPLE_CANVAS_PIXEL_OPERATION,
        SAMPLE_CANVAS_PIXEL_OPERATION_VERSION,
    );
    let query: CanvasPixelRequest = decode_canonical(request.payload.as_slice()).unwrap();
    let service_index = messages.iter().position(|message| matches!(message, RuntimeMessage::ServiceRequest(value) if value.request_id == request.request_id)).unwrap();
    let resources = messages[..service_index]
        .iter()
        .rev()
        .find_map(|message| match message {
            RuntimeMessage::PresentationSnapshot(snapshot) => Some(&snapshot.resources),
            RuntimeMessage::PresentationDelta(delta) => {
                delta
                    .operations
                    .iter()
                    .find_map(|operation| match operation {
                        PresentationOperation::SetResources { resources } => Some(resources),
                        _ => None,
                    })
            }
            _ => None,
        })
        .expect("current replay must precede the sample request");
    let canvas = resources
        .canvases
        .iter()
        .find(|canvas| canvas.canvas_id == query.canvas_id)
        .expect("new canvas must be present even without a mounted display");
    assert_eq!(canvas.revision, query.canvas_revision);
    assert!(canvas.commands.iter().any(|command| matches!(
        command,
        era_runtime_protocol::CanvasReplayCommand::Clear {
            argb: 0xff11_2233,
            rectangle: None
        }
    )));
    assert_eq!(
        query.context.presentation_revision,
        session.presentation.revision()
    );
    complete_projection_reply(
        &mut session,
        &request,
        encode_canonical(&CanvasPixelResponse {
            context: query.context,
            canvas_revision: query.canvas_revision,
            argb: 0xff11_2233,
        })
        .unwrap(),
    );
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
        0xff11_2233
    );
}

#[test]
fn canvas_sampling_rejects_matching_reply_after_the_current_canvas_changes() {
    for remove in [false, true] {
        let (mut session, request, _) = start_projection_service_with_messages(
            "@SYSTEM_TITLE\nRESULT = GCREATE(1, 2, 2)\nRESULT = GGETCOLOR(1, 0, 0)\nWAIT\nRETURN\n",
            ServiceKind::Canvas,
            SAMPLE_CANVAS_PIXEL_OPERATION,
            SAMPLE_CANVAS_PIXEL_OPERATION_VERSION,
        );
        let query: CanvasPixelRequest = decode_canonical(request.payload.as_slice()).unwrap();
        let graph = &mut session.project_snapshot.as_mut().unwrap().resource_graph;
        if remove {
            assert!(graph.dispose_canvas(1));
        } else {
            assert!(graph.clear_canvas(1, 0, None));
        }
        // Simulate an independent resource-generation change without a projection revision.
        // Matching the outstanding reply alone must not authorize this old raster.
        complete_projection_reply(
            &mut session,
            &request,
            encode_canonical(&CanvasPixelResponse {
                context: query.context,
                canvas_revision: query.canvas_revision,
                argb: 0,
            })
            .unwrap(),
        );
        assert_service_failure(&mut session);
    }
}
