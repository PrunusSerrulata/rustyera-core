#[test]
fn runtime_project_load_keeps_spritecreate_arity_profile_boundary() {
    for (profile, arity, expected_success) in [
        (erabasic_compat::CompatibilityProfileId::EmueraEm, 2, true),
        (erabasic_compat::CompatibilityProfileId::EmueraEm, 6, true),
        (erabasic_compat::CompatibilityProfileId::EmueraEm, 8, false),
        (erabasic_compat::CompatibilityProfileId::EmueraEm, 10, false),
        (
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
            2,
            true,
        ),
        (
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
            6,
            true,
        ),
        (
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
            8,
            true,
        ),
        (
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
            10,
            true,
        ),
    ] {
        let arguments = match arity {
            2 => "\"S\", 1",
            6 => "\"S\", 1, 0, 0, 2, 1",
            8 => "\"S\", 1, 0, 0, 2, 1, -3, 4",
            10 => "\"S\", 1, 0, 0, 2, 1, -3, 4, 7, 9",
            _ => unreachable!(),
        };
        let compatibility = erabasic_compat::CompatibilityIdentity::for_profile(profile);
        let mut session = negotiated_session();
        submit(
            &mut session,
            1,
            RuntimeMessage::ProjectManifest(ProjectManifest {
                compatibility,
                project_revision: 1,
                files: vec![
                    profile_configuration_file(profile),
                    SubmittedFile {
                        relative_path: "main.erb".into(),
                        category: FileCategory::Erb,
                        payload: FilePayload::Utf8(format!(
                            "@SYSTEM_TITLE\nRESULT = SPRITECREATE({arguments})\nRETURN\n"
                        )),
                        content_hash: None,
                    },
                ],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        let messages = drain(&mut session);
        let report = messages
            .iter()
            .find_map(|message| match message {
                RuntimeMessage::ProjectLoadReport(report) => Some(report),
                _ => None,
            })
            .expect("project load report");
        assert_eq!(
            report.success, expected_success,
            "{profile:?} arity {arity}: {:?}",
            report.diagnostics
        );
    }
}

#[test]
fn dynamic_varsize_uses_host_defaults_but_callstr_unique_restructure_dereferences_null() {
    let source = "@SYSTEM_TITLE\nFLAG:10 = VARSIZE(\"FLAG\")\nRESULTS:10 '= \"{VARSIZE(\\\"FLAG\\\")}|{VARSIZE(\\\"FLAG\\\",0)}|{VARSIZE(\\\"FLAG\\\",,)}\"\nRESULTS:12 '= STRFORM(RESULTS:10)\nRESULTS:11 '= \"TAKE(VARSIZE(\\\"FLAG\\\",,))\"\nFLAG:0 = STRFORMCHECK(\"{CALLER()}\")\nWAIT\nRETURN\n@CALLER\n#FUNCTION\nFLAG:1 += 1\nTRYCCALLSTR RESULTS:11\nCATCH\nFLAG:2 = 1\nENDCATCH\nFLAG:3 = 1\nRETURNF 1\n@TAKE(ARG)\nFLAG:4 = 1\nRETURN\n";
    let (session, _, messages) = run_immediate_query_project_with_profile(
        source,
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:?}");
    let vm = session.vm.as_ref().unwrap();
    let length = read_runtime_integer(vm, "FLAG", &[10], None).unwrap();
    let text = vm
        .read_runtime_state(&[erabasic_vm::VmRuntimeRead {
            variable: runtime_variable_key(vm, "RESULTS").unwrap(),
            indices: vec![12],
            character: None,
        }])
        .unwrap()
        .remove(0);
    assert_eq!(text, VmValue::String(format!("{length}|{length}|{length}")));
    assert_eq!(read_runtime_integer(vm, "FLAG", &[0], None).unwrap(), 0);
    assert_eq!(read_runtime_integer(vm, "FLAG", &[1], None).unwrap(), 1);
    for index in [2, 3, 4] {
        assert_eq!(read_runtime_integer(vm, "FLAG", &[index], None).unwrap(), 0);
    }
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::Fault(_)))
    );
}

#[test]
fn varsize_dimension_narrowing_is_shared_by_static_and_dynamic_calls_in_both_profiles() {
    let source = "@SYSTEM_TITLE\nFLAG:10 = VARSIZE(\"FLAG\")\nFLAG:11 = VARSIZE(\"FLAG\",4294967296)\nRESULTS:10 = {VARSIZE(\"FLAG\",4294967296)}|{VARSIZE(\"FLAG\",(-9223372036854775807 - 1))}\nRESULTS:12 '= STRFORM(RESULTS:10)\nWAIT\nRETURN\n";
    for profile in [
        erabasic_compat::CompatibilityProfileId::EmueraEm,
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    ] {
        let (session, _, messages) = run_immediate_query_project_with_profile(
            source,
            erabasic_compat::CompatibilityIdentity::for_profile(profile),
        );
        assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:?}");
        let vm = session.vm.as_ref().unwrap();
        let length = read_runtime_integer(vm, "FLAG", &[10], None).unwrap();
        assert_eq!(
            read_runtime_integer(vm, "FLAG", &[11], None).unwrap(),
            length
        );
        let value = vm
            .read_runtime_state(&[erabasic_vm::VmRuntimeRead {
                variable: runtime_variable_key(vm, "RESULTS").unwrap(),
                indices: vec![12],
                character: None,
            }])
            .unwrap()
            .remove(0);
        assert_eq!(value, VmValue::String(format!("{length}|{length}")));
    }
}

#[test]
fn animation_timer_preserves_profile_forms_and_snake_command_result() {
    let snake = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let (session, _, messages) = run_immediate_query_project_with_profile(
        "@SYSTEM_TITLE\nRESULT = 77\nSETANIMETIMER 1\nFLAG:0 = RESULT\nFLAG:1 = GETANIMETIMER()\nBITMAP_CACHE_ENABLE 1\nBITMAP_CACHE_ENABLE 0\nFLAG:2 = RESULT\nWAIT\nRETURN\n",
        snake,
    );
    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:?}");
    let vm = session.vm.as_ref().unwrap();
    assert_eq!(read_runtime_integer(vm, "FLAG", &[0], None).unwrap(), 77);
    assert_eq!(read_runtime_integer(vm, "FLAG", &[1], None).unwrap(), 10);
    assert_eq!(read_runtime_integer(vm, "FLAG", &[2], None).unwrap(), 77);
    assert_eq!(
        session
            .project_snapshot
            .as_ref()
            .unwrap()
            .resource_graph
            .animation_timer(),
        10
    );
    let notices = messages
        .iter()
        .filter_map(|message| match message {
            RuntimeMessage::Diagnostic(diagnostic)
                if diagnostic.code == "compat.bitmap_cache_enable_noop" =>
            {
                Some(diagnostic)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(notices.len(), 1, "{messages:#?}");
    assert_eq!(notices[0].level, RuntimeLogLevel::Warning);
    assert_eq!(notices[0].notification, DiagnosticNotification::LogOnly);
    assert_eq!(
        notices[0]
            .context
            .as_ref()
            .and_then(|context| context.api.as_deref()),
        Some("bitmap_cache_enable")
    );

    let (session, _, messages) = run_immediate_query_project(
        "@SYSTEM_TITLE\nRESULT = SETANIMETIMER(1)\nFLAG:0 = RESULT\nWAIT\nRETURN\n",
    );
    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:?}");
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "FLAG", &[0], None).unwrap(),
        1
    );
}

#[test]
fn snake_cbg_and_image_layer_calls_project_one_ordered_scene_authority() {
    let snake = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let source = "@SYSTEM_TITLE\n\
        FLAG:0 = GCREATE(1, 4, 4)\n\
        FLAG:1 = SPRITECREATE(\"ONE\", 1)\n\
        FLAG:2 = SPRITECREATE(\"HOVER\", 1)\n\
        FLAG:3 = CBGSETG(1, 10, 20, 5)\n\
        FLAG:4 = CBGSETSPRITE(\"ONE\")\n\
        FLAG:5 = CBGSETBUTTONSPRITE(33, \"ONE\", \"HOVER\", 1, 2, 6, \"tip\")\n\
        FLAG:6 = CBGSETBMAPG(1)\n\
        SETIMAGELAYER \"ONE\", 7\n\
        FLAG:7 = EXISTSIMAGELAYER(7)\n\
        SETIMAGELAYER \"ONE\", 7, 3, 4, 5, 6, 128\n\
        SETIMAGELAYERL \"ONE\", 8\n\
        CLEARIMAGELAYER 7\n\
        FLAG:8 = EXISTSIMAGELAYER(7)\n\
        CLEARIMAGELAYER_ALL\n\
        FLAG:9 = CBGREMOVERANGE(5, 5)\n\
        FLAG:10 = CBGCLEARBUTTON()\n\
        FLAG:11 = CBGSETG(1, 30, 40, 6)\n\
        FLAG:12 = CBGSETBUTTONSPRITE(44, \"MISSING\", \"HOVER\", 1, 2, 9)\n\
        WAIT\n\
        RETURN\n";
    let (mut session, _, messages) = run_immediate_query_project_with_profile(source, snake);
    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:#?}");
    let vm = session.vm.as_ref().unwrap();
    for index in 0..=7 {
        assert_eq!(
            read_runtime_integer(vm, "FLAG", &[index], None).unwrap(),
            1,
            "FLAG:{index}"
        );
    }
    assert_eq!(read_runtime_integer(vm, "FLAG", &[8], None).unwrap(), 0);
    assert_eq!(read_runtime_integer(vm, "FLAG", &[9], None).unwrap(), 1);
    assert_eq!(read_runtime_integer(vm, "FLAG", &[10], None).unwrap(), 1);
    assert_eq!(read_runtime_integer(vm, "FLAG", &[11], None).unwrap(), 1);
    assert_eq!(read_runtime_integer(vm, "FLAG", &[12], None).unwrap(), 1);

    session
        .presentation
        .set_projection(true, true, false, true, false);
    let snapshot = session.presentation.snapshot();
    assert_eq!(snapshot.scene.layers.len(), 2);
    assert_eq!(snapshot.scene.layers[0].depth, 6);
    assert_eq!(
        snapshot.scene.layers[0].offset.x,
        era_runtime_protocol::LogicalLength(30_000)
    );
    assert_eq!(
        snapshot.scene.layers[0].offset.y,
        era_runtime_protocol::LogicalLength(40_000)
    );
    assert_eq!(snapshot.scene.layers[1].depth, 1);
    assert!(matches!(
        &snapshot.scene.layers[1].source,
        era_runtime_protocol::SceneSourceV1::Sprite { sprite_name, .. }
            if sprite_name == "ONE"
    ));
    assert!(
        snapshot
            .scene
            .layers
            .iter()
            .all(|layer| layer.interaction.is_none())
    );
}

#[test]
fn snake_cbg_rejects_reserved_zero_depth_without_mutating_the_scene() {
    let snake = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let (session, _, messages) = run_immediate_query_project_with_profile(
        "@SYSTEM_TITLE\nFLAG:0 = GCREATE(1, 1, 1)\nFLAG:1 = CBGSETG(1, 0, 0, 0)\nWAIT\nRETURN\n",
        snake,
    );
    assert_eq!(session.phase(), RuntimePhase::Faulted, "{messages:#?}");
    assert!(session.presentation.snapshot().scene.layers.is_empty());
}

#[test]
fn snake_display_queries_and_whole_line_background_use_canonical_history() {
    let snake = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let (session, _, messages) = run_immediate_query_project_with_profile(
        "@SYSTEM_TITLE\nPRINTL oldest\nPRINT pending\nRESULTS '= GETDISPLAYLINE(-1)\nTEXT_BGC_ON 1122867, 50\nWAIT\nRETURN\n",
        snake,
    );
    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:?}");
    assert_eq!(
        read_runtime_string(session.vm.as_ref().unwrap(), "RESULTS").unwrap(),
        "oldest"
    );
    let snapshot = session.presentation.snapshot();
    assert_eq!(
        snapshot.settings.text_line_background,
        Some(era_runtime_protocol::Color {
            red: 0x11,
            green: 0x22,
            blue: 0x33,
            alpha: 127,
        })
    );
    assert!(
        snapshot
            .history
            .logical_lines
            .iter()
            .all(|line| line.text_background_eligible)
    );
}

#[test]
fn invalid_animation_timer_is_atomic() {
    let snake = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let (session, _, messages) = run_immediate_query_project_with_profile(
        "@SYSTEM_TITLE\nSETANIMETIMER 20\nSETANIMETIMER 32768\nRETURN\n",
        snake,
    );
    assert_eq!(session.phase(), RuntimePhase::Faulted, "{messages:?}");
    assert_eq!(
        session
            .project_snapshot
            .as_ref()
            .unwrap()
            .resource_graph
            .animation_timer(),
        20
    );
    assert!(
        messages.iter().any(|message| match message {
            RuntimeMessage::PresentationSnapshot(snapshot) => {
                snapshot.resources.animation_timer_ms == 20
            }
            RuntimeMessage::PresentationDelta(delta) => {
                delta.operations.iter().any(|operation| {
                    matches!(
                        operation,
                        PresentationOperation::SetResources { resources }
                            if resources.animation_timer_ms == 20
                    )
                })
            }
            _ => false,
        }),
        "{messages:#?}"
    );
}
