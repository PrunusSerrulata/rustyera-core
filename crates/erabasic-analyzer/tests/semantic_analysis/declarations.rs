use super::*;

#[test]
fn snake_constant_initializers_emit_warnings_without_losing_saturated_values() {
    let mut options = AnalyzerOptions::analysis_mode();
    options.compatibility = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![
                source(
                    "arithmetic.erh",
                    "#DIM CONST SATURATED = 9223372036854775807 + 1\n#DIM CONST ZERO_DIV = 9 / 0\n#DIM CONST WRAPPED = UNCHECKED_ADD(9223372036854775807, 1)\n#DIM CONST SKIPPED = 0 && (9 / 0)\n",
                ),
                source(
                    "arithmetic.erb",
                    "@SYSTEM_TITLE\n#DIM PRIVATE_VALUE = -(-9223372036854775807 - 1)\nRESULT = 9223372036854775807 + 1\nRETURN\n",
                ),
            ],
        },
        &options,
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
    let warnings: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            matches!(
                diagnostic.code,
                AnalyzerDiagnosticCode::IntegerOverflow
                    | AnalyzerDiagnosticCode::IntegerDivideByZero
            )
        })
        .collect();
    assert_eq!(warnings.len(), 3);
    assert!(warnings.iter().all(|diagnostic| diagnostic.severity
        == erabasic_analyzer::AnalyzerDiagnosticSeverity::Warning
        && diagnostic.source.is_some()));
    let project = report.project.unwrap();
    for (name, value) in [
        ("SATURATED", i64::MAX),
        ("ZERO_DIV", 0),
        ("WRAPPED", i64::MIN),
        ("PRIVATE_VALUE", i64::MAX),
        ("SKIPPED", 0),
    ] {
        let variable = project
            .program
            .variables
            .iter()
            .find(|variable| variable.name == name)
            .unwrap();
        assert_eq!(
            variable.initial_values,
            vec![erabasic_hir::ConstantValue::Integer(value)],
            "{name}"
        );
    }
    let statement = &project.program.functions[0].lines[0];
    let HirStatementKind::Assignment { value, .. } = &statement.kind else {
        panic!("expected assignment");
    };
    assert_eq!(
        value.constant, None,
        "runtime warning must not be folded away"
    );
}

#[test]
fn snake_project_constants_fold_power_named_colors_and_rename_values() {
    let mut project = empty_project();
    project
        .static_data
        .rename
        .insert("[[铃仙]]".into(), "42".into());
    let mut options = AnalyzerOptions::analysis_mode();
    options.compatibility = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let report = analyze_project(
        AnalysisInput {
            project_data: project,
            sources: vec![source(
                "constants.erh",
                "#DIM CONST C_G_END = POWER(2, 32) - 1\n#DIM CONST LIST_COLOR_DEFAULT = 0x44000000 + COLOR_FROMNAME(\"dimgray\")\n#DIM CONST RENAMED = [[铃仙]]\n",
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
        "{:#?}",
        report.diagnostics
    );
    let project = report.project.unwrap();
    for (name, expected) in [
        ("C_G_END", 4_294_967_295),
        ("LIST_COLOR_DEFAULT", 0x4469_6969),
        ("RENAMED", 42),
    ] {
        let variable = project
            .program
            .variables
            .iter()
            .find(|variable| variable.name == name)
            .unwrap();
        assert_eq!(
            variable.initial_values,
            vec![erabasic_hir::ConstantValue::Integer(expected)],
            "{name}"
        );
    }
}

#[test]
fn dynamic_private_initializer_is_lowered_at_function_entry() {
    let mut options = AnalyzerOptions::analysis_mode();
    options.compatibility = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "dynamic.erb",
                "@SYSTEM_TITLE\n#DIM DYNAMIC 行文字数 = STRLENSU(GETLINESTR(\"─\"))\nRESULT = 行文字数\nRETURN\n",
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
        "{:#?}",
        report.diagnostics
    );
    let function = &report.project.unwrap().program.functions[0];
    let HirStatementKind::Assignment { value, .. } = &function.lines[0].kind else {
        panic!("expected synthesized function-entry initializer");
    };
    assert!(value.constant.is_none());
    assert!(matches!(value.kind, erabasic_hir::HirExprKind::Call { .. }));
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
