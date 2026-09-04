use super::*;

#[test]
fn csv_name_tables_resolve_identifier_indices() {
    let project_data = load_project(
        &ProjectFiles {
            csv: vec![
                FrontendFile {
                    source_path: None,
                    relative_path: "ABL.csv".into(),
                    payload: CsvFilePayload::Utf8("2,later\n".into()),
                },
                FrontendFile {
                    source_path: None,
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
fn getnum_folds_only_pure_builtin_name_table_lookups() {
    let mut project_data = empty_project();
    project_data
        .static_data
        .name_tables
        .get_mut(&erabasic_data::NameTableKind::Cflag)
        .unwrap()
        .lookup
        .insert("known".into(), 17);
    project_data
        .static_data
        .name_tables
        .get_mut(&erabasic_data::NameTableKind::Cdflag2)
        .unwrap()
        .lookup
        .insert("second".into(), 23);
    let report = analyze_project(
        AnalysisInput {
            project_data,
            sources: vec![
                source(
                    "constants.erh",
                    "#DIMS CONST LOOKUP_KEY = \"known\"\n#DIM CONST ONE = 1\n",
                ),
                source(
                    "getnum.erb",
                    "@SYSTEM_TITLE\n\
                     RESULT = GETNUM(CFLAG, LOOKUP_KEY + \"\")\n\
                     RESULT = GETNUM(CFLAG, \"known\", 0)\n\
                     RESULT = GETNUM(CFLAG, \"known\", 1)\n\
                     RESULT = GETNUM(CFLAG, \"missing\")\n\
                     RESULT = GETNUM(CDFLAG, \"second\", +(ONE + (1 ? 1 # 0)))\n\
                     RESULT = GETNUM(CFLAG, \"known\", -ONE)\n\
                     RESULT = GETNUM(CFLAG, RESULTS)\n\
                     RESULT = GETNUM(CFLAG:0, \"known\")\n\
                     #DIM DYNAMIC LOCAL_TABLE, 1\n\
                     RESULT = GETNUM(LOCAL_TABLE, \"known\")\n\
                     RETURN\n",
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
    let project = report.project.expect("GETNUM source should analyze");
    let values = project.program.functions[0]
        .lines
        .iter()
        .filter_map(|line| match &line.kind {
            HirStatementKind::Assignment { value, .. } => Some(value),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(values.len(), 9);
    for (index, expected) in [(0, 17), (1, 17), (2, 17), (3, -1), (4, 23), (5, -1)] {
        assert!(
            matches!(
                &values[index].kind,
                erabasic_hir::HirExprKind::Integer { value } if *value == expected
            ),
            "unexpected folded value at {index}: {:#?}",
            values[index]
        );
    }
    for value in values.iter().skip(6) {
        assert!(
            matches!(&value.kind, erabasic_hir::HirExprKind::Call { .. }),
            "dynamic, indexed, and local lookups must remain calls: {value:#?}"
        );
    }
}

#[test]
fn user_getnum_method_is_not_folded_as_the_builtin() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "shadow.erb",
                "@SYSTEM_TITLE\nRESULT = GETNUM(CFLAG, \"known\")\nRETURN\n\
                 @GETNUM(ARG, ARGS)\n#FUNCTION\nRETURNF 99\n",
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
    let project = report.project.expect("shadowing method should analyze");
    let HirStatementKind::Assignment { value, .. } = &project.program.functions[0].lines[0].kind
    else {
        panic!("expected assignment");
    };
    assert!(matches!(
        &value.kind,
        erabasic_hir::HirExprKind::Call {
            target: erabasic_hir::CallTarget::User { .. },
            ..
        }
    ));
}

#[test]
fn getnum_fold_does_not_bypass_argument_diagnostics() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "invalid-getnum.erb",
                "@SYSTEM_TITLE\nRESULT = GETNUM(CFLAG, \"missing\", 0, UNKNOWN_VALUE)\nRETURN\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );

    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == AnalyzerDiagnosticCode::InvalidArgumentCount })
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == AnalyzerDiagnosticCode::UnknownIdentifier })
    );
}

#[test]
fn str_indices_resolve_through_strname_instead_of_initial_values() {
    let project_data = load_project(
        &ProjectFiles {
            csv: vec![
                FrontendFile {
                    source_path: None,
                    relative_path: "STR.csv".into(),
                    payload: CsvFilePayload::Utf8("0,initial text\n".into()),
                },
                FrontendFile {
                    source_path: None,
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
