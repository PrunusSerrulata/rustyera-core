fn html_batch_fail_or_cancel(
    session: &mut RuntimeSession,
    request: &ServiceRequest,
    response: &era_runtime_protocol::HtmlMeasureResponseV2,
    sequence: &mut u64,
    cancel: bool,
) {
    if cancel {
        session.return_to_title(100).unwrap();
        drain(session);
        session
            .complete_service(
                101,
                ServiceResponse {
                    request_id: request.request_id,
                    result: ServiceResult::Ready {
                        payload: ProtocolBytes::new(encode_canonical(response).unwrap()),
                    },
                },
            )
            .unwrap();
        assert!(drain(session).iter().any(|message| matches!(
            message,
            RuntimeMessage::CommandRejected(CommandRejected {
                code: CommandErrorCode::StaleRequest,
                ..
            })
        )));
    } else {
        html_batch_edit_pending(session, request, |value| {
            let remaining = encode_canonical(response).unwrap().len();
            value["budget"]["work"] = (64 * 1024 * 1024 - remaining).into();
        });
        let messages = html_batch_reply(session, request, response, sequence);
        assert_eq!(session.phase(), RuntimePhase::Faulted, "{messages:#?}");
        assert!(messages.iter().any(|message| matches!(
            message,
            RuntimeMessage::Fault(RuntimeFault {
                code: FaultCode::ResourceLimit,
                ..
            })
        )));
        assert_eq!(html_flag(session, 0), 1);
        assert_eq!(html_flag(session, 5), 0);
        assert_eq!(
            html_result(session.vm.as_ref().unwrap(), 0),
            VmValue::String("old-head".into())
        );
        assert_eq!(
            html_result(session.vm.as_ref().unwrap(), 1),
            VmValue::String("old-tail".into())
        );
        assert!(
            !messages
                .iter()
                .any(|message| matches!(message, RuntimeMessage::ServiceRequest(_)))
        );
    }
}

fn html_batch_reply(
    session: &mut RuntimeSession,
    request: &ServiceRequest,
    response: &era_runtime_protocol::HtmlMeasureResponseV2,
    sequence: &mut u64,
) -> Vec<RuntimeMessage> {
    submit(
        session,
        *sequence,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: request.request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(encode_canonical(response).unwrap()),
            },
        }),
    );
    *sequence += 1;
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(session)
}

fn html_batch_edit_pending(
    session: &mut RuntimeSession,
    request: &ServiceRequest,
    edit: impl FnOnce(&mut serde_json::Value),
) {
    let pending = session.operations.take_service(request.request_id).unwrap();
    let PendingService::Host(ExternalCompletion::HtmlQuery { mut continuation }) = pending else {
        panic!("expected HTML continuation");
    };
    // Use the actual positional MessagePack encoder, including absent trailing fields
    // in legacy/single-probe continuations. JSON is used only to inject boundary budgets.
    let bytes = rmp_serde::to_vec(&continuation).unwrap();
    let roundtrip: Box<crate::session::html_query::HtmlQueryContinuation> =
        rmp_serde::from_slice(&bytes).unwrap();
    assert_eq!(roundtrip, continuation);
    let mut value = serde_json::to_value(&continuation).unwrap();
    assert_eq!(
        bytes[0],
        if value.get("batch_transfers").is_some() {
            0x9f
        } else {
            0x9e
        }
    );
    edit(&mut value);
    continuation = serde_json::from_value(value).unwrap();
    session.operations.insert_service(
        request.request_id,
        PendingService::Host(ExternalCompletion::HtmlQuery { continuation }),
    );
}

#[test]
fn html_probe_batch_cut_identity_preserves_reordering_and_rejects_corruption() {
    for defect in 0..4 {
        let (mut session, request) = start_html_query(
            "@SYSTEM_TITLE\nFLAG:1 = 99\nFLAG:1 = HTML_STRINGLEN(\"<b>ab</b><i>cd</i>\", 1)\nWAIT\nRETURN\n",
            HTML_STRING_LEN_OPERATION,
            HTML_STRING_LEN_OPERATION_VERSION,
        );
        let payload = decode_canonical(request.payload.as_slice()).unwrap();
        let mut response = html_test_measurement(&payload, 9000);
        let era_runtime_protocol::HtmlProbeResultV2::TextMeasured { cuts, .. } =
            &mut response.probes[1].result
        else {
            panic!("text");
        };
        match defect {
            0 => cuts.reverse(),
            1 => cuts[0].id = 999,
            2 => cuts[1].id = cuts[0].id,
            _ => {
                cuts.pop();
            }
        }
        let messages = html_batch_reply(&mut session, &request, &response, &mut 3);
        if defect == 0 {
            assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:#?}");
            assert_eq!(html_flag(&session, 1), 36);
        } else {
            assert_eq!(session.phase(), RuntimePhase::Faulted, "{messages:#?}");
            assert_eq!(html_flag(&session, 1), 99);
        }
    }
}

#[test]
fn html_probe_batch_first_error_priority_and_origin_survive_budget_exhaustion() {
    use era_runtime_protocol::HtmlProbeResultV2;
    let backend = |code: &str| HtmlProbeResultV2::Error {
        error: era_runtime_protocol::ServiceError {
            code: code.into(),
            message: "fixture failure".into(),
        },
    };
    for (first_invalid, first_error, last_error, exhausted, expected) in [
        (
            false,
            None,
            "unsupported",
            false,
            FaultCode::UnsupportedRuntimeFeature,
        ),
        (true, None, "unsupported", false, FaultCode::ServiceFailure),
        (
            false,
            Some("frontend.backend_failure"),
            "unsupported",
            false,
            FaultCode::ServiceFailure,
        ),
        (
            false,
            Some("unsupported"),
            "backend_failure",
            true,
            FaultCode::UnsupportedRuntimeFeature,
        ),
        (
            false,
            Some("frontend.unsupported"),
            "backend_failure",
            true,
            FaultCode::UnsupportedRuntimeFeature,
        ),
    ] {
        let (mut session, request) = start_html_query(
            "@SYSTEM_TITLE\nFLAG:1 = 99\nRESULTS:0 = old-head\nRESULTS:1 = old-tail\nFLAG:1 = HTML_STRINGLEN(\"<b>a</b><i>b</i>\", 1)\nWAIT\nRETURN\n",
            HTML_STRING_LEN_OPERATION,
            HTML_STRING_LEN_OPERATION_VERSION,
        );
        html_batch_edit_pending(&mut session, &request, |value| {
            if exhausted {
                value["budget"]["work"] = (64 * 1024 * 1024).into();
            }
        });
        let payload = decode_canonical(request.payload.as_slice()).unwrap();
        let mut response = html_test_measurement(&payload, 9000);
        if first_invalid {
            response.probes[0].result = HtmlProbeResultV2::FixedReady;
        }
        if let Some(code) = first_error {
            response.probes[0].result = backend(code);
        }
        response.probes[1].result = backend(last_error);
        let messages = html_batch_reply(&mut session, &request, &response, &mut 3);
        assert_eq!(session.phase(), RuntimePhase::Faulted, "{messages:#?}");
        let fault = messages
            .iter()
            .find_map(|message| match message {
                RuntimeMessage::Fault(fault) => Some(fault),
                _ => None,
            })
            .expect("fault");
        assert_eq!(fault.code, expected, "{messages:#?}");
        let origin = fault.origin.as_ref().expect("script origin");
        assert_eq!(origin.function, "SYSTEM_TITLE");
        let source = origin.source.as_ref().expect("source");
        assert_eq!(source.relative_path, "projection.erb");
        assert_eq!(source.line, Some(5));
        assert_eq!(html_flag(&session, 1), 99);
        assert_eq!(
            html_result(session.vm.as_ref().unwrap(), 0),
            VmValue::String("old-head".into())
        );
        assert_eq!(
            html_result(session.vm.as_ref().unwrap(), 1),
            VmValue::String("old-tail".into())
        );
        assert!(
            !messages
                .iter()
                .any(|message| matches!(message, RuntimeMessage::ServiceRequest(_)))
        );
    }
}

#[test]
fn html_probe_batch_commits_independent_unicode_parts_together() {
    let source = "@SYSTEM_TITLE\nFLAG:1 = 99\nFLAG:1 = HTML_STRINGLEN(\"<b>a</b><i>😀</i><u>b</u>\", 1)\nWAIT\nRETURN\n";
    let (mut session, request) = start_html_query(
        source,
        HTML_STRING_LEN_OPERATION,
        HTML_STRING_LEN_OPERATION_VERSION,
    );
    let payload: era_runtime_protocol::HtmlMeasureRequestV2 =
        decode_canonical(request.payload.as_slice()).unwrap();
    assert_eq!(payload.probes.len(), 3);
    assert_eq!(
        payload
            .probes
            .iter()
            .map(|probe| probe.id)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(html_flag(&session, 1), 99);
    assert!(!session.operations.is_snapshot_stable());
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
    let messages = pump_html_execution(&mut session, &mut 4);
    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:#?}");
    assert_eq!(html_flag(&session, 1), 27);
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::ServiceRequest(_)))
    );
}

#[test]
fn html_probe_batch_tracks_batch_chunk_and_slot_boundaries_with_snapshot_roundtrips() {
    let many = (0..17)
        .map(|n| if n % 2 == 0 { "<b>x</b>" } else { "<i>x</i>" })
        .collect::<String>();
    for (markup, expected_sizes) in [
        (many, vec![16, 1]),
        (
            format!("<nobr><b>x</b><i>{}</i><u>z</u></nobr>", "a".repeat(400)),
            vec![1, 1, 1, 1],
        ),
        ("<b>x</b><img src='face'><i>z</i>".into(), vec![1, 1, 1]),
    ] {
        let source =
            format!("@SYSTEM_TITLE\nFLAG:1 = HTML_STRINGLEN(\"{markup}\", 1)\nWAIT\nRETURN\n");
        let (mut session, mut request) = start_html_query(
            &source,
            HTML_STRING_LEN_OPERATION,
            HTML_STRING_LEN_OPERATION_VERSION,
        );
        let mut sizes = Vec::new();
        let mut ids = Vec::new();
        let mut cut_counts = Vec::new();
        let mut work = 0;
        let mut measurements = 0;
        let mut sequence = 3;
        for _ in 0..64 {
            html_batch_edit_pending(&mut session, &request, |value| {
                let next_work = value["budget"]["work"].as_u64().unwrap();
                let next_measurements = value["budget"]["measurements"].as_u64().unwrap();
                assert!(next_work >= work && next_measurements >= measurements);
                (work, measurements) = (next_work, next_measurements);
            });
            let payload: era_runtime_protocol::HtmlMeasureRequestV2 =
                decode_canonical(request.payload.as_slice()).unwrap();
            sizes.push(payload.probes.len());
            ids.extend(payload.probes.iter().map(|probe| probe.id));
            cut_counts.extend(payload.probes.iter().map(|probe| probe.cuts.len()));
            let messages = html_batch_reply(
                &mut session,
                &request,
                &html_test_measurement(&payload, 9000),
                &mut sequence,
            );
            if session.phase() == RuntimePhase::WaitingInput {
                break;
            }
            request = messages
                .into_iter()
                .find_map(|message| match message {
                    RuntimeMessage::ServiceRequest(request) => Some(request),
                    _ => None,
                })
                .expect("next measurement");
        }
        assert_eq!(session.phase(), RuntimePhase::WaitingInput);
        assert_eq!(sizes, expected_sizes);
        assert_eq!(
            ids,
            (1..=u32::try_from(ids.len()).unwrap()).collect::<Vec<_>>()
        );
        if markup.contains(&"a".repeat(400)) {
            assert_eq!(cut_counts, vec![2, 256, 145, 2]);
        }
    }
}

#[test]
fn html_probe_batch_pending_debug_and_diagnosis_snapshots_decode_but_normal_is_rejected() {
    let (mut session, request) = start_html_query(
        "@SYSTEM_TITLE\nFLAG:1 = HTML_STRINGLEN(\"<b>a</b><i>b</i>\", 1)\nWAIT\nRETURN\n",
        HTML_STRING_LEN_OPERATION,
        HTML_STRING_LEN_OPERATION_VERSION,
    );
    session
        .negotiated_features
        .insert(RuntimeFeature::VmSnapshot);
    session.active_debug_grant = Some(ActiveDebugGrant {
        token: GrantToken {
            grant_id: SessionId { high: 1, low: 2 },
            session_epoch: session.epoch.0,
            program_generation: 0,
            issued_runtime_revision: session.revision,
        },
        scopes: BTreeSet::from([DebugScope::ExecutionControl]),
    });
    for purpose in [
        SnapshotExportPurpose::Normal,
        SnapshotExportPurpose::Debug,
        SnapshotExportPurpose::Diagnosis,
    ] {
        session
            .export_state(
                100,
                StateExportRequest {
                    kind: StateExportKind::VmSnapshot,
                    snapshot_purpose: purpose,
                },
            )
            .unwrap();
        let messages = drain(&mut session);
        if purpose == SnapshotExportPurpose::Normal {
            assert!(messages.iter().any(|message| matches!(
                message,
                RuntimeMessage::StateExportReady(StateExportReady {
                    result: StateExportResult::Ineligible { .. },
                    ..
                })
            )));
            assert!(session.outbound_transfer.is_none());
        } else {
            let transfer = session
                .outbound_transfer
                .take()
                .expect("diagnostic snapshot");
            let bytes = transfer.bytes.copy_range(0..transfer.bytes.len());
            let mut decoded = runtime_snapshot::decode(&bytes, usize::MAX).unwrap();
            assert!(matches!(
                decoded.operations.take_service(request.request_id),
                Some(PendingService::Host(ExternalCompletion::HtmlQuery { .. }))
            ));
        }
    }
}

#[test]
fn html_probe_batch_remaining_measurement_budget_shrinks_without_consuming_unsent_ids() {
    for count in [17, 18] {
        let markup = (0..count)
            .map(|n| if n % 2 == 0 { "<b>x</b>" } else { "<i>x</i>" })
            .collect::<String>();
        let source = format!(
            "@SYSTEM_TITLE\nFLAG:1 = 99\nFLAG:1 = HTML_STRINGLEN(\"{markup}\", 1)\nWAIT\nRETURN\n"
        );
        let (mut session, request) = start_html_query(
            &source,
            HTML_STRING_LEN_OPERATION,
            HTML_STRING_LEN_OPERATION_VERSION,
        );
        html_batch_edit_pending(&mut session, &request, |value| {
            value["budget"]["measurements"] = (1_048_576 - 2).into();
        });
        let payload = decode_canonical(request.payload.as_slice()).unwrap();
        let mut sequence = 3;
        let messages = html_batch_reply(
            &mut session,
            &request,
            &html_test_measurement(&payload, 9000),
            &mut sequence,
        );
        let next = messages
            .into_iter()
            .find_map(|message| match message {
                RuntimeMessage::ServiceRequest(request) => Some(request),
                _ => None,
            })
            .expect("one remaining part fits");
        let payload: era_runtime_protocol::HtmlMeasureRequestV2 =
            decode_canonical(next.payload.as_slice()).unwrap();
        assert_eq!(payload.probes.len(), 1);
        assert_eq!(payload.probes[0].id, 17);
        let messages = html_batch_reply(
            &mut session,
            &next,
            &html_test_measurement(&payload, 9000),
            &mut sequence,
        );
        if count == 17 {
            assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:#?}");
            assert_eq!(html_flag(&session, 1), 153);
        } else {
            assert_eq!(session.phase(), RuntimePhase::Faulted, "{messages:#?}");
            assert_eq!(html_flag(&session, 1), 99);
            assert!(messages.iter().any(|message| matches!(
                message,
                RuntimeMessage::Fault(RuntimeFault {
                    code: FaultCode::ResourceLimit,
                    ..
                })
            )));
            assert!(
                !messages
                    .iter()
                    .any(|message| matches!(message, RuntimeMessage::ServiceRequest(_)))
            );
        }
    }
}

#[test]
fn html_probe_batch_keeps_dependent_nested_lengths_serial_and_cancellation_atomic() {
    for lines in [false, true] {
        for cancel in [false, true] {
            let call = if lines {
                "FLAG:1 = HTML_STRINGLINES(\"<b>a</b><i>b</i>c\", WIDTH())"
            } else {
                "STR:0 '= HTML_SUBSTRING(\"<b>a</b><i>b</i>c\", WIDTH())"
            };
            let source = format!(
                "@SYSTEM_TITLE\nRESULTS:0 = old-head\nRESULTS:1 = old-tail\n{call}\nFLAG:5 = 1\nWAIT\nRETURN\n@WIDTH\n#FUNCTION\nFLAG:0 += 1\nRETURNF 1\n"
            );
            let operation = if lines {
                HTML_STRING_LINES_OPERATION
            } else {
                HTML_SUBSTRING_OPERATION
            };
            let (mut session, mut request) =
                start_html_query(&source, operation, ProtocolVersion::new(2, 0));
            let mut sequence = 3;
            let mut found = false;
            let mut previous_work = 0;
            for _ in 0..16 {
                html_batch_edit_pending(&mut session, &request, |value| {
                    let work = value["budget"]["work"].as_u64().unwrap();
                    assert!(work >= previous_work);
                    previous_work = work;
                });
                let payload: era_runtime_protocol::HtmlMeasureRequestV2 =
                    decode_canonical(request.payload.as_slice()).unwrap();
                let response = html_test_measurement(&payload, 9000);
                // SUBSTRING authors one scalar/atomic fragment, never a whole prefix.
                // Independent batching must not prefetch the next dependent fragment.
                assert_eq!(payload.probes.len(), 1);
                if sequence > 3 {
                    found = true;
                    assert_eq!(html_flag(&session, 0), 1);
                    assert_eq!(html_flag(&session, 5), 0);
                    html_batch_fail_or_cancel(
                        &mut session,
                        &request,
                        &response,
                        &mut sequence,
                        cancel,
                    );
                    break;
                }
                let messages = html_batch_reply(&mut session, &request, &response, &mut sequence);
                request = messages
                    .into_iter()
                    .find_map(|message| match message {
                        RuntimeMessage::ServiceRequest(request) => Some(request),
                        _ => None,
                    })
                    .expect("next dependent scalar measurement");
            }
            assert!(found, "fixture must reach its second dependent length");
        }
    }
}

#[test]
fn html_probe_batch_chunk_budget_is_not_reset_between_prefix_requests() {
    for extra in [false, true] {
        let tail = if extra { "<i>x</i>" } else { "" };
        let source = format!(
            "@SYSTEM_TITLE\nFLAG:1 = 99\nFLAG:1 = HTML_STRINGLEN(\"<nobr><b>{}</b>{tail}</nobr>\", 1)\nWAIT\nRETURN\n",
            "a".repeat(400)
        );
        let (mut session, request) = start_html_query(
            &source,
            HTML_STRING_LEN_OPERATION,
            HTML_STRING_LEN_OPERATION_VERSION,
        );
        let payload: era_runtime_protocol::HtmlMeasureRequestV2 =
            decode_canonical(request.payload.as_slice()).unwrap();
        assert_eq!(payload.probes[0].cuts.len(), 256);
        html_batch_edit_pending(&mut session, &request, |value| {
            value["budget"]["measurements"] = (1_048_576 - 145).into();
        });
        let mut sequence = 3;
        let messages = html_batch_reply(
            &mut session,
            &request,
            &html_test_measurement(&payload, 9000),
            &mut sequence,
        );
        let next = messages
            .into_iter()
            .find_map(|message| match message {
                RuntimeMessage::ServiceRequest(request) => Some(request),
                _ => None,
            })
            .expect("remaining cuts");
        html_batch_edit_pending(&mut session, &next, |_| {});
        let payload: era_runtime_protocol::HtmlMeasureRequestV2 =
            decode_canonical(next.payload.as_slice()).unwrap();
        assert_eq!(payload.probes[0].cuts.len(), 145);
        let messages = html_batch_reply(
            &mut session,
            &next,
            &html_test_measurement(&payload, 9000),
            &mut sequence,
        );
        if extra {
            assert_eq!(session.phase(), RuntimePhase::Faulted, "{messages:#?}");
            assert_eq!(html_flag(&session, 1), 99);
            assert!(messages.iter().any(|message| matches!(
                message,
                RuntimeMessage::Fault(RuntimeFault {
                    code: FaultCode::ResourceLimit,
                    ..
                })
            )));
        } else {
            assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:#?}");
        }
        assert!(
            !messages
                .iter()
                .any(|message| matches!(message, RuntimeMessage::ServiceRequest(_)))
        );
    }
}

#[test]
fn html_probe_batch_lazy_lines_carries_budget_into_the_next_width_step() {
    let source = "@SYSTEM_TITLE\nFLAG:1 = 99\nFLAG:1 = HTML_STRINGLINES(\"<b>a</b><i>b</i>c\", WIDTH())\nWAIT\nRETURN\n@WIDTH\n#FUNCTION\nFLAG:0 += 1\nIF FLAG:0 == 2\nINPUT\nENDIF\nRETURNF 2\n";
    let mut session = prepare_html_execution(source, Some(ProtocolVersion::new(2, 0)));
    let (mut sequence, _) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(html_flag(&session, 0), 2);
    let mut operations = serde_json::to_value(&session.operations).unwrap();
    let flow = operations["html_lines"]["entries"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .expect("unfinished lazy flow");
    assert!(flow["budget"]["work"].as_u64().unwrap() > 0);
    flow["budget"]["work"] = (64 * 1024 * 1024).into();
    session.operations = serde_json::from_value(operations).unwrap();
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
    assert_eq!(session.phase(), RuntimePhase::Faulted, "{messages:#?}");
    assert_eq!(html_flag(&session, 0), 2);
    assert_eq!(html_flag(&session, 1), 99);
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Fault(RuntimeFault {
            code: FaultCode::ResourceLimit,
            ..
        })
    )));
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::ServiceRequest(_)))
    );
}

#[test]
fn html_probe_batch_rejects_incomplete_duplicate_and_malformed_results_atomically() {
    for defect in 0..7 {
        let source = "@SYSTEM_TITLE\nFLAG:1 = 99\nFLAG:1 = HTML_STRINGLEN(\"<b>a</b><i>b</i>\", 1)\nWAIT\nRETURN\n";
        let (mut session, request) = start_html_query(
            source,
            HTML_STRING_LEN_OPERATION,
            HTML_STRING_LEN_OPERATION_VERSION,
        );
        let payload = decode_canonical(request.payload.as_slice()).unwrap();
        let mut response = html_test_measurement(&payload, 9000);
        assert_eq!(response.probes.len(), 2);
        match defect {
            0 => {
                response.probes.pop();
            }
            1 => response.probes[1].id = response.probes[0].id,
            2 => response.probes[1].result = era_runtime_protocol::HtmlProbeResultV2::FixedReady,
            3 => response.probes[1].id = 999,
            4 => response.probes.push(response.probes[0].clone()),
            5 => response.probes.clear(),
            _ => response.probes.reverse(),
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
        assert_eq!(html_flag(&session, 1), 99);
    }
}

#[test]
fn html_probe_batch_is_bounded_and_large_parts_keep_the_single_probe_path() {
    let markup = (0..17)
        .map(|index| {
            if index % 2 == 0 {
                "<b>x</b>"
            } else {
                "<i>x</i>"
            }
        })
        .collect::<String>();
    for (markup, expected_batch, expected_length) in [
        (markup, 16, 153),
        (format!("<b>{}</b><i>x</i>", "a".repeat(40)), 1, 369),
    ] {
        let source =
            format!("@SYSTEM_TITLE\nFLAG:1 = HTML_STRINGLEN(\"{markup}\", 1)\nWAIT\nRETURN\n");
        let (mut session, request) = start_html_query(
            &source,
            HTML_STRING_LEN_OPERATION,
            HTML_STRING_LEN_OPERATION_VERSION,
        );
        let payload: era_runtime_protocol::HtmlMeasureRequestV2 =
            decode_canonical(request.payload.as_slice()).unwrap();
        assert_eq!(payload.probes.len(), expected_batch);
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
        pump_html_execution(&mut session, &mut 4);
        assert_eq!(session.phase(), RuntimePhase::WaitingInput);
        assert_eq!(html_flag(&session, 1), expected_length);
    }
}
