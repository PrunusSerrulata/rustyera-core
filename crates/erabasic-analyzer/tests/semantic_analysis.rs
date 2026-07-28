use erabasic_analyzer::{
    AnalysisInput, AnalyzerDiagnosticCode, AnalyzerOptions, ArgumentConstraint, CallableSignature,
    ExtensionRegistry, InstructionSignature, ProjectSource, SourcePayload, analyze_project,
};
use erabasic_csv::{
    CsvLoadOptions, FilePayload as CsvFilePayload, FrontendFile, ProjectFiles, load_project,
};
use erabasic_hir::{HirArgument, HirStatementKind, SemanticType};
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
fn varsize_is_a_load_time_constant_for_global_and_private_dimensions() {
    let sources = vec![
        source(
            "sizes.erh",
            "#DIMS LABELS = \"A\", \"B\", \"C\"\n\
             #DIM GRID, 2, 4\n\
             #DIM CONST LABEL_COUNT = VARSIZE(\"LABELS\")\n\
             #DIM CONST SECOND_LENGTH = VARSIZE(\"GRID\", 1)\n\
             #DIM VALUES, LABEL_COUNT\n",
        ),
        source(
            "sizes.erb",
            "@SYSTEM_TITLE\n\
             #DIM CFLAG_COPY, VARSIZE(\"CFLAG\")\n\
             RETURN\n",
        ),
    ];
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: sources.clone(),
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
    assert_eq!(
        project.data.schema.variable("VALUES").unwrap().dimensions,
        [3]
    );
    let second_length = project
        .program
        .variables
        .iter()
        .find(|variable| variable.name == "SECOND_LENGTH")
        .unwrap();
    assert_eq!(
        second_length.initial_values,
        [erabasic_hir::ConstantValue::Integer(4)]
    );
    let cflag_copy = project
        .program
        .variables
        .iter()
        .find(|variable| variable.name == "CFLAG_COPY")
        .unwrap();
    assert_eq!(
        cflag_copy.dimensions,
        project.data.schema.variable("CFLAG").unwrap().dimensions
    );

    let mut one_based = AnalyzerOptions::analysis_mode();
    one_based.varsize_dimension_is_one_based = true;
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources,
        },
        &one_based,
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
    let second_length = report
        .project
        .unwrap()
        .program
        .variables
        .into_iter()
        .find(|variable| variable.name == "SECOND_LENGTH")
        .unwrap();
    assert_eq!(
        second_length.initial_values,
        [erabasic_hir::ConstantValue::Integer(2)]
    );
}

#[test]
fn getnum_and_string_lengths_are_available_during_declaration_loading() {
    let mut project_data = empty_project();
    project_data
        .static_data
        .name_tables
        .get_mut(&erabasic_data::NameTableKind::Cflag)
        .unwrap()
        .lookup
        .insert("known".into(), 17);
    let report = analyze_project(
        AnalysisInput {
            project_data,
            sources: vec![source(
                "constants.erh",
                "#DIM LOOKUPS = GETNUM(CFLAG, \"known\"), GETNUM(CFLAG, \"missing\")\n\
                 #DIM CONST LEGACY_WIDTH = STRLENS(\"A界\")\n\
                 #DIM CONST UTF16_WIDTH = STRLENSU(\"😀\")\n\
                 #DIM CONST DEFAULT_COLOR = GETDEFCOLOR()\n",
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
    let variables = report.project.unwrap().program.variables;
    let lookups = variables
        .iter()
        .find(|variable| variable.name == "LOOKUPS")
        .unwrap();
    assert_eq!(
        lookups.initial_values,
        [
            erabasic_hir::ConstantValue::Integer(17),
            erabasic_hir::ConstantValue::Integer(-1)
        ]
    );
    let legacy_width = variables
        .iter()
        .find(|variable| variable.name == "LEGACY_WIDTH")
        .unwrap();
    assert_eq!(
        legacy_width.initial_values,
        [erabasic_hir::ConstantValue::Integer(3)]
    );
    let utf16_width = variables
        .iter()
        .find(|variable| variable.name == "UTF16_WIDTH")
        .unwrap();
    assert_eq!(
        utf16_width.initial_values,
        [erabasic_hir::ConstantValue::Integer(2)]
    );
    let default_color = variables
        .iter()
        .find(|variable| variable.name == "DEFAULT_COLOR")
        .unwrap();
    assert_eq!(
        default_color.initial_values,
        [erabasic_hir::ConstantValue::Integer(0x00c0_c0c0)]
    );
}

#[test]
fn unresolved_named_indices_in_dynamic_call_candidates_are_deferred() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "dynamic.erb",
                "@SYSTEM_TITLE\n\
                 CALLFORM \"OPTIONAL\"\n\
                 RETURN\n\
                 @OPTIONAL\n\
                 RESULT = CFLAG:LOCAL\n\
                 RESULT = CFLAG:not_in_csv\n\
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
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == AnalyzerDiagnosticCode::DeferredIndex
            && diagnostic.severity == erabasic_analyzer::AnalyzerDiagnosticSeverity::Warning
    }));
    let project = report.project.unwrap();
    let optional = project
        .program
        .functions
        .iter()
        .find(|function| function.name == "OPTIONAL")
        .unwrap();
    let HirStatementKind::Assignment {
        value: local_index, ..
    } = &optional.lines[0].kind
    else {
        panic!("expected local-index assignment");
    };
    let erabasic_hir::HirExprKind::Variable { place } = &local_index.kind else {
        panic!("expected indexed variable");
    };
    assert!(matches!(
        &place.indices[0].kind,
        erabasic_hir::HirExprKind::Variable { .. }
    ));
    let HirStatementKind::Assignment { value, .. } = &optional.lines[1].kind else {
        panic!("expected assignment");
    };
    let erabasic_hir::HirExprKind::Variable { place } = &value.kind else {
        panic!("expected indexed variable");
    };
    assert!(matches!(
        &place.indices[0].kind,
        erabasic_hir::HirExprKind::Call {
            target: erabasic_hir::CallTarget::Builtin { name },
            ..
        } if name == "__INDEXBYNAME"
    ));
}

#[test]
fn named_color_instructions_keep_the_unquoted_remainder() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "colors.erb",
                "@SYSTEM_TITLE\nSETCOLORBYNAME GRAY\nSETBGCOLORBYNAME navy\nRETURN\n",
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
            if matches!(arguments.as_slice(), [HirArgument::Raw(value)] if value == "GRAY")
    ));
    assert!(matches!(
        &function.lines[1].kind,
        HirStatementKind::Instruction { arguments, .. }
            if matches!(arguments.as_slice(), [HirArgument::Raw(value)] if value == "navy")
    ));
}

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
                include_str!("../../../tools/runtime-tester/fixture-reference/erb/oracle.erb"),
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
fn bitmap_cache_enable_requires_the_reference_argument() {
    for (call, accepted) in [
        ("BITMAP_CACHE_ENABLE()", false),
        ("BITMAP_CACHE_ENABLE(1)", true),
    ] {
        let report = analyze_project(
            AnalysisInput {
                project_data: empty_project(),
                sources: vec![source(
                    "bitmap-cache.erb",
                    &format!("@SYSTEM_TITLE\nRESULT = {call}\nRETURN\n"),
                )],
            },
            &AnalyzerOptions::analysis_mode(),
            &ExtensionRegistry::default(),
        );
        let has_argument_error = report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == AnalyzerDiagnosticCode::InvalidArgumentCount);
        assert_eq!(
            has_argument_error, !accepted,
            "{call}: {:#?}",
            report.diagnostics
        );
    }
}

#[test]
fn csv_name_tables_resolve_identifier_indices() {
    let project_data = load_project(
        &ProjectFiles {
            csv: vec![
                FrontendFile {
                    relative_path: "ABL.csv".into(),
                    payload: CsvFilePayload::Utf8("2,later\n".into()),
                },
                FrontendFile {
                    relative_path: "BASE.csv".into(),
                    payload: CsvFilePayload::Utf8("0,health\n".into()),
                },
            ],
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
                "@SYSTEM_TITLE\nRESULT = ABL:later\nRESULT = MAXBASE:health\nRETURN\n",
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
fn str_indices_resolve_through_strname_instead_of_initial_values() {
    let project_data = load_project(
        &ProjectFiles {
            csv: vec![
                FrontendFile {
                    relative_path: "STR.csv".into(),
                    payload: CsvFilePayload::Utf8("0,initial text\n".into()),
                },
                FrontendFile {
                    relative_path: "STRNAME.csv".into(),
                    payload: CsvFilePayload::Utf8("7,named_slot\n".into()),
                },
            ],
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
                "strname.erb",
                "@SYSTEM_TITLE\nSTR:named_slot = \"updated\"\nRETURN\n",
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
    let HirStatementKind::Assignment { target, .. } = &function.lines[0].kind else {
        panic!("expected STR assignment");
    };
    assert_eq!(
        target.indices[0].constant,
        Some(erabasic_hir::ConstantValue::Integer(7))
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
                "@SYSTEM_TITLE\n#DIMS KEYS, 4\nRESULTS '= MAP_GETKEYS(\"m\", KEYS, 1)\nRESULT = DT_COLUMN_NAMES(\"t\", KEYS)\nRETURN\n",
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
                "@SYSTEM_TITLE\nRESULTS '= MAP_GETKEYS(\"m\", \"not a place\", 1)\nRETURN\n",
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
