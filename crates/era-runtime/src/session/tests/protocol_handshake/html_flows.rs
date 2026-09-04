#[test]
fn snake_html_pixel_columns_preserve_alignment_width_and_normalized_markup() {
    let source = "@SYSTEM_TITLE\nHTML_PRINTC \"\", 40\nHTML_PRINTLC \"\"\nHTML_PRINTC \"<b>R</b>\", 40\nHTML_PRINTLC \"<i>L</i>\", 40\nHTML_PRINTC \"D\"\nHTML_PRINTLC \"wide\", 999\nPRINTL\nWAIT\nRETURN\n";
    let mut session = prepare_html_execution_with_profile(
        source,
        None,
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    let (_, messages) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:#?}");
    let project = session.project_snapshot.as_ref().unwrap();
    let default_pixel_width = project.print_c_length.saturating_mul(project.font_size) / 2;
    let snapshot = session.presentation.snapshot();
    let line = snapshot
        .history
        .logical_lines
        .iter()
        .find(|line| {
            line.runs
                .iter()
                .filter(|run| matches!(run, DisplayRun::ColumnCell { .. }))
                .count()
                == 4
        })
        .expect("HTML pixel cells share the current logical line");
    let cells = line
        .runs
        .iter()
        .filter_map(|run| match run {
            DisplayRun::ColumnCell {
                content,
                alignment,
                width,
            } => Some((content, *alignment, *width)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(cells[0].1, era_runtime_protocol::CellAlignment::Right);
    assert_eq!(cells[1].1, era_runtime_protocol::CellAlignment::Left);
    assert_eq!(
        cells.iter().map(|cell| cell.2).collect::<Vec<_>>(),
        [
            era_runtime_protocol::CellWidthIntent::LogicalPixels(40),
            era_runtime_protocol::CellWidthIntent::LogicalPixels(40),
            era_runtime_protocol::CellWidthIntent::LogicalPixels(default_pixel_width),
            era_runtime_protocol::CellWidthIntent::LogicalPixels(999),
        ]
    );
    assert!(matches!(
        cells[0].0.as_slice(),
        [DisplayRun::HtmlDocument { document }]
            if erabasic_html::serialize_document(document) == "<b>R</b>"
    ));
}

#[test]
fn rejected_html_attribute_emits_identity_scoped_diagnostic_without_raw_markup() {
    let source = "@SYSTEM_TITLE\nHTML_PRINT \"<img src='x' arbitrary='secret'>\"\nWAIT\nRETURN\n";
    let identity = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let mut session = prepare_html_execution_with_profile(source, None, identity.clone());
    let (_, messages) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Faulted);
    let diagnostic = messages
        .iter()
        .find_map(|message| match message {
            RuntimeMessage::Diagnostic(diagnostic)
                if diagnostic.code == "runtime.html.profile_attribute_rejected" =>
            {
                Some(diagnostic)
            }
            _ => None,
        })
        .expect("profile-scoped HTML diagnostic");
    assert_eq!(
        diagnostic
            .context
            .as_ref()
            .and_then(|context| context.identity.as_ref()),
        Some(&identity)
    );
    assert!(!diagnostic.message.contains("secret"));
    assert!(session.presentation.snapshot().history.logical_lines.is_empty());
}

#[test]
fn snake_html_image_matrix_is_resolved_to_fixed_protocol_values() {
    let source = "@SYSTEM_TITLE\n#DIM MATRIX, 5, 5\nMATRIX:0:0 = 256\nMATRIX:4:4 = -7\nHTML_PRINT \"<img src='face' cm='MATRIX'>\"\nPRINTL\nWAIT\nRETURN\n";
    let mut session = prepare_html_execution_with_profile(
        source,
        None,
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    let (_, messages) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:#?}");
    let snapshot = session.presentation.snapshot();
    let matrix = snapshot
        .history
        .logical_lines
        .iter()
        .flat_map(|line| &line.runs)
        .find_map(|run| {
            let DisplayRun::HtmlDocument { document } = run else {
                return None;
            };
            let erabasic_html::HtmlNode::Element {
                semantic:
                    erabasic_html::HtmlElementSemantic::Image {
                        color_matrix: Some(erabasic_html::HtmlColorMatrix::Fixed(matrix)),
                        ..
                    },
                ..
            } = document.nodes.first()?
            else {
                return None;
            };
            Some(matrix)
        })
        .expect("resolved image color matrix");
    assert_eq!(matrix[0], 256);
    assert_eq!(matrix[24], -7);
}

#[test]
fn original_profile_keeps_rejecting_snake_only_html_attributes() {
    let source = "@SYSTEM_TITLE\nHTML_PRINT \"<font size='12'>x</font>\"\nWAIT\nRETURN\n";
    let identity = erabasic_compat::CompatibilityIdentity::reference();
    let mut session = prepare_html_execution_with_profile(source, None, identity.clone());
    let (_, messages) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Faulted, "{messages:#?}");
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Diagnostic(diagnostic)
            if diagnostic.code == "runtime.html.profile_attribute_rejected"
                && diagnostic.context.as_ref().and_then(|context| context.identity.as_ref())
                    == Some(&identity)
    )));
}

#[test]
fn original_profile_rejects_snake_html_query_before_provider_observes_it() {
    let source = "@SYSTEM_TITLE\nRESULT = HTML_STRINGLEN(\"<font size='12'>x</font>\", 1)\nWAIT\nRETURN\n";
    let mut session = prepare_html_execution_with_profile(
        source,
        Some(ProtocolVersion::new(2, 0)),
        erabasic_compat::CompatibilityIdentity::reference(),
    );
    let (_, messages) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Faulted, "{messages:#?}");
    assert!(!messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::ServiceRequest(request)
            if request.kind == ServiceKind::PresentationQuery
    )));
}

#[test]
fn snake_html_query_preserves_nested_font_render_intent_in_provider_probe() {
    let source = "@SYSTEM_TITLE\nRESULT = HTML_STRINGLEN(\"<font size='12' valign='middle' render='skia' edging='subpixel' hinting='full'><b>x</b></font>\", 1)\nWAIT\nRETURN\n";
    let mut session = prepare_html_execution_with_profile(
        source,
        Some(ProtocolVersion::new(2, 0)),
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    let (_, messages) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:#?}");
    let payload = messages
        .iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request)
                if request.kind == ServiceKind::PresentationQuery =>
            {
                decode_canonical::<era_runtime_protocol::HtmlMeasureRequestV2>(
                    request.payload.as_slice(),
                )
                .ok()
            }
            _ => None,
        })
        .expect("presentation query probe");
    let font = payload.probes.iter().find_map(|probe| {
        fn find_font(
            nodes: &[erabasic_html::HtmlNode],
        ) -> Option<&erabasic_html::HtmlElementSemantic> {
            nodes.iter().find_map(|node| match node {
                erabasic_html::HtmlNode::Text { .. } => None,
                erabasic_html::HtmlNode::Element {
                    semantic, children, ..
                } => matches!(semantic, erabasic_html::HtmlElementSemantic::Font { .. })
                    .then_some(semantic)
                    .or_else(|| find_font(children)),
            })
        }
        find_font(&probe.document.nodes)
    });
    assert!(matches!(
        font,
        Some(erabasic_html::HtmlElementSemantic::Font {
            size_millipixels: Some(12_000),
            vertical_alignment: Some(erabasic_html::HtmlVerticalAlignment::Middle),
            render_intent: erabasic_html::HtmlTextRenderIntent {
                renderer: Some(erabasic_html::HtmlTextRenderer::Skia),
                edging: Some(erabasic_html::HtmlFontEdging::SubpixelAntiAlias),
                hinting: Some(erabasic_html::HtmlFontHinting::Full),
            },
            ..
        })
    ));
}

#[test]
fn snake_html_query_never_exposes_color_matrix_variable_addresses() {
    let source = "@SYSTEM_TITLE\n#DIM MATRIX, 5, 5\nRESULT = HTML_STRINGLEN(\"<img src='missing' cm='MATRIX'>\", 1)\nWAIT\nRETURN\n";
    let mut session = prepare_html_execution_with_profile(
        source,
        Some(ProtocolVersion::new(2, 0)),
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    let (_, messages) = start_html_execution(&mut session);
    let payload = messages
        .iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request)
                if request.kind == ServiceKind::PresentationQuery =>
            {
                decode_canonical::<era_runtime_protocol::HtmlMeasureRequestV2>(
                    request.payload.as_slice(),
                )
                .ok()
            }
            _ => None,
        })
        .expect("image measurement probe");
    let erabasic_html::HtmlNode::Element {
        attributes,
        semantic: erabasic_html::HtmlElementSemantic::Image { color_matrix, .. },
        ..
    } = &payload.probes[0].document.nodes[0]
    else {
        panic!("image probe root");
    };
    assert!(attributes.iter().all(|attribute| attribute.name != "cm"));
    assert!(color_matrix.is_none());
}

fn html_flag(session: &RuntimeSession, index: u64) -> i64 {
    read_runtime_integer(session.vm.as_ref().unwrap(), "FLAG", &[index], None).unwrap()
}

#[test]
fn html_lazy_evaluation_preserves_flag_width_order_and_nested_flows() {
    let source = "@SYSTEM_TITLE\nFLAG:1 = HTML_STRINGLEN(\"ab\", HTML_FLAG())\nFLAG:2 = HTML_STRINGLINES(\"abc\", HTML_WIDTH())\nFLAG:3 = HTML_STRINGLINES(\"\", HTML_WIDTH())\nWAIT\nRETURN\n@HTML_FLAG\n#FUNCTION\nFLAG:0 += 1\nRETURNF 1\n@HTML_WIDTH\n#FUNCTION\nFLAG:4 += 1\nFLAG:5 += HTML_STRINGLINES(\"x\", 1)\nRETURNF FLAG:4\n";
    let mut session = prepare_html_execution(source, Some(ProtocolVersion::new(2, 0)));
    start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    assert_eq!((html_flag(&session, 0), html_flag(&session, 1)), (1, 18));
    assert_eq!((html_flag(&session, 2), html_flag(&session, 3)), (2, 0));
    assert_eq!((html_flag(&session, 4), html_flag(&session, 5)), (2, 2));
    assert!(session.operations.html_lines.is_empty());
}

#[test]
fn html_malformed_source_fails_before_flag_and_empty_lines_still_requires_v2() {
    let mut session = prepare_html_execution(
        "@SYSTEM_TITLE\nFLAG:1 = HTML_STRINGLEN(\"<not-supported>\", HTML_FLAG())\nWAIT\nRETURN\n@HTML_FLAG\n#FUNCTION\nFLAG:0 += 1\nRETURNF 1\n",
        Some(ProtocolVersion::new(2, 0)),
    );
    let (_, messages) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Faulted, "{messages:#?}");
    assert_eq!(html_flag(&session, 0), 0);
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::ServiceRequest(_)))
    );
    for version in [None, Some(ProtocolVersion::new(1, 0))] {
        let mut session = prepare_html_execution(
            "@SYSTEM_TITLE\nFLAG:1 = HTML_STRINGLINES(\"\", HTML_WIDTH())\nWAIT\nRETURN\n@HTML_WIDTH\n#FUNCTION\nFLAG:0 += 1\nRETURNF 1\n",
            version,
        );
        let (_, messages) = start_html_execution(&mut session);
        assert_eq!(html_flag(&session, 0), 0);
        assert!(messages.iter().any(|message| matches!(message, RuntimeMessage::Fault(RuntimeFault {
            code: FaultCode::UnsupportedRuntimeFeature, context: Some(context), ..
        }) if context.required_capability.as_ref().is_some_and(|required|
            required.operation == HTML_STRING_LINES_OPERATION && required.version == ProtocolVersion::new(2, 0)))));
    }
}

fn html_result(vm: &RuntimeVm, index: u64) -> VmValue {
    vm.read_runtime_state(&[erabasic_vm::VmRuntimeRead {
        variable: runtime_variable_key(vm, "RESULTS").unwrap(),
        indices: vec![index],
        character: None,
    }])
    .unwrap()
    .remove(0)
}

#[test]
fn html_substring_keeps_results_atomic_and_rejects_bad_probe_response() {
    let source = "@SYSTEM_TITLE\nRESULTS:0 = old-head\nRESULTS:1 = old-tail\nSTR:0 '= HTML_SUBSTRING(\"a😀b\", 2)\nWAIT\nRETURN\n";
    let (mut session, first) = start_html_query(
        source,
        HTML_SUBSTRING_OPERATION,
        HTML_SUBSTRING_OPERATION_VERSION,
    );
    let mut payload: era_runtime_protocol::HtmlMeasureRequestV2 =
        decode_canonical(first.payload.as_slice()).unwrap();
    let vm = session.vm.as_ref().unwrap();
    assert_eq!(html_result(vm, 0), VmValue::String("old-head".into()));
    payload.probes[0].id += 1;
    submit(
        &mut session,
        3,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: first.request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&html_test_measurement(&payload, 9000)).unwrap(),
                ),
            },
        }),
    );
    assert_service_failure(&mut session);
    assert_eq!(
        html_result(session.vm.as_ref().unwrap(), 0),
        VmValue::String("old-head".into())
    );
    assert_eq!(
        html_result(session.vm.as_ref().unwrap(), 1),
        VmValue::String("old-tail".into())
    );
    let mut session = prepare_html_execution(source, Some(ProtocolVersion::new(2, 0)));
    start_html_execution(&mut session);
    let vm = session.vm.as_ref().unwrap();
    assert_eq!(html_result(vm, 0), VmValue::String("a😀".into()));
    assert_eq!(html_result(vm, 1), VmValue::String("b".into()));
}

#[test]
fn html_lines_input_snapshot_restores_exact_flow_and_rejects_tampered_owner() {
    for dynamic in [false, true] {
        let source = "@SYSTEM_TITLE\nFLAG:1 = HTML_STRINGLINES(\"abc\", HTML_WIDTH())\nWAIT\nRETURN\n@HTML_WIDTH\n#FUNCTION\nFLAG:0 += 1\nIF FLAG:0 == 1\nINPUT\nENDIF\nRETURNF 2\n";
        let source = if dynamic {
            "@SYSTEM_TITLE\nRESULTS:10 = {HTML_STRINGLINES(\"abc\", HTML_WIDTH())}\nFLAG:1 = TOINT(STRFORM(RESULTS:10))\nWAIT\nRETURN\n@HTML_WIDTH\n#FUNCTION\nFLAG:0 += 1\nIF FLAG:0 == 1\nINPUT\nENDIF\nRETURNF 2\n"
        } else {
            source
        };
        let mut session = prepare_html_execution(source, Some(ProtocolVersion::new(2, 0)));
        let (mut sequence, _) = start_html_execution(&mut session);
        assert_eq!(session.operations.html_lines.len(), 1);
        assert!(session.operations.is_snapshot_stable());
        session
            .export_state(
                100,
                StateExportRequest {
                    kind: StateExportKind::VmSnapshot,
                    snapshot_purpose: SnapshotExportPurpose::Normal,
                },
            )
            .unwrap();
        let bytes = session
            .outbound_transfer
            .take()
            .expect("stable HTML width INPUT snapshot")
            .bytes;
        let bytes = bytes.copy_range(0..bytes.len());
        drain(&mut session);
        let before_epoch = session.epoch;
        let before_wait = session.operations.active_input().unwrap().wait.clone();
        for field in [
            "epoch",
            "frame",
            "generation",
            "depth",
            "count",
            "in_flight",
        ] {
            let mut snapshot = runtime_snapshot::decode(&bytes, usize::MAX).unwrap();
            let mut json = serde_json::to_value(&snapshot.operations).unwrap();
            let flow = json["html_lines"]["entries"]
                .as_object_mut()
                .unwrap()
                .values_mut()
                .next()
                .unwrap();
            flow[field] = if field == "in_flight" {
                serde_json::json!(true)
            } else {
                serde_json::json!(999_999)
            };
            snapshot.operations = serde_json::from_value(json).unwrap();
            session
                .start_vm_snapshot(101, &runtime_snapshot::encode(&snapshot).unwrap())
                .unwrap();
            let messages = drain(&mut session);
            assert!(
                messages
                    .iter()
                    .any(|message| matches!(message, RuntimeMessage::CommandRejected(_))),
                "{field}: {messages:#?}"
            );
            assert_eq!(session.epoch, before_epoch);
            assert_eq!(session.operations.active_input().unwrap().wait, before_wait);
        }
        session.start_vm_snapshot(102, &bytes).unwrap();
        drain(&mut session);
        assert_ne!(session.epoch, before_epoch);
        assert_eq!(session.operations.html_lines.len(), 1);
        session
            .operations
            .html_lines
            .validate_snapshot(session.vm.as_ref().unwrap(), session.epoch.0)
            .unwrap();
        let wait = session.operations.active_input().unwrap().wait.clone();
        submit(
            &mut session,
            sequence,
            RuntimeMessage::Input(FrontendInput {
                wait_id: wait.wait_id,
                token: wait.submission_token,
                monotonic_time_ns: 1,
                intent: InputIntent::CommitText("7".into()),
                message_skip: false,
            }),
        );
        sequence += 1;
        let messages = pump_html_execution(&mut session, &mut sequence);
        assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:#?}");
        assert_eq!((html_flag(&session, 0), html_flag(&session, 1)), (2, 2));
        assert!(session.operations.html_lines.is_empty());
    }
}

#[test]
fn html_lines_reload_blocks_active_flow_and_cancellation_clears_it() {
    for source in [
        "@SYSTEM_TITLE\nFLAG:1 = HTML_STRINGLINES(\"abc\", HTML_WIDTH())\nWAIT\nRETURN\n@HTML_WIDTH\n#FUNCTION\nINPUT\nRETURNF 1\n",
        "@SYSTEM_TITLE\nRESULTS:9 = {HTML_STRINGLINES(\"abc\", HTML_WIDTH())}\nFLAG:1 = TOINT(STRFORM(RESULTS:9))\nWAIT\nRETURN\n@HTML_WIDTH\n#FUNCTION\nINPUT\nRETURNF 1\n",
    ] {
        let mut session = prepare_html_execution(source, Some(ProtocolVersion::new(2, 0)));
        let (sequence, _) = start_html_execution(&mut session);
        let before_epoch = session.epoch;
        session
            .reload_project(
                100,
                &ReloadProject {
                    base_revision: 1,
                    target_revision: 2,
                    changes: Vec::new(),
                },
            )
            .unwrap();
        let messages = drain(&mut session);
        assert!(messages.iter().any(|message| matches!(message,
        RuntimeMessage::CommandRejected(CommandRejected { message, .. }) if message.contains("ActiveBlocks"))));
        assert_eq!(session.epoch, before_epoch);
        assert_eq!(session.operations.html_lines.len(), 1);
        submit(
            &mut session,
            sequence,
            RuntimeMessage::ReturnToTitle(era_runtime_protocol::ReturnToTitleRequest {}),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        assert!(session.operations.html_lines.is_empty());
        assert_ne!(session.epoch, before_epoch);
    }
}

#[test]
fn html_lines_no_progress_fault_clears_flows_without_results_side_effects() {
    let source =
        "@SYSTEM_TITLE\nRESULTS:0 = unchanged\nFLAG:1 = HTML_STRINGLINES(\"x\", 0)\nWAIT\nRETURN\n";
    let mut session = prepare_html_execution(source, Some(ProtocolVersion::new(2, 0)));
    let (_, messages) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Faulted);
    assert!(messages.iter().any(|message| matches!(message,
        RuntimeMessage::Fault(RuntimeFault { message, .. }) if message.contains("NoProgress"))));
    assert_eq!(
        html_result(session.vm.as_ref().unwrap(), 0),
        VmValue::String("unchanged".into())
    );
    assert!(session.operations.html_lines.is_empty());
}

#[test]
fn html_later_measurement_round_is_cancelled_and_old_reply_cannot_resume_it() {
    for source in [
        "@SYSTEM_TITLE\nSTR:0 '= HTML_SUBSTRING(\"abc\", 1)\nWAIT\nRETURN\n",
        "@SYSTEM_TITLE\nRESULTS:9 = %HTML_SUBSTRING(\"abc\", 1)%\nSTR:0 '= STRFORM(RESULTS:9)\nWAIT\nRETURN\n",
    ] {
        let (mut session, first) = start_html_query(
            source,
            HTML_SUBSTRING_OPERATION,
            HTML_SUBSTRING_OPERATION_VERSION,
        );
        let payload = decode_canonical(first.payload.as_slice()).unwrap();
        submit(
            &mut session,
            3,
            RuntimeMessage::ServiceResponse(ServiceResponse {
                request_id: first.request_id,
                result: ServiceResult::Ready {
                    payload: ProtocolBytes::new(
                        encode_canonical(&html_test_measurement(&payload, 9000)).unwrap(),
                    ),
                },
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        let second = drain(&mut session)
            .into_iter()
            .find_map(|message| match message {
                RuntimeMessage::ServiceRequest(request)
                    if request.operation == HTML_SUBSTRING_OPERATION =>
                {
                    Some(request)
                }
                _ => None,
            })
            .expect("second measurement under the original VM wait");
        assert_ne!(first.request_id, second.request_id);
        session.return_to_title(100).unwrap();
        let cancelled = drain(&mut session);
        assert!(
            cancelled.iter().any(|message| matches!(message,
        RuntimeMessage::CancelExternalRequest(request) if request.request_id == second.request_id))
        );
        let after_epoch = session.epoch;
        let payload = decode_canonical(second.payload.as_slice()).unwrap();
        // Even an attacker rebinding the old payload to the current envelope epoch has
        // no pending request to complete. The genuine old-epoch envelope is rejected earlier.
        session
            .complete_service(
                101,
                ServiceResponse {
                    request_id: second.request_id,
                    result: ServiceResult::Ready {
                        payload: ProtocolBytes::new(
                            encode_canonical(&html_test_measurement(&payload, 9000)).unwrap(),
                        ),
                    },
                },
            )
            .unwrap();
        let messages = drain(&mut session);
        assert!(messages.iter().any(|message| matches!(
            message,
            RuntimeMessage::CommandRejected(CommandRejected {
                code: CommandErrorCode::StaleRequest,
                ..
            })
        )));
        assert_eq!(session.epoch, after_epoch);
    }
}

#[test]
fn html_length_flag_is_not_evaluated_while_measurements_are_pending() {
    let source = "@SYSTEM_TITLE\nFLAG:1 = HTML_STRINGLEN(\"ab\", HTML_FLAG())\nWAIT\nRETURN\n@HTML_FLAG\n#FUNCTION\nFLAG:0 += 1\nRETURNF 1\n";
    let (mut session, request) = start_html_query(
        source,
        HTML_STRING_LEN_OPERATION,
        HTML_STRING_LEN_OPERATION_VERSION,
    );
    assert_eq!(html_flag(&session, 0), 0);
    let payload = decode_canonical(request.payload.as_slice()).unwrap();
    submit(
        &mut session,
        3,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: request.request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&html_test_measurement(&payload, 9000)).unwrap(),
                ),
            },
        }),
    );
    let mut sequence = 4;
    pump_html_execution(&mut session, &mut sequence);
    assert_eq!((html_flag(&session, 0), html_flag(&session, 1)), (1, 18));
}

#[test]
fn html_nested_flow_limit_fails_before_allocating_a_seventeenth_flow() {
    let source = "@SYSTEM_TITLE\nFLAG:1 = HTML_STRINGLINES(\"x\", HTML_WIDTH())\nWAIT\nRETURN\n@HTML_WIDTH\n#FUNCTION\nFLAG:0 += 1\nRETURNF HTML_STRINGLINES(\"x\", HTML_WIDTH())\n";
    let mut session = prepare_html_execution(source, Some(ProtocolVersion::new(2, 0)));
    let (_, messages) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Faulted);
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Fault(RuntimeFault {
            code: FaultCode::ResourceLimit,
            ..
        })
    )));
    assert_eq!(html_flag(&session, 0), 16);
    assert!(session.operations.html_lines.is_empty());
}

#[test]
fn html_response_checks_all_three_revisions_and_preserves_backend_categories() {
    let source = "@SYSTEM_TITLE\nFLAG:1 = HTML_STRINGLEN(\"x\", 1)\nWAIT\nRETURN\n";
    for field in ["presentation", "environment", "space"] {
        let (mut session, request) = start_html_query(
            source,
            HTML_STRING_LEN_OPERATION,
            HTML_STRING_LEN_OPERATION_VERSION,
        );
        let payload = decode_canonical(request.payload.as_slice()).unwrap();
        let mut response = html_test_measurement(&payload, 9000);
        match field {
            "presentation" => response.context.presentation_revision += 1,
            "environment" => response.context.environment_revision += 1,
            _ => response.context.projection_space_revision += 1,
        }
        submit(
            &mut session,
            3,
            RuntimeMessage::ServiceResponse(ServiceResponse {
                request_id: request.request_id,
                result: ServiceResult::Ready {
                    payload: ProtocolBytes::new(encode_canonical(&response).unwrap()),
                },
            }),
        );
        assert_service_failure(&mut session);
        assert_eq!(html_flag(&session, 1), 0, "{field}");
    }
    for (category, code) in [
        ("unsupported", FaultCode::UnsupportedRuntimeFeature),
        ("invalid_request", FaultCode::ServiceFailure),
        ("stale_projection", FaultCode::ServiceFailure),
        ("resource_limit", FaultCode::ResourceLimit),
        ("backend_failure", FaultCode::ServiceFailure),
        (
            "frontend.unsupported_service",
            FaultCode::UnsupportedRuntimeFeature,
        ),
        ("frontend.resource_limit", FaultCode::ResourceLimit),
        ("frontend.invalid_request", FaultCode::ServiceFailure),
        ("frontend.stale_projection", FaultCode::ServiceFailure),
        ("frontend.backend_failure", FaultCode::ServiceFailure),
        ("unknown_backend_error", FaultCode::ServiceFailure),
    ] {
        let (mut session, request) = start_html_query(
            source,
            HTML_STRING_LEN_OPERATION,
            HTML_STRING_LEN_OPERATION_VERSION,
        );
        submit(
            &mut session,
            3,
            RuntimeMessage::ServiceResponse(ServiceResponse {
                request_id: request.request_id,
                result: ServiceResult::Error {
                    error: era_runtime_protocol::ServiceError {
                        code: category.into(),
                        message: "fixture failure".into(),
                    },
                },
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        let messages = drain(&mut session);
        assert!(
            messages.iter().any(|message| matches!(message,
            RuntimeMessage::Fault(RuntimeFault { code: actual, message, .. })
            if *actual == code && message.contains(category))),
            "{messages:#?}"
        );
        assert_eq!(html_flag(&session, 1), 0);
    }
}
