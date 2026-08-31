use super::*;

#[test]
fn progress_covers_declaration_and_indexing_work_without_changing_analysis() {
    use erabasic_analyzer::{AnalysisProgressStage, analyze_project_with_progress};
    let input = AnalysisInput {
        project_data: empty_project(),
        sources: vec![
            source("header.erh", "#DIM CONST LATER = 3\n#DIM VALUES, LATER\n"),
            source(
                "main.erb",
                "@SYSTEM_TITLE\n#DIM PRIVATE X\nRETURN\n@UNUSED\nRETURN\n",
            ),
        ],
    };
    let options = AnalyzerOptions::analysis_mode();
    let expected = analyze_project(input.clone(), &options, &ExtensionRegistry::default());
    let events = std::sync::Mutex::new(Vec::new());
    let actual =
        analyze_project_with_progress(input, &options, &ExtensionRegistry::default(), &|event| {
            events.lock().unwrap().push(event);
        });
    assert_eq!(actual, expected);
    let events = events.into_inner().unwrap();
    for stage in [
        AnalysisProgressStage::DeclaringGlobals,
        AnalysisProgressStage::IndexingFunctions,
        AnalysisProgressStage::DeclaringLocals,
        AnalysisProgressStage::Analyzing,
    ] {
        let work: Vec<_> = events.iter().filter(|event| event.stage == stage).collect();
        assert_eq!(work.first().unwrap().completed, 0);
        let final_event = work.last().unwrap();
        assert!(final_event.total > 0);
        assert_eq!(final_event.completed, final_event.total);
        assert!(
            work.windows(2)
                .all(|pair| pair[0].completed <= pair[1].completed)
        );
    }
}

#[test]
fn frontend_observation_reports_source_and_control_dependency() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "projection.erb",
                "@SYSTEM_TITLE\nIF CLIENTWIDTH() > 640\nPRINT wide\nENDIF\nRETURN\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );
    assert!(
        report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == AnalyzerDiagnosticCode::FrontendObservationSource
                && diagnostic
                    .source
                    .as_ref()
                    .is_some_and(|source| source.byte_start > 0)
        }),
        "{:#?}",
        report.diagnostics
    );
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == AnalyzerDiagnosticCode::FrontendObservationDependency
            && diagnostic.severity == erabasic_analyzer::AnalyzerDiagnosticSeverity::Warning
    }));
}

#[test]
fn restart_branches_to_the_current_function_entry_without_falling_through() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "restart.erb",
                "@SYSTEM_TITLE\nPRINT first\nRESTART\nPRINT unreachable\nRETURN\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );
    assert!(
        !report.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.severity,
            erabasic_analyzer::AnalyzerDiagnosticSeverity::Error
                | erabasic_analyzer::AnalyzerDiagnosticSeverity::Fatal
        )),
        "{:#?}",
        report.diagnostics
    );
    let function = &report
        .project
        .expect("valid RESTART project")
        .program
        .functions[0];
    let restart = function
        .lines
        .iter()
        .find(|line| {
            matches!(
                &line.kind,
                HirStatementKind::Instruction { target, .. } if target.name() == "RESTART"
            )
        })
        .expect("RESTART line");
    assert!(function.control_flow.iter().any(|edge| {
        edge.kind == erabasic_hir::ControlFlowKind::Goto
            && edge.from == restart.id
            && edge.to == function.lines.first().map(|line| line.id)
    }));
    assert!(!function.control_flow.iter().any(|edge| {
        edge.kind == erabasic_hir::ControlFlowKind::Next && edge.from == restart.id
    }));
}

#[test]
fn inline_comment_does_not_become_part_of_a_static_call_target() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "calls.erb",
                "@SYSTEM_TITLE\nCALL HELPER; inline comment\nRETURN\n@HELPER\nRESULT = 1\nRETURN\n",
            )],
        },
        &AnalyzerOptions::default(),
        &ExtensionRegistry::default(),
    );
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.reference_level >= 2),
        "{:#?}",
        report.diagnostics
    );
    let project = report.project.unwrap();
    let title_function = project
        .program
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap();
    let helper_function = project
        .program
        .functions
        .iter()
        .find(|function| function.name == "HELPER")
        .unwrap();
    assert!(
        !helper_function.lines.is_empty(),
        "callee was treated as unreachable"
    );
    assert!(title_function.control_flow.iter().any(|edge| {
        edge.kind == erabasic_hir::ControlFlowKind::Call
            && edge.function == Some(helper_function.id)
    }));
}

#[test]
fn trycallf_keeps_its_static_target_reachable() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "trycallf.erb",
                "@SYSTEM_TITLE\nTRYCALLF HELPER(1)\nRETURN\n@HELPER(ARG)\nRETURN\n",
            )],
        },
        &AnalyzerOptions::default(),
        &ExtensionRegistry::default(),
    );
    let project = report.project.expect("valid TRYCALLF project");
    let helper = project
        .program
        .functions
        .iter()
        .find(|function| function.name == "HELPER")
        .unwrap();
    assert!(!helper.lines.is_empty());
}

#[test]
fn static_targets_expand_rename_indices_before_symbol_lookup() {
    let mut data = empty_project();
    data.static_data
        .rename
        .insert("[[TARGET_NO]]".into(), "7".into());
    let report = analyze_project(
        AnalysisInput {
            project_data: data,
            sources: vec![source(
                "rename-call.erb",
                "@SYSTEM_TITLE\nTRYCALL HELPER[[TARGET_NO]]\nRETURN\n@HELPER7\nRETURN\n",
            )],
        },
        &AnalyzerOptions::default(),
        &ExtensionRegistry::default(),
    );
    let project = report.project.expect("valid renamed static target");
    let helper = project
        .program
        .functions
        .iter()
        .find(|function| function.name == "HELPER7")
        .unwrap();
    assert!(!helper.lines.is_empty());
}

#[test]
fn trygoto_list_func_entries_remain_labels_not_function_targets() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "function-lists.erb",
                "@SYSTEM_TITLE\nTRYCALLLIST\nFUNC CALLED\nENDFUNC\nTRYGOTOLIST\nFUNC LABEL_ONLY\nENDFUNC\n$LABEL_ONLY\nRETURN\n@CALLED\nRETURN\n@LABEL_ONLY\nRETURN\n",
            )],
        },
        &AnalyzerOptions::default(),
        &ExtensionRegistry::default(),
    );
    let project = report.project.expect("valid TRY*LIST project");
    for (name, expected) in [("CALLED", true), ("LABEL_ONLY", false)] {
        let function = project
            .program
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap();
        assert_eq!(!function.lines.is_empty(), expected, "{name}");
    }
}

#[test]
fn structured_formatted_try_call_keeps_dynamic_targets_reachable() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "requests.erb",
                "@SYSTEM_TITLE\n\
                 #DIM REQUEST_ID\n\
                 REQUEST_ID = 2005\n\
                 TRYCCALLFORM IRAI_一般{REQUEST_ID % 1000}(2, REQUEST_ID, \"依頼実行時\")\n\
                 CATCH\n\
                 FLAG:0 = -1\n\
                 ENDCATCH\n\
                 RETURN\n\
                 @IRAI_一般5(CHARA, IRAI_ID, SCENE)\n\
                 #DIM CHARA\n\
                 #DIM IRAI_ID\n\
                 #DIMS SCENE\n\
                 FLAG:0 = CHARA + IRAI_ID + (SCENE == \"依頼実行時\")\n\
                 RETURN\n",
            )],
        },
        &AnalyzerOptions::default(),
        &ExtensionRegistry::default(),
    );
    assert!(
        !report.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.severity,
            erabasic_analyzer::AnalyzerDiagnosticSeverity::Error
                | erabasic_analyzer::AnalyzerDiagnosticSeverity::Fatal
        )),
        "{:#?}",
        report.diagnostics
    );
    let project = report.project.expect("valid dynamic call project");
    let target = project
        .program
        .functions
        .iter()
        .find(|function| function.name == "IRAI_一般5")
        .expect("dynamic target declaration");
    assert!(
        !target.lines.is_empty(),
        "TRYCCALLFORM target was treated as unreachable"
    );
}

#[test]
fn bounded_dynamic_methods_and_runtime_forms_reach_only_their_candidates() {
    for (statement, expected) in [
        (
            "RESULT = GETMETH(\"CAN_MOVE_\" + TOSTR(1), 0)",
            "CAN_MOVE_1",
        ),
        (
            "#DIM DYNAMIC VALUE\nVALUE = GETMETH(\"CAN_MOVE_\" + TOSTR(1), 0)",
            "CAN_MOVE_1",
        ),
        (
            "#DIM DYNAMIC STR\nSTR = GETMETH(\"CAN_MOVE_\" + TOSTR(1), 0)",
            "CAN_MOVE_1",
        ),
        ("RESULTS '= GETMETHS(\"SCOM_\" + TOSTR(1), \"\")", "SCOM_1"),
        ("RESULT = EXISTMETH(\"CAN_MOVE_\" + TOSTR(1))", "CAN_MOVE_1"),
        (
            "PRINTFORML {GETMETH(\"CAN_MOVE_\" + TOSTR(1), 0)}",
            "CAN_MOVE_1",
        ),
        (
            "PRINTFORML {1, GETMETH(\"CAN_MOVE_\" + TOSTR(1), 0)}",
            "CAN_MOVE_1",
        ),
        (
            "PRINTFORML \\@ GETMETH(\"CAN_MOVE_\" + TOSTR(1), 0) ? yes # no \\@",
            "CAN_MOVE_1",
        ),
        ("RESULTS '= STRFORM(\"SCOM_1()\")", "SCOM_1"),
        ("RESULT = STRFORMCHECK(\"CAN_MOVE_1()\")", "CAN_MOVE_1"),
        ("RESULT = EXISTVAR(\"CAN_MOVE_1()\", 1)", "CAN_MOVE_1"),
    ] {
        let mut options = AnalyzerOptions::default();
        if statement.contains("STRFORMCHECK") || statement.contains("EXISTVAR") {
            options.compatibility = erabasic_compat::CompatibilityIdentity::for_profile(
                erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
            );
        }
        let report = analyze_project(
            AnalysisInput {
                project_data: empty_project(),
                sources: vec![source(
                    "reachability.erb",
                    &format!(
                        "@SYSTEM_TITLE\n{statement}\nRETURN\n@CAN_MOVE_1\n#FUNCTION\nRETURNF 1\n@SCOM_1\n#FUNCTIONS\nRETURNF \"one\"\n"
                    ),
                )],
            },
            &options,
            &ExtensionRegistry::default(),
        );
        assert!(
            !report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.reference_level >= 2),
            "{statement}: {:#?}",
            report.diagnostics
        );
        let project = report.project.expect("reachable method project");
        for name in ["CAN_MOVE_1", "SCOM_1"] {
            let method = project
                .program
                .functions
                .iter()
                .find(|function| function.name == name)
                .expect("method definition");
            assert_eq!(
                !method.lines.is_empty(),
                name == expected,
                "{statement} selected the wrong dynamic candidate {name}"
            );
        }
    }
}

#[test]
fn unbounded_dynamic_targets_block_pruning_and_keep_every_candidate() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "unbounded.erb",
                "@SYSTEM_TITLE\n#DIMS TARGET_NAME\nTRYCALLFORM %TARGET_NAME%\nRETURN\n@UNRELATED\nRETURN\n",
            )],
        },
        &AnalyzerOptions::default(),
        &ExtensionRegistry::default(),
    );
    assert!(report.diagnostics.iter().all(|diagnostic| {
        diagnostic.severity != erabasic_analyzer::AnalyzerDiagnosticSeverity::Error
    }));
    let project = report.project.expect("unbounded graph remains analyzable");
    let unrelated = project
        .program
        .functions
        .iter()
        .find(|function| function.name == "UNRELATED")
        .unwrap();
    assert!(!unrelated.lines.is_empty());
}

#[test]
fn direct_method_calls_in_formatted_subexpressions_remain_reachable() {
    for statement in [
        "PRINTFORML {FORM_VALUE()}",
        "PRINTFORML {1, FORM_VALUE()}",
        "PRINTFORML \\@ FORM_VALUE() ? {FORM_VALUE()} # no \\@",
        "RESULTS = @\"{FORM_VALUE()}\"",
    ] {
        let report = analyze_project(
            AnalysisInput {
                project_data: empty_project(),
                sources: vec![source(
                    "formatted-reachability.erb",
                    &format!(
                        "@SYSTEM_TITLE\n{statement}\nRETURN\n@FORM_VALUE\n#FUNCTION\nRETURNF 1\n@UNUSED\n#FUNCTION\nRETURNF 2\n"
                    ),
                )],
            },
            &AnalyzerOptions::default(),
            &ExtensionRegistry::default(),
        );
        assert!(
            !report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.reference_level >= 2),
            "{statement}: {:#?}",
            report.diagnostics
        );
        let project = report.project.expect("formatted call project");
        let called = project
            .program
            .functions
            .iter()
            .find(|function| function.name == "FORM_VALUE")
            .unwrap();
        let unused = project
            .program
            .functions
            .iter()
            .find(|function| function.name == "UNUSED")
            .unwrap();
        assert!(!called.lines.is_empty(), "{statement} discarded its method");
        assert!(
            unused.lines.is_empty(),
            "static FORM unnecessarily disabled pruning"
        );
    }
}

#[test]
fn dynamic_method_spelling_in_literal_string_assignment_is_not_a_call() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "literal.erb",
                "@SYSTEM_TITLE\nRESULTS = GETMETH(\"UNUSED\", 0)\nRETURN\n@UNUSED\n#FUNCTION\nRETURNF 1\n",
            )],
        },
        &AnalyzerOptions::default(),
        &ExtensionRegistry::default(),
    );
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.reference_level >= 2),
        "{:#?}",
        report.diagnostics
    );
    let project = report.project.unwrap();
    assert!(
        project
            .program
            .functions
            .iter()
            .find(|function| function.name == "UNUSED")
            .unwrap()
            .lines
            .is_empty()
    );
}
