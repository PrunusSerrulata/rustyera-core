use super::*;

#[test]
fn string_input_defaults_use_formatted_string_grammar() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "inputs.erb",
                "@SYSTEM_TITLE\nINPUTS 決定, 1, 0\nINPUTS %RESULTS%\nRETURN\n",
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
    assert!(matches!(
        &function.lines[0].kind,
        HirStatementKind::Instruction { arguments, .. }
            if matches!(
                arguments.as_slice(),
                [
                    HirArgument::Formatted(_),
                    HirArgument::Expression(erabasic_hir::HirExpr {
                        constant: Some(erabasic_hir::ConstantValue::Integer(1)),
                        ..
                    }),
                    HirArgument::Expression(erabasic_hir::HirExpr {
                        constant: Some(erabasic_hir::ConstantValue::Integer(0)),
                        ..
                    })
                ]
            )
    ));
    assert!(matches!(
        &function.lines[1].kind,
        HirStatementKind::Instruction { arguments, .. }
            if matches!(arguments.as_slice(), [HirArgument::Formatted(_)])
    ));
}

#[test]
fn legacy_string_methods_keep_their_distinct_statement_grammars() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "strings.erb",
                "@SYSTEM_TITLE\n\
                 #DIMS NAME\n\
                 ENCODETOUNI %NAME%\n\
                 RESULTS = %SUBSTRING(NAME, , 1)%\n\
                 SETVAR \"NAME\", \"updated\"\n\
                 RETURN\n",
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
    assert!(matches!(
        &function.lines[0].kind,
        HirStatementKind::Instruction { target, arguments }
            if target.name() == "ENCODETOUNI"
                && matches!(arguments.as_slice(), [HirArgument::Formatted(_)])
    ));
    assert!(matches!(
        &function.lines[2].kind,
        HirStatementKind::Instruction {
            target: erabasic_hir::InstructionTarget::BuiltinMethod {
                name,
                return_type: SemanticType::Integer,
            },
            arguments,
        } if name == "SETVAR"
            && matches!(
                arguments.as_slice(),
                [HirArgument::Expression(_), HirArgument::Expression(_)]
            )
    ));
}

#[test]
fn statement_varsize_preserves_the_array_reference() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "varsize.erb",
                "@SYSTEM_TITLE\n#DIM VALUES, 3\nVARSIZE VALUES\nRETURN\n",
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
    assert!(function.lines.iter().any(|line| {
        matches!(
            &line.kind,
            HirStatementKind::Instruction {
                target: erabasic_hir::InstructionTarget::Builtin(name),
                arguments,
            } if name == "VARSIZE"
                && matches!(arguments.as_slice(), [HirArgument::Place(_)])
        )
    }));
}

#[test]
fn configured_full_width_space_and_stray_carriage_return_can_prefix_lines() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "spacing.erb",
                "@SYSTEM_TITLE\n\u{3000}; translated comment\n\u{3000}\tPRINTL first\n\rPRINTL second\nRETURN\n",
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
    assert_eq!(report.project.unwrap().program.functions[0].lines.len(), 3);
}

#[test]
fn block_conditions_ignore_a_final_argument_separator() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "condition.erb",
                "@SYSTEM_TITLE\nIF RESULT,\nRESULT *= 2,\nPRINTL active\nENDIF\nRETURN\n",
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
}

#[test]
fn rand_accepts_reference_one_or_two_argument_forms_only() {
    let accepted = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "rand.erb",
                "@SYSTEM_TITLE\nRESULT = RAND(5)\nRESULT = RAND(2, 5)\nRESULT = RAND(, 5)\nRETURN\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );
    assert!(
        !accepted.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.severity,
            erabasic_analyzer::AnalyzerDiagnosticSeverity::Error
                | erabasic_analyzer::AnalyzerDiagnosticSeverity::Fatal
        )),
        "{:#?}",
        accepted.diagnostics
    );

    let rejected = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "rand.erb",
                "@SYSTEM_TITLE\nRESULT = RAND(1, 2, 3)\nRETURN\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );
    assert!(
        rejected
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == AnalyzerDiagnosticCode::InvalidArgumentCount })
    );
}

#[test]
fn omitted_for_start_counts_as_a_supplied_argument_slot() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "main.erb",
                "@SYSTEM_TITLE\n#DIM INDEX\nFOR INDEX, , 2\nNEXT\nRETURN\n",
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
}

#[test]
fn full_width_directive_separator_and_enumfiles_return_type_are_accepted() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "main.erb",
                "@SYSTEM_TITLE\n#DIM\u{3000}FILE_COUNT\nFILE_COUNT = ENUMFILES(\"data\", \"*.erb\")\nRETURN\n",
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
}

#[test]
fn accepts_reference_dim_comments_continuations_and_sign_bit_masks() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![
                source(
                    "vars.erh",
                    "#DIM COMMENTED; no separating whitespace\n#DIM CONST SIZE = 2,; a trailing comma is accepted\n#DIM CONST HIGH_BIT = 1p63\n#DIMS CONST PADDING = \" \" * 3\n{\n#DIMS CONST VALUES, 2 = @\"%UNICODE(0x2660)%\",\n\"C\"\n}\n",
                ),
                source(
                    "main.erb",
                    "@SYSTEM_TITLE\n#DIM LOCAL_VALUES, SIZE\nRETURN\n",
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
    let project = report.project.unwrap();
    assert!(project.data.schema.variable("COMMENTED").is_some());
    assert_eq!(
        project.data.schema.variable("VALUES").unwrap().dimensions,
        [2]
    );
    let padding = project
        .program
        .variables
        .iter()
        .find(|variable| variable.name == "PADDING")
        .unwrap();
    assert_eq!(
        padding.initial_values,
        [erabasic_hir::ConstantValue::String("   ".into())]
    );
    let local = project
        .program
        .variables
        .iter()
        .find(|variable| variable.name == "LOCAL_VALUES")
        .unwrap();
    assert_eq!(local.dimensions, [2]);
}

#[test]
fn plain_print_accepts_ascii_art_format_metacharacters() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "art.erb",
                "@SYSTEM_TITLE\nPRINTDL .:{ 50% \\ // } [raw]\nDEBUGPRINTFORML value={1}\nRETURN\n",
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
}

#[test]
fn character_variables_accept_implicit_and_explicit_character_indices() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "character.erb",
                "@SYSTEM_TITLE\nCFLAG:7 = 1\nCFLAG:2:7 = 2\nCALLNAME = \"target\"\nCALLNAME:2 = \"explicit\"\nRETURN\n",
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
}
