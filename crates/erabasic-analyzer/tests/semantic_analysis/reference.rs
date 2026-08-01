use super::*;

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
                include_str!("../../../../tools/runtime-tester/fixture-reference/erb/oracle.erb"),
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
            "ORACLE_HTML_POP",
            "ORACLE_PRESENTATION_23",
            "ORACLE_TEST",
            "ORACLE_NATIVE",
            "ORACLE_DYNAMIC_VARIABLES",
            "ORACLE_INPUT",
            "ORACLE_REFLECTION",
            "ORACLE_MAP",
            "ORACLE_STRUCTURED",
            "ORACLE_COMPAT_12",
            "ORACLE_PRESENTATION_3",
        ]
    );
    assert_eq!(program.functions[0].lines.len(), 16);
    assert_eq!(
        program.functions[0]
            .control_flow
            .iter()
            .filter(|edge| edge.kind == erabasic_hir::ControlFlowKind::Call)
            .count(),
        10
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
        serde_json::from_str(include_str!("../fixtures/reference-input-signatures.json"))
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
