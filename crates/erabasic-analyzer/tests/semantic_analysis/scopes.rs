use super::*;

#[test]
fn registers_private_variables_and_function_parameter_places() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "main.erb",
                "@SYSTEM_TITLE(ARG:0 = 1)\n#DIM DYNAMIC TMP, 2\nFOR ARG:0, 0, 2\nTMP:ARG:0 = ARG:0\nNEXT\nRETURN\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
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
    let function = &project.program.functions[0];
    assert_eq!(function.parameters.len(), 1);
    let parameter = &function.parameters[0];
    assert_eq!(parameter.target.indices.len(), 1);
    assert!(project.program.variables.iter().any(|variable| {
        variable.name == "TMP" && variable.owner == Some(function.id) && !variable.static_lifetime
    }));
    assert!(
        function
            .control_flow
            .iter()
            .any(|edge| edge.kind == erabasic_hir::ControlFlowKind::LoopBack)
    );
}

#[test]
fn scoped_variable_instructions_register_and_initialize_frame_locals() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "main.erb",
                "@SYSTEM_TITLE(ARG:0 = 2)\nVARI WIDTH = ARG:0 + 1\nVARS TEXT = \"ok\"\nVARI ITEMS, 3\nVARI WIDTH = 7\nTEXT += \"!\"\nITEMS:2 = WIDTH\nRETURN\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
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
    let project = report.project.expect("scoped declarations should analyze");
    let function = &project.program.functions[0];
    let locals = project
        .program
        .variables
        .iter()
        .filter(|variable| variable.owner == Some(function.id))
        .map(|variable| (variable.name.as_str(), variable.dimensions.as_slice()))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(locals.get("WIDTH"), Some(&[1].as_slice()));
    assert_eq!(locals.get("TEXT"), Some(&[1].as_slice()));
    assert_eq!(locals.get("ITEMS"), Some(&[3].as_slice()));
    assert!(matches!(
        function.lines[0].kind,
        HirStatementKind::Assignment { .. }
    ));
    assert!(matches!(
        function.lines[1].kind,
        HirStatementKind::Assignment { .. }
    ));
    assert!(matches!(
        &function.lines[2].kind,
        HirStatementKind::Instruction { target, arguments }
            if target.name() == "VARI" && arguments.is_empty()
    ));
    assert!(matches!(
        function.lines[3].kind,
        HirStatementKind::Assignment { .. }
    ));
}

#[test]
fn scoped_declaration_keyword_can_be_a_private_assignment_target() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "main.erb",
                "@SYSTEM_TITLE\n#DIMS VARS\nVARS = CFLAG\nRETURN\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
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
    let project = report
        .project
        .expect("private VARS assignment should analyze");
    let function = &project.program.functions[0];
    assert!(
        project
            .program
            .variables
            .iter()
            .any(|variable| { variable.name == "VARS" && variable.owner == Some(function.id) })
    );
    assert!(matches!(
        function.lines[0].kind,
        HirStatementKind::Assignment { .. }
    ));
}

#[test]
fn private_dimensions_can_still_resolve_project_constants() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![
                source("constants.erh", "#DIM CONST PRIVATE_SIZE = 3\n"),
                source(
                    "main.erb",
                    "@SYSTEM_TITLE\n#DIM DYNAMIC TMP, PRIVATE_SIZE\nTMP:2 = 1\nRETURN\n@UNRELATED\nRETURN\n",
                ),
            ],
        },
        &AnalyzerOptions::analysis_mode(),
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
    let project = report.project.expect("analysis should produce HIR");
    let system_title = &project.program.functions[0];
    let private = project
        .program
        .variables
        .iter()
        .find(|variable| variable.name == "TMP" && variable.owner == Some(system_title.id))
        .expect("private variable should be registered");
    assert_eq!(private.dimensions, [3]);
}

#[test]
fn local_size_directives_resolve_constants_and_update_era_local_dimensions() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![
                source("constants.erh", "#DIM CONST LOCAL_BASE = 1200\n"),
                source(
                    "main.erb",
                    "@RESIZED\n#LOCALSIZE LOCAL_BASE * 2\n#LOCALSSIZE LOCAL_BASE / 6\nRETURN\n",
                ),
            ],
        },
        &AnalyzerOptions::analysis_mode(),
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
    let program = report.project.expect("analysis should produce HIR").program;
    let function = &program.functions[0];
    let dimensions = program
        .variables
        .iter()
        .filter(|variable| {
            variable.owner == Some(function.id)
                && variable.scope == erabasic_hir::VariableScope::EraFunction
        })
        .map(|variable| (variable.name.as_str(), variable.dimensions.as_slice()))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(dimensions.get("LOCAL"), Some(&[2400].as_slice()));
    assert_eq!(dimensions.get("LOCALS"), Some(&[200].as_slice()));
}

#[test]
fn repeated_local_size_directives_warn_and_keep_the_last_valid_size() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "main.erb",
                "@RESIZED\n#LOCALSIZE 1200\n#LOCALSIZE 1300\nRETURN\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );

    let duplicate_warnings = report
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.code == AnalyzerDiagnosticCode::InvalidDeclaration
                && diagnostic.severity == erabasic_analyzer::AnalyzerDiagnosticSeverity::Warning
                && diagnostic
                    .message
                    .contains("replaces an earlier size declaration")
        })
        .count();
    assert_eq!(duplicate_warnings, 1, "{:#?}", report.diagnostics);
    let program = report.project.expect("analysis should produce HIR").program;
    let function = &program.functions[0];
    let local = program
        .variables
        .iter()
        .find(|variable| variable.name == "LOCAL" && variable.owner == Some(function.id))
        .expect("function LOCAL");
    assert_eq!(local.dimensions, [1300]);
}

#[test]
fn event_functions_ignore_local_size_directives() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "events.erb",
                "@EVENTFIRST\n#LOCALSIZE 2000\nLOCAL = 1\nRETURN\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );

    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == AnalyzerDiagnosticCode::InvalidDeclaration
            && diagnostic.severity == erabasic_analyzer::AnalyzerDiagnosticSeverity::Warning
            && diagnostic
                .message
                .contains("event function ignores #LOCALSIZE")
    }));
    let program = report.project.expect("analysis should produce HIR").program;
    let function = &program.functions[0];
    let local = program
        .variables
        .iter()
        .find(|variable| variable.name == "LOCAL" && variable.owner == Some(function.id))
        .expect("event LOCAL");
    assert_eq!(local.dimensions, [1000]);
}

#[test]
fn prohibited_era_locals_stay_disabled_without_duplicate_size_warnings() {
    let project_data = load_project(
        &ProjectFiles {
            csv: vec![FrontendFile {
                relative_path: "VariableSize.csv".into(),
                payload: CsvFilePayload::Utf8("LOCAL,-1\nLOCALS,-1\n".into()),
            }],
            erb: Vec::new(),
        },
        &CsvLoadOptions::default(),
    )
    .data
    .expect("disabled local arrays should load");
    let report = analyze_project(
        AnalysisInput {
            project_data,
            sources: vec![source(
                "main.erb",
                "@RESIZED\n#LOCALSIZE 1200\n#LOCALSIZE 1300\n#LOCALSSIZE 120\n#LOCALSSIZE 130\nRETURN\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );

    let prohibited = report
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.code == AnalyzerDiagnosticCode::InvalidDeclaration
                && diagnostic.severity == erabasic_analyzer::AnalyzerDiagnosticSeverity::Error
                && diagnostic
                    .message
                    .contains("is prohibited by the project variable schema")
        })
        .count();
    let duplicate = report.diagnostics.iter().filter(|diagnostic| {
        diagnostic.code == AnalyzerDiagnosticCode::InvalidDeclaration
            && diagnostic
                .message
                .contains("replaces an earlier size declaration")
    });
    assert_eq!(prohibited, 4, "{:#?}", report.diagnostics);
    assert_eq!(duplicate.count(), 0, "{:#?}", report.diagnostics);
    let program = report.project.expect("analysis should produce HIR").program;
    let function = &program.functions[0];
    for name in ["LOCAL", "LOCALS"] {
        let variable = program
            .variables
            .iter()
            .find(|variable| variable.name == name && variable.owner == Some(function.id))
            .expect("disabled Era local");
        assert_eq!(variable.dimensions, [0]);
    }
}

#[test]
fn erb_parallel_parsing_preserves_source_order_and_shared_erh_macros() {
    let input = AnalysisInput {
        project_data: empty_project(),
        sources: vec![
            source("z.erb", "@ZED\nRESULT = SHARED_VALUE\nRETURN\n"),
            source("definitions.erh", "#DEFINE SHARED_VALUE 7\n"),
            source("a.erb", "@ALPHA\nRESULT = SHARED_VALUE\nRETURN\n"),
            source("m.erb", "@MIDDLE\nRESULT = SHARED_VALUE\nRETURN\n"),
        ],
    };
    let options = AnalyzerOptions::analysis_mode();
    let first = analyze_project(input.clone(), &options, &ExtensionRegistry::default());
    let second = analyze_project(input, &options, &ExtensionRegistry::default());

    assert_eq!(first, second, "parallel analysis must remain deterministic");
    let project = first.project.expect("analysis should produce HIR");
    let names = project
        .program
        .functions
        .iter()
        .map(|function| function.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["ZED", "ALPHA", "MIDDLE"]);
    for function in &project.program.functions {
        let HirStatementKind::Assignment { value, .. } = &function.lines[0].kind else {
            panic!("first line should be an assignment");
        };
        assert_eq!(
            value.constant,
            Some(erabasic_hir::ConstantValue::Integer(7))
        );
    }
}

#[test]
fn event_definitions_share_era_local_storage_and_keep_dispatch_attributes() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "events.erb",
                "@EVENTFIRST\n#PRI\nLOCAL:0 = 1\nRETURN\n@EVENTFIRST\n#LATER\n#SINGLE\nLOCAL:0 += 1\nRETURN\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
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
    let program = report.project.unwrap().program;
    assert_eq!(program.functions.len(), 2);
    assert!(program.functions[0].event_attributes.priority);
    assert!(program.functions[1].event_attributes.later);
    assert!(program.functions[1].event_attributes.single);
    assert_eq!(program.functions[0].definition_order, 0);
    assert_eq!(program.functions[1].definition_order, 1);

    let local_ids: Vec<_> = program
        .functions
        .iter()
        .map(|function| match &function.lines[0].kind {
            HirStatementKind::Assignment { target, .. } => target.variable,
            other => panic!("expected LOCAL assignment, found {other:?}"),
        })
        .collect();
    assert_eq!(local_ids[0], local_ids[1]);
    let local = program
        .variables
        .iter()
        .find(|variable| variable.id == local_ids[0])
        .expect("shared LOCAL definition");
    assert_eq!(local.scope, erabasic_hir::VariableScope::EraFunction);
}
