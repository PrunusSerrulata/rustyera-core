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
fn method_reachability_covers_formatted_values_widths_conditions_and_runtime_forms() {
    for statement in [
        "RESULT = GETMETH(\"CAN_MOVE_\" + TOSTR(1), 0)",
        "#DIM DYNAMIC VALUE\nVALUE = GETMETH(\"CAN_MOVE_\" + TOSTR(1), 0)",
        "#DIM DYNAMIC STR\nSTR = GETMETH(\"CAN_MOVE_\" + TOSTR(1), 0)",
        "RESULTS '= GETMETHS(\"SCOM_\" + TOSTR(1), \"\")",
        "RESULT = EXISTMETH(\"CAN_MOVE_\" + TOSTR(1))",
        "PRINTFORML {GETMETH(\"CAN_MOVE_\" + TOSTR(1), 0)}",
        "PRINTFORML {1, GETMETH(\"CAN_MOVE_\" + TOSTR(1), 0)}",
        "PRINTFORML \\@ GETMETH(\"CAN_MOVE_\" + TOSTR(1), 0) ? yes # no \\@",
        "RESULTS '= STRFORM(STR:0)",
    ] {
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
        let project = report.project.expect("reachable method project");
        for name in ["CAN_MOVE_1", "SCOM_1"] {
            let method = project
                .program
                .functions
                .iter()
                .find(|function| function.name == name)
                .expect("method definition");
            assert!(!method.lines.is_empty(), "{statement} discarded {name}");
        }
    }
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
