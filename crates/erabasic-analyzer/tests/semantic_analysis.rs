use erabasic_analyzer::{
    AnalysisInput, AnalyzerDiagnosticCode, AnalyzerOptions, ArgumentConstraint, CallableSignature,
    ExtensionRegistry, InstructionSignature, ProjectSource, SourcePayload, analyze_project,
};
use erabasic_csv::{
    CsvLoadOptions, FilePayload as CsvFilePayload, FrontendFile, ProjectFiles, load_project,
};
use erabasic_hir::{HirStatementKind, SemanticType};
use erabasic_parser::ArgumentStyle;

fn empty_project() -> erabasic_data::ProjectData {
    load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
        .data
        .expect("the default project schema should load")
}

fn source(path: &str, text: &str) -> ProjectSource {
    ProjectSource {
        relative_path: path.into(),
        payload: SourcePayload::Utf8(text.into()),
    }
}

#[test]
fn resolves_header_constants_variables_and_typed_expressions() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![
                source(
                    "vars.erh",
                    "#DIM CONST SIZE = 3\n#DIMS NAMES, SIZE = \"A\", \"B\", \"C\"\n",
                ),
                source(
                    "main.erb",
                    "@SYSTEM_TITLE\nRESULT = SIZE + 1\nIF RESULT\nPRINTFORM value={RESULT}\nENDIF\nRETURN\n",
                ),
            ],
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
    let project = report
        .project
        .expect("recoverable analysis should produce HIR");
    assert_eq!(
        project.data.schema.variable("SIZE").unwrap().dimensions,
        [1]
    );
    assert_eq!(
        project.data.schema.variable("NAMES").unwrap().dimensions,
        [3]
    );
    let function = &project.program.functions[0];
    let HirStatementKind::Assignment { value, .. } = &function.lines[0].kind else {
        panic!("first line should be an assignment");
    };
    assert_eq!(value.value_type, SemanticType::Integer);
    assert_eq!(
        value.constant,
        Some(erabasic_hir::ConstantValue::Integer(4))
    );
    assert!(
        function.control_flow.iter().any(|edge| {
            edge.kind == erabasic_hir::ControlFlowKind::Branch && edge.to.is_some()
        })
    );
}

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

#[test]
fn semantic_errors_recover_to_error_lines_and_stable_diagnostics() {
    let input = || AnalysisInput {
        project_data: empty_project(),
        sources: vec![source(
            "bad.erb",
            "@SYSTEM_TITLE\nRESULT = \"bad\"\nUNKNOWN 1\nBREAK\nRETURN\n",
        )],
    };
    let first = analyze_project(
        input(),
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );
    let second = analyze_project(
        input(),
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );

    assert!(first.project.is_some());
    assert!(
        first
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == AnalyzerDiagnosticCode::TypeMismatch)
    );
    assert!(
        first
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == AnalyzerDiagnosticCode::UnknownInstruction)
    );
    assert!(
        first
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == AnalyzerDiagnosticCode::InvalidControlFlow)
    );
    assert_eq!(
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&second).unwrap()
    );
}

#[test]
fn timed_input_wait_and_getkey_signatures_match_the_reference() {
    let valid = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "input.erb",
                "@SYSTEM_TITLE\nINPUT 7, 1, 0\nINPUTS \"D\", 1, 0\nONEINPUT 7, 1, 0\nONEINPUTS \"N\", 1, 0\nONEBINPUT 7, 1, 0\nONEBINPUTS \"N\", 1, 0\nTINPUT 100, 0, 1, \"timeout\", 0, 0\nTINPUTS 100, \"D\", 1, \"timeout\", 0, 0\nTONEINPUT 100, 0\nTONEINPUTS 100, \"N\"\nTWAIT 100, 0\nFORCEWAIT\nRESULT = GETKEY(65)\nRETURN\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );
    assert!(
        !valid
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.reference_level >= 2),
        "{:#?}",
        valid.diagnostics
    );

    let invalid = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "bad-input.erb",
                "@SYSTEM_TITLE\nTWAIT\nTINPUT 100, \"bad\"\nTONEINPUTS 100, 1\nFORCEWAIT 1\nRESULT = GETKEY()\nTINPUTNF 100, 0\nRETURN\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );
    assert!(
        invalid
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == AnalyzerDiagnosticCode::InvalidArgumentCount })
    );
    assert!(
        invalid
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == AnalyzerDiagnosticCode::TypeMismatch })
    );
    assert!(invalid.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == AnalyzerDiagnosticCode::UnknownInstruction
            && diagnostic.message.contains("TINPUTNF")
    }));
}

#[test]
fn extension_signatures_participate_without_executing_plugin_code() {
    let mut extensions = ExtensionRegistry::default();
    assert!(extensions.register_function(CallableSignature {
        name: "DOUBLE".into(),
        return_type: SemanticType::Integer,
        arguments: vec![ArgumentConstraint::Integer],
        minimum_arguments: 1,
        variadic: false,
        allow_omitted: false,
    }));
    assert!(extensions.register_instruction(InstructionSignature {
        name: "CUSTOM".into(),
        argument_style: ArgumentStyle::Expressions,
        arguments: vec![ArgumentConstraint::Integer],
        minimum_arguments: 1,
        variadic: false,
        allow_omitted: false,
    }));
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "extension.erb",
                "@SYSTEM_TITLE\nRESULT = DOUBLE(2)\nCUSTOM RESULT\nRETURN\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
        &extensions,
    );

    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.reference_level >= 2),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn reference_cli_project_fixture_has_compatible_semantic_shape() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "oracle.erb",
                include_str!("../../../tools/emuera-reference-cli/tests/fixture/erb/oracle.erb"),
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
    assert_eq!(
        program
            .functions
            .iter()
            .map(|function| function.name.as_str())
            .collect::<Vec<_>>(),
        [
            "SYSTEM_TITLE",
            "ORACLE_COMPAT",
            "ORACLE_DYNAMIC_1",
            "ORACLE_COMPAT_REST",
            "ORACLE_LIST_TARGET",
            "EVENTFIRST",
            "ORACLE_PRESENTATION",
            "ORACLE_TEST",
            "ORACLE_NATIVE",
            "ORACLE_INPUT",
            "ORACLE_REFLECTION",
            "ORACLE_MAP",
            "ORACLE_STRUCTURED",
            "ORACLE_COMPAT_12"
        ]
    );
    assert_eq!(program.functions[0].lines.len(), 14);
    assert_eq!(
        program.functions[0]
            .control_flow
            .iter()
            .filter(|edge| edge.kind == erabasic_hir::ControlFlowKind::Call)
            .count(),
        9
    );
}

#[test]
fn reference_analyze_line_assignment_projection_matches() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "assignment.erb",
                "@SYSTEM_TITLE\nRESULT = 9\nRETURN\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );
    let project = report.project.unwrap();
    let HirStatementKind::Assignment { value, .. } = &project.program.functions[0].lines[0].kind
    else {
        panic!("the reference SET line must lower to an assignment");
    };
    // The matching C# `analyzeLine` smoke response reports FunctionCode.SET,
    // SpSetArgument, Int64, and the constant value 9. This compact projection avoids
    // comparing reflection-only object identities from the oracle graph.
    let rust_projection = serde_json::json!({
        "functionCode": "SET",
        "argumentType": "SpSetArgument",
        "operandType": match value.value_type {
            SemanticType::Integer => "Int64",
            SemanticType::String => "String",
            SemanticType::Void => "Void",
            SemanticType::Error => "Error",
        },
        "constant": value.constant,
    });
    assert_eq!(
        rust_projection,
        serde_json::json!({
            "functionCode": "SET",
            "argumentType": "SpSetArgument",
            "operandType": "Int64",
            "constant": { "type": "integer", "value": 9 },
        })
    );
}

#[test]
fn reference_input_signature_observations_match_rust_diagnostics() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/reference-input-signatures.json"))
            .expect("reference input fixture should be valid JSON");
    assert_eq!(
        fixture["referenceCommit"],
        "26a35dc9334bb67590b96f7b8efbefbf199e391e"
    );
    for observation in fixture["observations"].as_array().unwrap() {
        let line = observation["source"].as_str().unwrap();
        let report = analyze_project(
            AnalysisInput {
                project_data: empty_project(),
                sources: vec![source(
                    "reference-input.erb",
                    &format!("@SYSTEM_TITLE\n{line}\nRETURN\n"),
                )],
            },
            &AnalyzerOptions::analysis_mode(),
            &ExtensionRegistry::default(),
        );
        let errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.reference_level >= 2)
            .collect();
        if observation["accepted"].as_bool().unwrap() {
            assert!(errors.is_empty(), "{line}: {errors:#?}");
            continue;
        }
        let expected = match observation["rustDiagnostic"].as_str().unwrap() {
            "invalid_argument_count" => AnalyzerDiagnosticCode::InvalidArgumentCount,
            "type_mismatch" => AnalyzerDiagnosticCode::TypeMismatch,
            "unknown_instruction" => AnalyzerDiagnosticCode::UnknownInstruction,
            value => panic!("unknown fixture diagnostic {value}"),
        };
        assert!(
            errors.iter().any(|diagnostic| diagnostic.code == expected),
            "{line}: expected {expected:?}, got {errors:#?}"
        );
    }
}

#[test]
fn csv_name_tables_resolve_identifier_indices() {
    let project_data = load_project(
        &ProjectFiles {
            csv: vec![FrontendFile {
                relative_path: "ABL.csv".into(),
                payload: CsvFilePayload::Utf8("2,later\n".into()),
            }],
            erb: Vec::new(),
        },
        &CsvLoadOptions::default(),
    )
    .data
    .unwrap();
    let report = analyze_project(
        AnalysisInput {
            project_data,
            sources: vec![source(
                "named-index.erb",
                "@SYSTEM_TITLE\nRESULT = ABL:later\nRETURN\n",
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
    let function = &report.project.unwrap().program.functions[0];
    let HirStatementKind::Assignment { value, .. } = &function.lines[0].kind else {
        panic!("expected assignment");
    };
    let erabasic_hir::HirExprKind::Variable { place } = &value.kind else {
        panic!("expected indexed variable");
    };
    assert_eq!(
        place.indices[0].constant,
        Some(erabasic_hir::ConstantValue::Integer(2))
    );
}

#[test]
fn user_character_data_requires_binary_save_configuration() {
    let input = AnalysisInput {
        project_data: empty_project(),
        sources: vec![source(
            "character.erh",
            "#DIM CHARADATA CUSTOM_CHARACTER, 10\n",
        )],
    };
    let text = analyze_project(
        input.clone(),
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );
    assert!(text.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == AnalyzerDiagnosticCode::InvalidDeclaration
            && diagnostic.message.contains("require binary saves")
    }));

    let mut binary_options = AnalyzerOptions::analysis_mode();
    binary_options.system_save_in_binary = true;
    let binary = analyze_project(input, &binary_options, &ExtensionRegistry::default());
    assert!(
        !binary
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.reference_level >= 2),
        "{:#?}",
        binary.diagnostics
    );
}

#[test]
fn structured_native_signatures_require_mutable_array_outputs() {
    let valid = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "structured.erb",
                "@SYSTEM_TITLE\n#DIMS KEYS, 4\nRESULTS = MAP_GETKEYS(\"m\", KEYS, 1)\nRESULT = DT_COLUMN_NAMES(\"t\", KEYS)\nRETURN\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );
    assert!(
        !valid
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.reference_level >= 2),
        "{:#?}",
        valid.diagnostics
    );

    let invalid = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "structured-invalid.erb",
                "@SYSTEM_TITLE\nRESULTS = MAP_GETKEYS(\"m\", \"not a place\", 1)\nRETURN\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );
    assert!(
        invalid
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == AnalyzerDiagnosticCode::InvalidAssignment }),
        "{:#?}",
        invalid.diagnostics
    );
}
