use super::*;
use std::fmt::Write as _;

const ERAFL_TITLE_ERB: &str =
    include_str!("../../../../tools/runtime-tester/fixture-reference/erb/strform-title.erb");
const ERAFL_TITLE_ERH: &str =
    include_str!("../../../../tools/runtime-tester/fixture-reference/erb/strform-title.erh");
const ERAFL_TITLE_XML: &str =
    include_str!("../../../../tools/runtime-tester/fixture-reference/xml/CHARA_TITLE.xml");
const ERAFL_TITLE_FLAG_CSV: &str =
    include_str!("../../../../tools/runtime-tester/fixture-reference/csv/FLAG.CSV");

fn compile_erafl_title_fixture() -> BytecodeArtifact {
    let data = load_project(
        &ProjectFiles {
            csv: vec![FrontendFile {
                source_path: None,
                relative_path: "FLAG.CSV".into(),
                payload: CsvFilePayload::Utf8(ERAFL_TITLE_FLAG_CSV.into()),
            }],
            erb: Vec::new(),
        },
        &CsvLoadOptions::default(),
    )
    .data
    .expect("the real eraFL FLAG row should load");
    let mut options = AnalyzerOptions::analysis_mode();
    options.system_save_in_binary = true;
    let analysis = analyze_project(
        AnalysisInput {
            project_data: data,
            sources: vec![
                ProjectSource {
                    relative_path: "strform-title.erh".into(),
                    payload: SourcePayload::Utf8(ERAFL_TITLE_ERH.into()),
                },
                ProjectSource {
                    relative_path: "strform-title.erb".into(),
                    payload: SourcePayload::Utf8(ERAFL_TITLE_ERB.into()),
                },
            ],
        },
        &options,
        &ExtensionRegistry::default(),
    );
    assert!(
        analysis.project.is_some() && analysis.diagnostics.is_empty(),
        "archive-derived analysis: {:#?}",
        analysis.diagnostics
    );
    let compilation = compile_project(
        &analysis.project.unwrap(),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    assert!(
        compilation.artifact.is_some(),
        "archive-derived compilation: {:#?}",
        compilation.diagnostics
    );
    compilation.artifact.unwrap()
}

#[derive(Default)]
struct ArchiveFixtureHost {
    loadtext_calls: usize,
}

impl VmHost for ArchiveFixtureHost {
    fn call(&mut self, request: HostCallRequest) -> HostCallResult {
        if request.import.name.eq_ignore_ascii_case("LOADTEXT") {
            assert_eq!(
                request.arguments.first(),
                Some(&VmValue::String("XML/CHARA_TITLE.xml".into()))
            );
            self.loadtext_calls += 1;
            return HostCallResult::Ready(HostReady {
                value: Some(VmValue::String(ERAFL_TITLE_XML.into())),
                writes: Vec::new(),
            });
        }
        HostCallResult::Error(
            format!(
                "unexpected archive fixture host call: {}",
                request.import.name
            )
            .into(),
        )
    }
}

fn named_key(artifact: &BytecodeArtifact, name: &str) -> SymbolKey {
    artifact
        .globals
        .iter()
        .find(|global| global.name == name)
        .unwrap_or_else(|| panic!("missing fixture global {name}"))
        .key
}

fn completed_without_fault(report: &erabasic_vm::VmRunReport, fiber: FiberId) {
    assert_eq!(report.stop, erabasic_vm::VmRunStop::Idle);
    assert!(
        report.events.iter().any(
            |event| matches!(event, VmEvent::FiberCompleted { fiber: completed, value: None } if *completed == fiber)
        ),
        "{:#?}",
        report.events
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{:#?}",
        report.events
    );
}

#[test]
fn erafl_archive_fixture_matches_the_reference_cli_termination_and_watches() {
    assert!(
        ERAFL_TITLE_ERB.contains("IF !TOINT(STRFORM(TITLE_ACTIVATE_CONDITION))"),
        "the regression must retain CHARA_TITLE.ERB line 30"
    );
    assert!(
        ERAFL_TITLE_ERB.contains("RETURNF NO:(ARG:0) < MAX_FIXED_CHARA"),
        "the regression must retain eraFL's real IS_UNIQUE_CHARA"
    );
    assert_eq!(
        ERAFL_TITLE_FLAG_CSV.trim(),
        "500,領地評判_商業",
        "the regression must retain the real eraFL Flag.csv mapping"
    );
    assert!(
        ERAFL_TITLE_XML.contains("{FLAG:領地評判_商業 >= 150}"),
        "the regression must retain the archive's real title requirement"
    );
    assert!(
        ERAFL_TITLE_XML.contains("{FLAG:領地評判_商業 >= 300}"),
        "the regression must retain the next real merchant-title boundary"
    );
    assert!(
        ERAFL_TITLE_ERB.contains("IF !TOINT(STRFORM(TITLE_REQCONDITION))"),
        "the regression must retain the faulting CHARA_TITLE.ERB line 66"
    );

    let artifact = compile_erafl_title_fixture();
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "ORACLE_STRFORM_TITLE")
        .expect("ORACLE_STRFORM_TITLE")
        .key;
    let result = named_key(&artifact, "RESULT");
    let results = named_key(&artifact, "RESULTS");
    let mut host = ArchiveFixtureHost::default();
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());

    completed_without_fault(&report, fiber);
    assert_eq!(host.loadtext_calls, 1);
    let watches = [
        ("RESULT:80", vm.read_variable(result, &[80], None)),
        ("RESULT:81", vm.read_variable(result, &[81], None)),
        ("RESULT:82", vm.read_variable(result, &[82], None)),
        ("RESULT:83", vm.read_variable(result, &[83], None)),
        ("RESULT:84", vm.read_variable(result, &[84], None)),
        ("RESULT:85", vm.read_variable(result, &[85], None)),
        ("RESULTS:80", vm.read_variable(results, &[80], None)),
        ("RESULTS:81", vm.read_variable(results, &[81], None)),
    ];
    assert_eq!(
        watches,
        [
            ("RESULT:80", Ok(VmValue::Integer(0))),
            ("RESULT:81", Ok(VmValue::Integer(1))),
            ("RESULT:82", Ok(VmValue::Integer(0))),
            ("RESULT:83", Ok(VmValue::Integer(1))),
            ("RESULT:84", Ok(VmValue::Integer(0))),
            ("RESULT:85", Ok(VmValue::Integer(1))),
            ("RESULTS:80", Ok(VmValue::String("0".into()))),
            ("RESULTS:81", Ok(VmValue::String("1".into()))),
        ],
        "these are the same completed termination watches asserted by both reference CLI smoke scripts"
    );
}

fn run_entry(artifact: &BytecodeArtifact, config: VmConfig) -> (Vm, erabasic_vm::VmRunReport) {
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let mut vm = Vm::new(validated(artifact), config);
    let mut natives = NativeServiceRegistry::for_artifact(artifact);
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    (vm, report)
}

fn compile_with_header(header: &str, source: &str, options: &AnalyzerOptions) -> BytecodeArtifact {
    compile_with_header_and_compiler(header, source, options, &CompilerOptions::default())
}

fn compile_with_header_and_compiler(
    header: &str,
    source: &str,
    options: &AnalyzerOptions,
    compiler: &CompilerOptions,
) -> BytecodeArtifact {
    let analysis = analyze_project(
        AnalysisInput {
            project_data: project_data(),
            sources: vec![
                ProjectSource {
                    relative_path: "runtime-form.erh".into(),
                    payload: SourcePayload::Utf8(header.into()),
                },
                ProjectSource {
                    relative_path: "runtime-form.erb".into(),
                    payload: SourcePayload::Utf8(source.into()),
                },
            ],
        },
        options,
        &ExtensionRegistry::default(),
    );
    assert!(analysis.project.is_some(), "{:#?}", analysis.diagnostics);
    let compilation = compile_project(
        &analysis.project.unwrap(),
        compiler,
        &default_host_registry(),
        None,
    );
    assert!(
        compilation.artifact.is_some(),
        "{:#?}",
        compilation.diagnostics
    );
    compilation.artifact.unwrap()
}

const METHOD_FIXTURE_HEADER: &str =
    include_str!("../../../../tools/runtime-tester/fixture-snake-methods/erb/methods.erh");
const METHOD_FIXTURE_SOURCE: &str =
    include_str!("../../../../tools/runtime-tester/fixture-snake-methods/erb/methods.erb");

fn method_options(snake: bool) -> AnalyzerOptions {
    let mut options = AnalyzerOptions::analysis_mode();
    if snake {
        options.compatibility = erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        );
    }
    options
}

fn run_method_case(
    artifact: &BytecodeArtifact,
    name: &str,
    config: VmConfig,
) -> (Vm, erabasic_vm::VmRunReport) {
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == name)
        .expect("method fixture entry")
        .key;
    let mut vm = Vm::new(validated(artifact), config);
    let mut natives = NativeServiceRegistry::for_artifact_with_seed(artifact, 123_456);
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    (vm, report)
}

fn assert_method_watch(
    vm: &Vm,
    artifact: &BytecodeArtifact,
    name: &str,
    index: u64,
    expected: VmValue,
) {
    assert_eq!(
        vm.read_variable(named_key(artifact, name), &[index], None),
        Ok(expected),
        "{name}:{index}"
    );
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Keep the case table beside the shared dual-profile and optimization assertions."
)]
fn dynamic_methods_execute_lazily_with_defaults_ref_and_optimization_parity() {
    for snake in [false, true] {
        for optimization in [
            erabasic_compiler::OptimizationLevel::None,
            erabasic_compiler::OptimizationLevel::Basic,
        ] {
            let artifact = compile_with_header_and_compiler(
                METHOD_FIXTURE_HEADER,
                METHOD_FIXTURE_SOURCE,
                &method_options(snake),
                &CompilerOptions {
                    optimization,
                    ..CompilerOptions::default()
                },
            );
            for (case, result, trace, bodies) in [
                ("INTEGER", 42, 0, 1),
                ("PRESENT_SKIPS_FALLBACK", 23, 1234, 1),
                ("MISSING_ONLY_FALLBACK", 90, 19, 0),
                ("EXPLICIT_OMITTED_SLOT", 57, 0, 1),
                ("TRAILING_DEFAULTS", 56, 0, 1),
                ("I64_MIN_IS_VALUE", i64::MIN, 0, 1),
                ("WHOLE_ARRAY_REF_WRITEBACK", 11, 0, 1),
                ("WHOLE_ARRAY_REF_SKIPS_INDEX", 11, 0, 1),
                ("FINITE_RECURSION", 4, 0, 4),
                ("VALUE_CAPTURED_BEFORE_NEXT_ARGUMENT", 102, 4, 1),
                ("CAN_MOVE_DYNAMIC_PATTERN", 3, 0, 1),
                ("ODEKAKEMAP_DYNAMIC_PATTERN", 1, 0, 1),
                ("EVENT_INVISIBLE", 90, 19, 0),
                ("BUILTIN_INVISIBLE", 90, 19, 0),
                ("INTEGER_STATEMENT", 42, 0, 1),
            ] {
                let (vm, report) = run_method_case(
                    &artifact,
                    &format!("METHOD_CASE_{case}"),
                    VmConfig::default(),
                );
                assert!(
                    !report
                        .events
                        .iter()
                        .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
                    "{case}: {report:?}"
                );
                assert_eq!(report.stop, erabasic_vm::VmRunStop::Idle, "{case}");
                for (name, expected) in [
                    ("RESULT", result),
                    ("METHOD_TRACE", trace),
                    ("METHOD_BODY_COUNT", bodies),
                    ("METHOD_INDEX_COUNT", 0),
                ] {
                    assert_eq!(
                        vm.read_variable(named_key(&artifact, name), &[0], None),
                        Ok(VmValue::Integer(expected)),
                        "{case}: {name}"
                    );
                }
                if case.starts_with("WHOLE_ARRAY_REF") {
                    assert_method_watch(&vm, &artifact, "METHOD_VALUES", 0, VmValue::Integer(11));
                    assert_method_watch(
                        &vm,
                        &artifact,
                        "METHOD_WORDS",
                        1,
                        VmValue::String("changed".into()),
                    );
                }
                if case == "VALUE_CAPTURED_BEFORE_NEXT_ARGUMENT" {
                    assert_method_watch(&vm, &artifact, "METHOD_VALUES", 0, VmValue::Integer(99));
                }
            }
            for (case, result, trace, bodies) in [
                ("STRING", "zero", 0, 1),
                ("STRING_PRESENT_SKIPS_FALLBACK", "text:2", 124, 1),
                ("STRING_MISSING_ONLY_FALLBACK", "fallback", 19, 0),
                ("STRING_DYNAMIC_PATTERN", "pair:10:11", 0, 1),
                ("FORMATTED_EXPRESSION", "value=42", 1, 1),
                ("STRFORM_RUNTIME_EXPRESSION", "value=42", 0, 1),
                ("STRING_STATEMENT", "zero", 0, 1),
            ] {
                let (vm, report) = run_method_case(
                    &artifact,
                    &format!("METHOD_CASE_{case}"),
                    VmConfig::default(),
                );
                assert!(
                    !report
                        .events
                        .iter()
                        .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
                    "{case}: {report:?}"
                );
                assert_method_watch(&vm, &artifact, "RESULTS", 0, VmValue::String(result.into()));
                assert_method_watch(&vm, &artifact, "METHOD_TRACE", 0, VmValue::Integer(trace));
                assert_method_watch(
                    &vm,
                    &artifact,
                    "METHOD_BODY_COUNT",
                    0,
                    VmValue::Integer(bodies),
                );
            }
            let (vm, report) = run_method_case(
                &artifact,
                "METHOD_CASE_EXIST_ZERO_ARGUMENT_RESOLUTION",
                VmConfig::default(),
            );
            assert!(
                !report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
                "{report:?}"
            );
            for (index, value) in [1, 2, 0, 0, 0, 1, 0, 0, 0].into_iter().enumerate() {
                assert_method_watch(
                    &vm,
                    &artifact,
                    "RESULT",
                    u64::try_from(index).unwrap(),
                    VmValue::Integer(value),
                );
            }
            assert_method_watch(&vm, &artifact, "METHOD_BODY_COUNT", 0, VmValue::Integer(0));
        }
    }
}

#[test]
fn dynamic_method_signature_faults_preserve_only_name_evaluation_side_effects() {
    for snake in [false, true] {
        let artifact = compile_with_header(
            METHOD_FIXTURE_HEADER,
            METHOD_FIXTURE_SOURCE,
            &method_options(snake),
        );
        for (case, code) in [
            ("MISSING_NO_FALLBACK", VmFaultCode::MissingSymbol),
            ("MISSING_OMITTED_FALLBACK", VmFaultCode::MissingSymbol),
            ("ORDINARY_FUNCTION", VmFaultCode::TypeMismatch),
            ("WRONG_INTEGER_RETURN", VmFaultCode::TypeMismatch),
            ("WRONG_STRING_RETURN", VmFaultCode::TypeMismatch),
            ("WRONG_ARGUMENT_TYPE", VmFaultCode::TypeMismatch),
            ("MISSING_REQUIRED_ARGUMENT", VmFaultCode::TypeMismatch),
            ("MISSING_REF", VmFaultCode::TypeMismatch),
            ("EXPRESSION_NOT_REF", VmFaultCode::TypeMismatch),
            ("WRONG_REF_TYPE", VmFaultCode::TypeMismatch),
            ("WRONG_REF_RANK", VmFaultCode::TypeMismatch),
            ("EXTRA_ARGUMENT_POLICY", VmFaultCode::TypeMismatch),
        ] {
            let (vm, report) = run_method_case(
                &artifact,
                &format!("METHOD_CASE_{case}"),
                VmConfig::default(),
            );
            if snake && case == "EXTRA_ARGUMENT_POLICY" {
                assert!(
                    report
                        .events
                        .iter()
                        .any(|event| { matches!(event, VmEvent::FiberCompleted { .. }) }),
                    "{report:?}"
                );
                assert!(
                    !report
                        .events
                        .iter()
                        .any(|event| { matches!(event, VmEvent::FiberFaulted { .. }) }),
                    "{report:?}"
                );
                assert_eq!(report.events.iter().filter(|event| {
                    matches!(event, VmEvent::Diagnostic { code, .. } if code == "compat.call.excess_arguments")
                }).count(), 1);
                for (name, expected) in [
                    ("RESULT", 2),
                    ("METHOD_TRACE", 12),
                    ("METHOD_BODY_COUNT", 1),
                    ("METHOD_INDEX_COUNT", 0),
                    ("METHOD_EVENT_COUNT", 0),
                ] {
                    assert_method_watch(&vm, &artifact, name, 0, VmValue::Integer(expected));
                }
                continue;
            }
            assert_eq!(take_fault(report).code, code, "{case}");
            // Faulted runtime-tester sessions cannot expose debug watches: inspect VM storage directly.
            for (name, expected) in [
                ("METHOD_TRACE", 1),
                ("METHOD_BODY_COUNT", 0),
                ("METHOD_INDEX_COUNT", 0),
                ("METHOD_EVENT_COUNT", 0),
            ] {
                assert_method_watch(&vm, &artifact, name, 0, VmValue::Integer(expected));
            }
            assert_method_watch(&vm, &artifact, "METHOD_VALUES", 0, VmValue::Integer(10));
            assert_method_watch(
                &vm,
                &artifact,
                "METHOD_WORDS",
                1,
                VmValue::String("unchanged".into()),
            );
        }
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Keep the typed expression matrix beside its shared side-effect assertions."
)]
fn runtime_form_dynamic_methods_share_lazy_resolution_and_capture_semantics() {
    let expressions = [
        (
            r#"GETMETH(METHOD_NAME("METHOD_PAIR"), METHOD_FALLBACK(), METHOD_ARG(2), METHOD_ARG(3))"#,
            "23",
            1234,
            1,
        ),
        (
            r#"GETMETH(METHOD_NAME("MISSING"), METHOD_FALLBACK(), METHOD_ARG(2))"#,
            "90",
            19,
            0,
        ),
        (r#"GETMETH("METHOD_DEFAULT", , , 7)"#, "57", 0, 1),
        (
            r#"GETMETH("METHOD_ECHO", , (-9223372036854775807 - 1))"#,
            "-9223372036854775808",
            0,
            1,
        ),
        (
            r#"GETMETH("METHOD_REF", , METHOD_VALUES:METHOD_INDEX(), METHOD_WORDS:METHOD_INDEX())"#,
            "11",
            0,
            1,
        ),
        (
            r#"GETMETH("METHOD_PAIR", , METHOD_VALUES:0, METHOD_MUTATE_VALUE())"#,
            "102",
            4,
            1,
        ),
        (
            r#"GETMETH("METHOD_PAIR", , GETMETH("METHOD_ECHO", , 2), 3)"#,
            "23",
            4,
            2,
        ),
        (r#"EXISTMETH("METHOD_DEFAULT")"#, "1", 0, 0),
        (
            r#"GETMETH("METHOD_PAIR", , "a" < "b", (1 ? 2 # 3))"#,
            "12",
            4,
            1,
        ),
        (r#"GETMETHS("FORM_STRING_ECHO", , "a" + "b")"#, "ab", 0, 1),
        (r#"GETMETHS("FORM_STRING_ECHO", , "a" * 2)"#, "aa", 0, 1),
        (r#"GETMETHS("FORM_STRING_ECHO", , 2 * "b")"#, "bb", 0, 1),
        (
            r#"GETMETHS("FORM_STRING_ECHO", , (0 ? "a" # "b"))"#,
            "b",
            0,
            1,
        ),
    ];
    let mut source = METHOD_FIXTURE_SOURCE.to_owned()
        + "\n@FORM_STRING_ECHO(TEXT)\n#FUNCTIONS\n#DIMS DYNAMIC TEXT\nMETHOD_BODY_COUNT += 1\nRETURNF TEXT\n";
    for (index, (expression, _, _, _)) in expressions.iter().enumerate() {
        let escaped = expression.replace('"', "\\\"");
        let form = if expression.starts_with("GETMETHS") {
            format!("%{escaped}%")
        } else {
            format!("{{{escaped}}}")
        };
        write!(
            source,
            "\n@FORM_METHOD_{index}\nCALL METHOD_RESET\nRESULTS:0 '= STRFORM(\"{form}\")\nRETURN\n"
        )
        .unwrap();
    }
    let artifact = compile_with_header(METHOD_FIXTURE_HEADER, &source, &method_options(true));
    for (index, (_, output, trace, bodies)) in expressions.iter().enumerate() {
        let (vm, report) = run_method_case(
            &artifact,
            &format!("FORM_METHOD_{index}"),
            VmConfig::default(),
        );
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{index}: {report:?}"
        );
        assert_method_watch(
            &vm,
            &artifact,
            "RESULTS",
            0,
            VmValue::String((*output).into()),
        );
        assert_method_watch(&vm, &artifact, "METHOD_TRACE", 0, VmValue::Integer(*trace));
        assert_method_watch(
            &vm,
            &artifact,
            "METHOD_BODY_COUNT",
            0,
            VmValue::Integer(*bodies),
        );
        assert_method_watch(&vm, &artifact, "METHOD_INDEX_COUNT", 0, VmValue::Integer(0));
    }
}

#[test]
fn runtime_form_method_faults_do_not_evaluate_fallback_actuals_or_ref_indices() {
    let expressions = [
        r#"GETMETH(METHOD_NAME("METHOD_ORDINARY"), METHOD_FALLBACK(), METHOD_ARG(2))"#,
        r#"GETMETH(METHOD_NAME("METHOD_TEXT"), METHOD_FALLBACK(), METHOD_ARG(2))"#,
        r#"GETMETH(METHOD_NAME("METHOD_ECHO"), METHOD_FALLBACK(), METHOD_STRING_ARG())"#,
        r#"GETMETH(METHOD_NAME("METHOD_REQUIRED"), METHOD_FALLBACK())"#,
        r#"GETMETH(METHOD_NAME("METHOD_REF_INT"), METHOD_FALLBACK(), METHOD_MATRIX:METHOD_INDEX():0)"#,
        r#"GETMETH(METHOD_NAME("METHOD_REF_INT"), METHOD_FALLBACK(), METHOD_WORDS:METHOD_INDEX())"#,
        r#"GETMETH(METHOD_NAME("METHOD_PAIR"), METHOD_FALLBACK(), METHOD_ARG(2), "x" - 1)"#,
        r#"GETMETH(METHOD_NAME("METHOD_PAIR"), METHOD_FALLBACK(), METHOD_ARG(2), -"x")"#,
        r#"GETMETH(METHOD_NAME("METHOD_PAIR"), METHOD_FALLBACK(), METHOD_ARG(2), ("x" ? 1 # 2))"#,
        r#"GETMETH(METHOD_NAME("METHOD_PAIR"), METHOD_FALLBACK(), METHOD_ARG(2), "x" == 1)"#,
        r#"GETMETH(METHOD_NAME("METHOD_PAIR"), METHOD_FALLBACK(), METHOD_ARG(2), METHOD_ECHO("x" - 1))"#,
    ];
    let mut source = METHOD_FIXTURE_SOURCE.to_owned();
    for (index, expression) in expressions.iter().enumerate() {
        let escaped = expression.replace('"', "\\\"");
        write!(source, "\n@FORM_FAULT_{index}\nCALL METHOD_RESET\nRESULTS:0 '= STRFORM(\"{{{escaped}}}\")\nRETURN\n").unwrap();
    }
    let artifact = compile_with_header(METHOD_FIXTURE_HEADER, &source, &method_options(true));
    for index in 0..expressions.len() {
        let (vm, report) = run_method_case(
            &artifact,
            &format!("FORM_FAULT_{index}"),
            VmConfig::default(),
        );
        assert_eq!(
            take_fault(report).code,
            VmFaultCode::TypeMismatch,
            "{index}"
        );
        for (name, expected) in [
            // Invalid source types fail before evaluating the dynamic target.
            // The first six cases have valid source types and fail only when
            // the computed target's runtime signature is bound.
            ("METHOD_TRACE", i64::from(index < 6)),
            ("METHOD_BODY_COUNT", 0),
            ("METHOD_INDEX_COUNT", 0),
        ] {
            assert_method_watch(&vm, &artifact, name, 0, VmValue::Integer(expected));
        }
    }
}

#[test]
fn dynamic_method_recursion_stops_at_vm_call_depth() {
    let artifact = compile_with_header(
        METHOD_FIXTURE_HEADER,
        METHOD_FIXTURE_SOURCE,
        &method_options(true),
    );
    let (vm, report) = run_method_case(
        &artifact,
        "METHOD_CASE_FINITE_RECURSION",
        VmConfig {
            maximum_call_depth: 3,
            ..VmConfig::default()
        },
    );
    assert_eq!(take_fault(report).code, VmFaultCode::ResourceLimit);
    assert_method_watch(&vm, &artifact, "METHOD_BODY_COUNT", 0, VmValue::Integer(2));
}

#[test]
fn dynamic_method_ref_forwarding_resolves_the_original_array_owner() {
    let source = METHOD_FIXTURE_SOURCE.to_owned()
        + r#"
@FORWARD_ENTRY
CALL METHOD_RESET
RESULT:0 = GETMETH("FORWARD_ARRAYS", , METHOD_VALUES, METHOD_WORDS)
RETURN RESULT:0
@FORWARD_ARRAYS(NUMBERS, TEXTS)
#FUNCTION
#DIM REF NUMBERS
#DIMS REF TEXTS
RETURNF GETMETH("METHOD_REF", , NUMBERS:METHOD_INDEX(), TEXTS:METHOD_INDEX())
@NESTED_ENTRY
CALL METHOD_RESET
RESULT:0 = GETMETH("METHOD_PAIR", , GETMETH("METHOD_ECHO", , 2), 3)
RETURN RESULT:0
"#;
    let artifact = compile_with_header(METHOD_FIXTURE_HEADER, &source, &method_options(true));
    let (vm, report) = run_method_case(&artifact, "FORWARD_ENTRY", VmConfig::default());
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_method_watch(&vm, &artifact, "RESULT", 0, VmValue::Integer(11));
    assert_method_watch(&vm, &artifact, "METHOD_VALUES", 0, VmValue::Integer(11));
    assert_method_watch(
        &vm,
        &artifact,
        "METHOD_WORDS",
        1,
        VmValue::String("changed".into()),
    );
    assert_method_watch(&vm, &artifact, "METHOD_INDEX_COUNT", 0, VmValue::Integer(0));
    let (vm, report) = run_method_case(&artifact, "NESTED_ENTRY", VmConfig::default());
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_method_watch(&vm, &artifact, "RESULT", 0, VmValue::Integer(23));
    assert_method_watch(&vm, &artifact, "METHOD_BODY_COUNT", 0, VmValue::Integer(2));
}

#[test]
fn dynamic_methods_keep_existing_optional_and_conversion_policy() {
    let mut options = method_options(true);
    options.compatible_function_argument_optional = true;
    options.compatible_function_argument_auto_convert = true;
    let artifact = compile_with_header(
        "",
        r#"@SYSTEM_TITLE
RESULTS:0 '= GETMETHS("POLICY_TEXT", , 123)
RESULTS:1 '= GETMETHS("POLICY_TEXT")
RESULT:0 = EXISTMETH("POLICY_TEXT")
RETURN RESULT:0
@POLICY_TEXT(TEXT)
#FUNCTIONS
#DIMS DYNAMIC TEXT
RETURNF TEXT
"#,
        &options,
    );
    let (vm, report) = run_method_case(&artifact, "SYSTEM_TITLE", VmConfig::default());
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_method_watch(&vm, &artifact, "RESULTS", 0, VmValue::String("123".into()));
    assert_method_watch(&vm, &artifact, "RESULTS", 1, VmValue::String(String::new()));
    assert_method_watch(&vm, &artifact, "RESULT", 0, VmValue::Integer(2));
}

#[test]
fn dynamic_method_and_runtime_form_lookups_stay_in_the_callers_generation() {
    let source = r#"@METHOD_ENTRY
#FUNCTION
RETURNF GETMETH("VALUE_METHOD") + EXISTMETH("NEW_METHOD") * 100
@FORM_ENTRY
#FUNCTIONS
RETURNF STRFORM("{GETMETH(\"VALUE_METHOD\")}:{EXISTMETH(\"NEW_METHOD\")}")
@VALUE_METHOD
#FUNCTION
RETURNF 1
"#;
    let updated =
        source.replace("RETURNF 1", "RETURNF 2") + "\n@NEW_METHOD\n#FUNCTION\nRETURNF 7\n";
    let base = compile_source_with_options(source, &method_options(true));
    let target = compile_source_with_options(&updated, &method_options(true));
    let patch = create_patch(&base, &target);
    for (name, pause_opcode, old_result, new_result) in [
        (
            "METHOD_ENTRY",
            Opcode::ResolveUserCall,
            VmValue::Integer(1),
            VmValue::Integer(102),
        ),
        (
            "FORM_ENTRY",
            Opcode::CallNative,
            VmValue::String("1:0".into()),
            VmValue::String("2:1".into()),
        ),
    ] {
        let entry = base
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap();
        let instructions = entry
            .code
            .iter()
            .position(|instruction| instruction.opcode == pause_opcode as u16)
            .unwrap()
            + 1;
        let mut vm = Vm::new(validated(&base), VmConfig::default());
        let mut natives = NativeServiceRegistry::for_artifact_with_seed(&base, 123_456);
        let mut host = ReadyHost::default();
        let old = vm.spawn_entry(entry.key, Vec::new()).unwrap();
        let report = vm.run_slice(
            &mut host,
            &mut natives,
            RunBudget {
                maximum_instructions: u64::try_from(instructions).unwrap(),
                maximum_host_calls: 0,
                fiber_quantum: 4096,
            },
        );
        assert_eq!(report.stop, erabasic_vm::VmRunStop::BudgetExhausted);
        vm.prepare_hot_reload(&patch, &ValidationContext::for_artifact(&target))
            .unwrap();
        vm.commit_hot_reload().unwrap();
        let new = vm.spawn_entry(entry.key, Vec::new()).unwrap();
        let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{name}: {report:?}"
        );
        assert_eq!(
            vm.fiber_status(old),
            Some(FiberStatus::Completed(Some(old_result)))
        );
        assert_eq!(
            vm.fiber_status(new),
            Some(FiberStatus::Completed(Some(new_result)))
        );
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Keep both serialized continuation shapes and their corruption matrix in one scenario."
)]
fn suspended_dynamic_method_snapshots_validate_origin_generation_slots_and_form_bindings() {
    let source = r#"@SYSTEM_TITLE
RESULT:0 = GETMETH("SNAP_PAIR", , 2, SNAP_INPUT())
RETURN RESULT:0
@FORM_TITLE
RESULTS:0 '= STRFORM("{GETMETH(\"SNAP_PAIR\", , 2, SNAP_INPUT())}")
RETURN
@SNAP_PAIR(LEFT, RIGHT)
#FUNCTION
#DIM DYNAMIC LEFT
#DIM DYNAMIC RIGHT
RETURNF LEFT * 10 + RIGHT
@SNAP_INPUT
#FUNCTION
INPUT
RETURNF RESULT
"#;
    let artifact = compile_source_with_options(source, &method_options(true));
    for entry_name in ["SYSTEM_TITLE", "FORM_TITLE"] {
        let entry = artifact
            .functions
            .iter()
            .find(|function| function.name == entry_name)
            .unwrap()
            .key;
        let mut natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 123_456);
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
        let mut host = PendingHost {
            stability: HostWaitStability::StableInput,
            rebound: Vec::new(),
        };
        let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
        let request = report
            .events
            .iter()
            .find_map(|event| match event {
                VmEvent::HostPending { request, .. } => Some(*request),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{entry_name}: expected stable input, got {report:?}"));
        let snapshot = vm.snapshot(&natives).unwrap();
        let json = serde_json::to_value(&snapshot).unwrap();
        let mut cancelled = vm.clone();
        cancelled.cancel_fiber(fiber).unwrap();
        assert!(cancelled.resume_host(request, HostReady::empty()).is_err());
        let cancelled = serde_json::to_value(cancelled.snapshot(&natives).unwrap()).unwrap();
        assert!(
            cancelled["fibers"][fiber.0.to_string()]["frames"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        for corruption in ["origin", "generation", "slot", "target", "capture"] {
            let mut corrupted = json.clone();
            let frame = &mut corrupted["fibers"][fiber.0.to_string()]["frames"][0];
            if entry_name == "SYSTEM_TITLE" {
                let pending = &mut frame["user_calls"][0];
                match corruption {
                    "origin" => pending["resolve"] = serde_json::json!(usize::MAX),
                    "generation" => pending["call"]["generation"] = serde_json::json!(999),
                    "slot" => pending["next_slot"] = serde_json::json!(999),
                    "target" => {
                        pending["call"]["function"] = serde_json::to_value(entry).unwrap();
                    }
                    "capture" => {
                        pending["captured"][0] =
                            serde_json::to_value(VmValue::String("forged".into())).unwrap();
                    }
                    _ => unreachable!(),
                }
            } else {
                let continuation = &mut frame["runtime_form"];
                let call = continuation["work"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find_map(|task| task.get_mut("CaptureMethodArgument"))
                    .expect("suspended STRFORM capture");
                match corruption {
                    "origin" => call["call"]["function"] = serde_json::to_value(entry).unwrap(),
                    "generation" => call["call"]["generation"] = serde_json::json!(999),
                    "slot" => call["next_slot"] = serde_json::json!(999),
                    "target" => call["call"]["bindings"] = serde_json::json!([]),
                    "capture" => {
                        call["captured"][0] =
                            serde_json::to_value(VmValue::String("forged".into())).unwrap();
                    }
                    _ => unreachable!(),
                }
            }
            let corrupted: VmSnapshot = serde_json::from_value(corrupted).unwrap();
            let mut rejected_host = PendingHost {
                stability: HostWaitStability::StableInput,
                rebound: Vec::new(),
            };
            let before = vm.encode_snapshot(&natives).unwrap();
            assert!(
                Vm::restore_snapshot(
                    validated(&artifact),
                    VmConfig::default(),
                    corrupted,
                    &mut rejected_host,
                    &mut natives
                )
                .is_err(),
                "{entry_name}/{corruption}"
            );
            assert!(
                rejected_host.rebound.is_empty(),
                "invalid method state must be rejected before host rebind"
            );
            assert_eq!(vm.encode_snapshot(&natives).unwrap(), before);
        }
        let mut restored = Vm::restore_snapshot(
            validated(&artifact),
            VmConfig::default(),
            snapshot,
            &mut host,
            &mut natives,
        )
        .unwrap();
        restored
            .write_variable(
                named_key(&artifact, "RESULT"),
                &[0],
                None,
                VmValue::Integer(3),
            )
            .unwrap();
        restored.resume_host(request, HostReady::empty()).unwrap();
        let report = restored.run_slice(
            &mut ReadyHost::default(),
            &mut natives,
            RunBudget::default(),
        );
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{entry_name}: {report:?}"
        );
        if entry_name == "SYSTEM_TITLE" {
            assert_method_watch(&restored, &artifact, "RESULT", 0, VmValue::Integer(23));
        } else {
            assert_method_watch(
                &restored,
                &artifact,
                "RESULTS",
                0,
                VmValue::String("23".into()),
            );
        }
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Keep the suspended REF fixture, corruption matrix and forwarding restoration assertions together."
)]
fn active_ref_method_snapshots_restore_local_arrays_and_reject_invalid_aliases() {
    let source = METHOD_FIXTURE_SOURCE.to_owned()
        + r#"
@REF_SNAPSHOT_TITLE
#DIM DYNAMIC LOCAL_NUMBERS, 3
#DIMS DYNAMIC LOCAL_WORDS, 3
LOCAL_NUMBERS:0 = 10
LOCAL_WORDS:1 '= "before"
RESULT:0 = GETMETH("SNAP_FORWARD", , LOCAL_NUMBERS, LOCAL_WORDS)
RESULT:1 = LOCAL_NUMBERS:0
RESULTS:0 '= LOCAL_WORDS:1
RETURN RESULT:0
@SNAP_FORWARD(NUMBERS, TEXTS)
#FUNCTION
#DIM REF NUMBERS
#DIMS REF TEXTS
RETURNF GETMETH("SNAP_WAIT", , NUMBERS, TEXTS)
@SNAP_WAIT(NUMBERS, TEXTS)
#FUNCTION
#DIM REF NUMBERS
#DIMS REF TEXTS
INPUT
NUMBERS:0 += 1
TEXTS:1 '= "restored"
RETURNF NUMBERS:0
"#;
    let source = source
        + r#"
@FORM_REF_SNAPSHOT_TITLE
#DIM DYNAMIC LOCAL_NUMBERS, 3
#DIMS DYNAMIC LOCAL_WORDS, 3
LOCAL_NUMBERS:0 = 10
LOCAL_WORDS:1 '= "before"
RESULT:0 = TOINT(STRFORM("{GETMETH(\"SNAP_FORWARD\", , LOCAL_NUMBERS, LOCAL_WORDS)}"))
RESULT:1 = LOCAL_NUMBERS:0
RESULTS:0 '= LOCAL_WORDS:1
RETURN RESULT:0
"#;
    let header = METHOD_FIXTURE_HEADER.to_owned() + "\n#DIM CONST SNAP_LOCKED, 3 = 1, 2, 3\n";
    let artifact = compile_with_header(&header, &source, &method_options(true));
    for entry_name in ["REF_SNAPSHOT_TITLE", "FORM_REF_SNAPSHOT_TITLE"] {
        let function = |name| {
            artifact
                .functions
                .iter()
                .find(|function| function.name == name)
                .unwrap()
        };
        let entry = function(entry_name).key;
        let forward = function("SNAP_FORWARD");
        let target = function("SNAP_WAIT");
        let parameter_key = |slot: usize| {
            serde_json::to_value(target.parameters[slot].key)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        };
        let mut natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 123_456);
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
        let mut host = PendingHost {
            stability: HostWaitStability::StableInput,
            rebound: Vec::new(),
        };
        let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
        let request = report
            .events
            .iter()
            .find_map(|event| match event {
                VmEvent::HostPending { request, .. } => Some(*request),
                _ => None,
            })
            .unwrap_or_else(|| panic!("expected active REF input: {report:?}"));
        let snapshot = serde_json::to_value(vm.snapshot(&natives).unwrap()).unwrap();
        let frames = &snapshot["fibers"][fiber.0.to_string()]["frames"];
        assert_eq!(frames.as_array().unwrap().len(), 3);
        for corruption in [
            "fiber",
            "owner",
            "generation",
            "type",
            "rank",
            "indices",
            "character",
            "character_source",
            "immutable",
            "cycle",
            "cell_type",
            "cell_shape",
        ] {
            let mut corrupted = snapshot.clone();
            let frames = &mut corrupted["fibers"][fiber.0.to_string()]["frames"];
            let target_id = frames[2]["id"].clone();
            let cell = &mut frames[2]["locals"][parameter_key(0)];
            let place = &mut cell[2]["IntegerPlaces"][0][1];
            match corruption {
                "fiber" => place["fiber"] = serde_json::json!(999),
                "owner" => place["frame"] = serde_json::json!(999),
                "generation" => frames[0]["generation"] = serde_json::json!(999),
                "type" => {
                    place["variable"] =
                        serde_json::to_value(named_key(&artifact, "METHOD_WORDS")).unwrap();
                    place["frame"] = serde_json::Value::Null;
                }
                "rank" => {
                    place["variable"] =
                        serde_json::to_value(named_key(&artifact, "METHOD_MATRIX")).unwrap();
                    place["frame"] = serde_json::Value::Null;
                }
                "indices" => place["indices"] = serde_json::json!([0]),
                "character" => place["character"] = serde_json::json!(0),
                "character_source" => {
                    place["variable"] =
                        serde_json::to_value(named_key(&artifact, "CFLAG")).unwrap();
                    place["frame"] = serde_json::Value::Null;
                }
                "immutable" => {
                    place["variable"] =
                        serde_json::to_value(named_key(&artifact, "SNAP_LOCKED")).unwrap();
                    place["frame"] = serde_json::Value::Null;
                }
                "cycle" => {
                    place["variable"] = serde_json::to_value(target.parameters[0].key).unwrap();
                    place["frame"] = target_id;
                }
                "cell_type" => {
                    let places = cell[2]["IntegerPlaces"].take();
                    cell[0] = serde_json::to_value(BytecodeType::StringPlace).unwrap();
                    cell[2] = serde_json::json!({"StringPlaces": places});
                }
                "cell_shape" => cell[1] = serde_json::json!([2]),
                _ => unreachable!(),
            }
            let corrupted: VmSnapshot = serde_json::from_value(corrupted).unwrap();
            let before = vm.encode_snapshot(&natives).unwrap();
            let mut rejected_host = PendingHost {
                stability: HostWaitStability::StableInput,
                rebound: Vec::new(),
            };
            assert!(
                Vm::restore_snapshot(
                    validated(&artifact),
                    VmConfig::default(),
                    corrupted,
                    &mut rejected_host,
                    &mut natives
                )
                .is_err(),
                "{corruption}"
            );
            assert!(
                rejected_host.rebound.is_empty(),
                "{corruption}: invalid alias rebound the host"
            );
            assert_eq!(
                vm.encode_snapshot(&natives).unwrap(),
                before,
                "{corruption}: native state changed"
            );
        }
        // Normalized bindings and a valid explicit forwarding chain must both retain the caller's arrays.
        for forwarding_chain in [false, true] {
            let mut snapshot = snapshot.clone();
            if forwarding_chain {
                let frames = &mut snapshot["fibers"][fiber.0.to_string()]["frames"];
                let owner = frames[1]["id"].clone();
                for (slot, storage) in ["IntegerPlaces", "StringPlaces"].into_iter().enumerate() {
                    let place = &mut frames[2]["locals"][parameter_key(slot)][2][storage][0][1];
                    place["variable"] = serde_json::to_value(forward.parameters[slot].key).unwrap();
                    place["frame"] = owner.clone();
                }
            }
            let snapshot: VmSnapshot = serde_json::from_value(snapshot).unwrap();
            let mut restored_host = PendingHost {
                stability: HostWaitStability::StableInput,
                rebound: Vec::new(),
            };
            let mut restored = Vm::restore_snapshot(
                validated(&artifact),
                VmConfig::default(),
                snapshot,
                &mut restored_host,
                &mut natives,
            )
            .unwrap();
            assert!(!restored_host.rebound.is_empty());
            restored.resume_host(request, HostReady::empty()).unwrap();
            let report = restored.run_slice(
                &mut ReadyHost::default(),
                &mut natives,
                RunBudget::default(),
            );
            assert!(
                !report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
                "forwarding={forwarding_chain}: {report:?}"
            );
            assert_method_watch(&restored, &artifact, "RESULT", 0, VmValue::Integer(11));
            assert_method_watch(&restored, &artifact, "RESULT", 1, VmValue::Integer(11));
            assert_method_watch(
                &restored,
                &artifact,
                "RESULTS",
                0,
                VmValue::String("restored".into()),
            );
        }
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Keep the pending-expression corruption matrix beside the valid restore control."
)]
fn runtime_form_snapshot_rejects_invalid_pending_operator_types_before_external_restore() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULTS:0 '= STRFORM("{GETMETH(\"SNAP_PAIR\", , SNAP_INPUT(), 2 + 3)}")
RETURN
@SNAP_PAIR(LEFT, RIGHT)
#FUNCTION
#DIM DYNAMIC LEFT
#DIM DYNAMIC RIGHT
RETURNF LEFT * 10 + RIGHT
@SNAP_INPUT
#FUNCTION
INPUT
RETURNF RESULT
"#,
        &method_options(true),
    );
    let mut natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 123_456);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm
        .spawn_entry(
            artifact
                .functions
                .iter()
                .find(|function| function.name == "SYSTEM_TITLE")
                .unwrap()
                .key,
            Vec::new(),
        )
        .unwrap();
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    let request = report
        .events
        .iter()
        .find_map(|event| match event {
            VmEvent::HostPending { request, .. } => Some(*request),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected suspended argument: {report:?}"));
    let snapshot = serde_json::to_value(vm.snapshot(&natives).unwrap()).unwrap();
    for corruption in [
        "binary",
        "unary",
        "condition",
        "comparison",
        "increment",
        "postfix",
    ] {
        let mut corrupted = snapshot.clone();
        let work = corrupted["fibers"][fiber.0.to_string()]["frames"][0]["runtime_form"]["work"]
            .as_array_mut()
            .unwrap();
        let call = work
            .iter_mut()
            .find_map(|task| task.get_mut("CaptureMethodArgument"))
            .unwrap();
        let argument = &mut call["arguments"][1];
        let span = argument["span"].clone();
        let integer = serde_json::json!({"kind": {"Integer": 1}, "span": span});
        let string = serde_json::json!({"kind": {"String": "wrong"}, "span": span});
        argument["kind"] = match corruption {
            "binary" => {
                serde_json::json!({"Binary": {"op": "Subtract", "left": string, "right": integer}})
            }
            "unary" => serde_json::json!({"Unary": {"op": "Minus", "operand": string}}),
            "condition" => {
                serde_json::json!({"Ternary": {"condition": string, "then_expr": integer, "else_expr": integer}})
            }
            "comparison" => {
                serde_json::json!({"Binary": {"op": "Equal", "left": string, "right": integer}})
            }
            "increment" => serde_json::json!({"Unary": {"op": "PreIncrement", "operand": integer}}),
            "postfix" => serde_json::json!({"Postfix": {"op": "Increment", "operand": integer}}),
            _ => unreachable!(),
        };
        let corrupted: VmSnapshot = serde_json::from_value(corrupted).unwrap();
        let before = vm.encode_snapshot(&natives).unwrap();
        let mut rejected_host = PendingHost {
            stability: HostWaitStability::StableInput,
            rebound: Vec::new(),
        };
        assert!(
            Vm::restore_snapshot(
                validated(&artifact),
                VmConfig::default(),
                corrupted,
                &mut rejected_host,
                &mut natives
            )
            .is_err(),
            "{corruption}"
        );
        assert!(rejected_host.rebound.is_empty());
        assert_eq!(vm.encode_snapshot(&natives).unwrap(), before);
    }
    let mut restored = Vm::restore_snapshot(
        validated(&artifact),
        VmConfig::default(),
        serde_json::from_value(snapshot).unwrap(),
        &mut host,
        &mut natives,
    )
    .unwrap();
    restored
        .write_variable(
            named_key(&artifact, "RESULT"),
            &[0],
            None,
            VmValue::Integer(3),
        )
        .unwrap();
    restored.resume_host(request, HostReady::empty()).unwrap();
    let report = restored.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_method_watch(
        &restored,
        &artifact,
        "RESULTS",
        0,
        VmValue::String("35".into()),
    );
}

#[test]
fn dynamic_method_ref_rejections_do_not_evaluate_immutable_or_character_indices() {
    let header = METHOD_FIXTURE_HEADER.to_owned() + "\n#DIM CONST METHOD_LOCKED, 3 = 1, 2, 3\n";
    let mut source = METHOD_FIXTURE_SOURCE.to_owned();
    for (index, actual) in ["METHOD_LOCKED:METHOD_INDEX()", "CFLAG:METHOD_INDEX()"]
        .iter()
        .enumerate()
    {
        let expression =
            format!("GETMETH(METHOD_NAME(\"METHOD_REF_INT\"), METHOD_FALLBACK(), {actual})");
        write!(
            source,
            "\n@REF_REJECT_{index}\nCALL METHOD_RESET\nRESULT:0 = {expression}\nRETURN\n"
        )
        .unwrap();
        let escaped = expression.replace('"', "\\\"");
        write!(source, "\n@FORM_REF_REJECT_{index}\nCALL METHOD_RESET\nRESULTS:0 '= STRFORM(\"{{{escaped}}}\")\nRETURN\n").unwrap();
    }
    let artifact = compile_with_header(&header, &source, &method_options(true));
    for entry in [
        "REF_REJECT_0",
        "REF_REJECT_1",
        "FORM_REF_REJECT_0",
        "FORM_REF_REJECT_1",
    ] {
        let (vm, report) = run_method_case(&artifact, entry, VmConfig::default());
        assert_eq!(
            take_fault(report).code,
            VmFaultCode::TypeMismatch,
            "{entry}"
        );
        for (name, expected) in [
            ("METHOD_TRACE", 1),
            ("METHOD_BODY_COUNT", 0),
            ("METHOD_INDEX_COUNT", 0),
        ] {
            assert_method_watch(&vm, &artifact, name, 0, VmValue::Integer(expected));
        }
        assert_method_watch(&vm, &artifact, "METHOD_LOCKED", 0, VmValue::Integer(1));
    }
}

#[test]
fn discarded_method_tokens_leave_no_pending_snapshot_state() {
    let (mut artifact, entry) = host_artifact(HostSnapshotCapability::StableWait);
    let target_key = SymbolKey::derive("test.method", b"discarded");
    let mut target = function(
        target_key,
        "DISCARDED_METHOD",
        vec![opcode::push_integer(42), opcode::return_value(true)],
    );
    target.kind = erabasic_bytecode::BytecodeFunctionKind::Method;
    target.result = Some(BytecodeType::Integer);
    artifact.functions.push(target);
    artifact
        .functions
        .iter_mut()
        .find(|function| function.key == entry)
        .unwrap()
        .code
        .splice(
            0..0,
            [
                opcode::push_string("MISSING_METHOD"),
                opcode::resolve_user_call(&erabasic_bytecode::UserCallSpec {
                    mode: erabasic_bytecode::UserCallMode::MethodInteger,
                    allow_missing: true,
                    missing_target: 5,
                    arguments: Vec::new(),
                }),
                opcode::invoke_user_call(1),
                erabasic_bytecode::EncodedInstruction::new(Opcode::Pop, Vec::new()),
                opcode::jump(Opcode::Jump, 6),
                opcode::abandon_user_call(1),
            ],
        );
    artifact.refresh_ids().unwrap();
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 123_456);
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::HostPending { .. })),
        "{report:?}"
    );
    let snapshot = vm.snapshot(&natives).unwrap();
    let json = serde_json::to_value(&snapshot).unwrap();
    assert!(
        json["fibers"][fiber.0.to_string()]["frames"][0]["user_calls"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        Vm::restore_snapshot(
            validated(&artifact),
            VmConfig::default(),
            snapshot,
            &mut host,
            &mut natives
        )
        .is_ok()
    );
}

#[test]
fn resolved_method_metadata_obeys_the_vm_operand_limit() {
    let artifact = compile_with_header(
        METHOD_FIXTURE_HEADER,
        METHOD_FIXTURE_SOURCE,
        &method_options(true),
    );
    let (vm, report) = run_method_case(
        &artifact,
        "METHOD_CASE_TRAILING_DEFAULTS",
        VmConfig {
            maximum_operand_stack: 3,
            ..VmConfig::default()
        },
    );
    assert_eq!(take_fault(report).code, VmFaultCode::ResourceLimit);
    assert_method_watch(&vm, &artifact, "METHOD_BODY_COUNT", 0, VmValue::Integer(0));
}

#[track_caller]
fn take_fault(report: erabasic_vm::VmRunReport) -> erabasic_vm::VmFault {
    let debug = format!("{report:#?}");
    report
        .events
        .into_iter()
        .find_map(|event| match event {
            VmEvent::FiberFaulted { fault, .. } => Some(fault),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected a VM fault, got {debug}"))
}

#[test]
fn runtime_form_covers_triples_escapes_interpolation_conditionals_and_calls() {
    let artifact = compile_source(
        r#"@SYSTEM_TITLE
ADDVOIDCHARA
NAME:0 = Target
CALLNAME:0 = Call
TARGET = 0
MASTER = 0
PLAYER = 0
ASSI = 0
#DIM DYNAMIC VALUE
#DIMS DYNAMIC TEXT
VALUE = 7
TEXT '= "text"
RESULT:9 = TOINT("7")
RESULTS:0 '= STRFORM("A\\s{VALUE,3,LEFT}|%TEXT%|\\@ VALUE == 7 ? yes # no \\@|***|+++|===|///|$$$|{TOINT(\"7\")}|%WRAP()%|\\%")
RETURN
@WRAP
#FUNCTIONS
RETURNF "[user]"
"#,
    );
    let results = named_key(&artifact, "RESULTS");
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert_eq!(report.stop, erabasic_vm::VmRunStop::Idle);
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{:#?}",
        report.events
    );
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String(
            "A 7  |text|yes|Target|Call|Call|Target|Call|7|[user]|%".into()
        ))
    );
}

#[test]
fn runtime_form_uses_the_project_ignore_triple_symbols_compatibility() {
    let mut options = AnalyzerOptions::analysis_mode();
    options.ignore_triple_symbols = true;
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT = 5
RESULTS:0 '= STRFORM("***\\s{RESULT}")
RETURN
"#,
        &options,
    );
    assert!(artifact.call_compatibility.ignore_triple_symbols);
    let results = named_key(&artifact, "RESULTS");
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert_eq!(report.stop, erabasic_vm::VmRunStop::Idle);
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("*** 5".into()))
    );
}

#[test]
fn runtime_form_resolves_dynamic_locals_and_statics_before_the_global() {
    let artifact = compile_with_header(
        "#DIMS VALUE\n",
        r#"@SYSTEM_TITLE
VALUE '= "global"
RESULTS:0 '= STRFORM("%VALUE%")
RESULTS:1 '= STRFORM("%READ_LOCAL()%")
RESULTS:2 '= STRFORM("%READ_STATIC()%")
RETURN
@READ_LOCAL
#FUNCTIONS
#DIMS DYNAMIC VALUE
VALUE '= "local"
RETURNF STRFORM("%VALUE%")
@READ_STATIC
#FUNCTIONS
#DIMS STATIC VALUE
VALUE '= "static"
RETURNF STRFORM("%VALUE%")
"#,
        &AnalyzerOptions::analysis_mode(),
    );
    let results = named_key(&artifact, "RESULTS");
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert_eq!(report.stop, erabasic_vm::VmRunStop::Idle);
    for (index, expected) in [(0, "global"), (1, "local"), (2, "static")] {
        assert_eq!(
            vm.read_variable(results, &[index], None),
            Ok(VmValue::String(expected.into()))
        );
    }
}

#[test]
fn runtime_form_named_indices_prefer_variables_then_functions_then_csv_keys() {
    let mut data = project_data();
    let flag = data
        .static_data
        .name_tables
        .get_mut(&erabasic_data::NameTableKind::Flag)
        .expect("FLAG name table");
    flag.lookup.insert("INDEX_FUNCTION".into(), 7);
    flag.lookup.insert("INDEX_VARIABLE".into(), 8);
    flag.lookup.insert("INDEX_KEY".into(), 7);
    let artifact = compile_source_with_data(
        r#"@SYSTEM_TITLE
#DIM DYNAMIC INDEX_VARIABLE
INDEX_VARIABLE = 10
FLAG:7 = 70
FLAG:8 = 80
FLAG:9 = 90
FLAG:10 = 100
RESULTS:0 '= STRFORM("{FLAG:INDEX_VARIABLE}")
RESULTS:1 '= STRFORM("{FLAG:INDEX_FUNCTION}")
RESULTS:2 '= STRFORM("{FLAG:INDEX_KEY}")
RETURN
@INDEX_FUNCTION
#FUNCTION
RETURNF 9
"#,
        data,
    );
    let results = named_key(&artifact, "RESULTS");
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert_eq!(report.stop, erabasic_vm::VmRunStop::Idle);
    for (index, expected) in [(0, "100"), (1, "90"), (2, "70")] {
        assert_eq!(
            vm.read_variable(results, &[index], None),
            Ok(VmValue::String(expected.into()))
        );
    }
}

#[test]
fn runtime_form_resolves_character_data_names_after_the_character_index() {
    let mut data = project_data();
    data.static_data
        .name_tables
        .get_mut(&erabasic_data::NameTableKind::Palam)
        .expect("PALAM name table")
        .lookup
        .insert("快Ｃ".into(), 17);
    let artifact = compile_source_with_data(
        r#"@SYSTEM_TITLE
ADDVOIDCHARA
CUP:0:17 = 9
RESULTS:0 '= STRFORM("{CUP:0:快Ｃ}")
RETURN
"#,
        data,
    );
    let results = named_key(&artifact, "RESULTS");
    let (vm, report) = run_entry(&artifact, VmConfig::default());

    assert_eq!(report.stop, erabasic_vm::VmRunStop::Idle);
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("9".into()))
    );
}

#[test]
fn runtime_form_rejects_private_column_import_before_side_effects() {
    let artifact = compile_source(
        r#"@SYSTEM_TITLE
DT_CREATE "t"
DT_COLUMN_ADD "t", "value", "int64", 1
DT_COLUMN_OPTIONS "t", "value", DEFAULT, 9
RESULT:10 = 0
RESULTS:0 '= STRFORM("{BUMP()}%DT__COLUMN_RESOLVE(\"value\", \"t\")%")
RETURN
@BUMP
#FUNCTION
RESULT:10 += 1
RETURNF 1
"#,
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    let fault = take_fault(report);
    assert!(
        fault.message.contains("internal column operation"),
        "{fault:?}"
    );
    assert_eq!(
        vm.read_variable(named_key(&artifact, "RESULT"), &[10], None),
        Ok(VmValue::Integer(0))
    );
}

#[test]
fn runtime_form_keeps_user_methods_named_like_private_column_imports() {
    let artifact = compile_source(
        r#"@SYSTEM_TITLE
RESULTS:0 '= STRFORM("%DT__COLUMN_RESOLVE()%")
RETURN
@DT__COLUMN_RESOLVE
#FUNCTIONS
RETURNF "user-method"
"#,
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_eq!(
        vm.read_variable(named_key(&artifact, "RESULTS"), &[0], None),
        Ok(VmValue::String("user-method".into()))
    );
}

#[test]
fn runtime_form_rejects_non_lvalue_mutation_before_any_user_side_effect() {
    let artifact = compile_source(
        r#"@SYSTEM_TITLE
RESULT = 0
RESULTS:0 '= STRFORM("{BUMP()}|{(RESULT + 1)++}")
RETURN
@BUMP
#FUNCTION
RESULT += 1
RETURNF 1
"#,
    );
    let result = named_key(&artifact, "RESULT");
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    let fault = take_fault(report);
    assert!(matches!(
        fault.category,
        erabasic_vm::FaultCategory::Script(
            erabasic_vm::ScriptFaultKind::Parse | erabasic_vm::ScriptFaultKind::Argument
        )
    ));
    assert_eq!(
        vm.read_variable(result, &[], None),
        Ok(VmValue::Integer(0)),
        "preflight must reject before BUMP executes"
    );
}

#[test]
fn runtime_form_survives_tiny_budgets_and_executes_user_side_effects_once() {
    let artifact = compile_source(
        r#"@SYSTEM_TITLE
RESULT:1 = 0
RESULTS:0 '= STRFORM("{BUMP()}")
RETURN
@BUMP
#FUNCTION
RESULT:1 += 1
RETURNF RESULT:1
"#,
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let result = named_key(&artifact, "RESULT");
    let results = named_key(&artifact, "RESULTS");
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let budget = RunBudget {
        maximum_instructions: 1,
        maximum_host_calls: 1,
        fiber_quantum: 1,
    };
    let mut exhausted = 0;
    let mut completed = false;
    for _ in 0..100 {
        let report = vm.run_slice(&mut ReadyHost::default(), &mut natives, budget);
        if report.stop == erabasic_vm::VmRunStop::BudgetExhausted {
            exhausted += 1;
        }
        completed |= report.events.iter().any(
            |event| matches!(event, VmEvent::FiberCompleted { fiber: done, .. } if *done == fiber),
        );
        if report.stop == erabasic_vm::VmRunStop::Idle {
            break;
        }
    }
    assert!(exhausted > 1, "the continuation must cross multiple slices");
    assert!(completed);
    assert_eq!(
        vm.read_variable(result, &[1], None),
        Ok(VmValue::Integer(1))
    );
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("1".into()))
    );
}

#[test]
fn runtime_form_reports_malformed_missing_type_bounds_and_resource_errors() {
    for (template, expected) in [
        ("value={RESULT", VmFaultCode::Native),
        ("{DOES_NOT_EXIST}", VmFaultCode::MissingSymbol),
        ("{RESULTS}", VmFaultCode::TypeMismatch),
        ("{RESULT:999999}", VmFaultCode::Bounds),
    ] {
        let artifact = compile_source(&format!(
            "@SYSTEM_TITLE\nRESULTS:0 '= STRFORM(\"{template}\")\nRETURN\n"
        ));
        let (_, report) = run_entry(&artifact, VmConfig::default());
        assert_eq!(take_fault(report).code, expected, "{template:?}");
    }

    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULTS:0 '= STRFORM(\"a{RESULT}b{RESULT}c{RESULT}\")\nRETURN\n",
    );
    let (_, report) = run_entry(
        &artifact,
        VmConfig {
            maximum_operand_stack: 4,
            ..VmConfig::default()
        },
    );
    assert_eq!(take_fault(report).code, VmFaultCode::ResourceLimit);
}

#[test]
fn recursive_runtime_forms_stop_at_the_vm_call_depth_without_rust_recursion() {
    let artifact = compile_source(
        r#"@SYSTEM_TITLE
RESULTS:0 '= STRFORM("%RECURSE(0)%")
RETURN
@RECURSE(ARG)
#FUNCTIONS
RETURNF STRFORM("%RECURSE(ARG + 1)%")
"#,
    );
    let (_, report) = run_entry(
        &artifact,
        VmConfig {
            maximum_call_depth: 8,
            ..VmConfig::default()
        },
    );
    assert_eq!(take_fault(report).code, VmFaultCode::ResourceLimit);
}

struct RuntimeFormHost {
    calls: usize,
    pending: bool,
    value: String,
}

impl VmHost for RuntimeFormHost {
    fn call(&mut self, request: HostCallRequest) -> HostCallResult {
        assert!(request.import.name.eq_ignore_ascii_case("LOADTEXT"));
        self.calls += 1;
        if self.pending {
            HostCallResult::Pending {
                stability: HostWaitStability::Transient,
                rebind_payload: b"runtime-form-loadtext".to_vec(),
            }
        } else {
            HostCallResult::Ready(HostReady {
                value: Some(VmValue::String(self.value.clone())),
                writes: Vec::new(),
            })
        }
    }
}

fn host_runtime_form_artifact() -> BytecodeArtifact {
    compile_source(
        r#"@SYSTEM_TITLE
RESULTS:0 '= STRFORM("%READ_HOST()%")
RETURN
@READ_HOST
#FUNCTIONS
RETURNF LOADTEXT("fixture.txt")
"#,
    )
}

#[test]
fn runtime_form_obeys_host_caps_and_resumes_after_a_suspended_user_function() {
    let artifact = host_runtime_form_artifact();
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let results = named_key(&artifact, "RESULTS");
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = RuntimeFormHost {
        calls: 0,
        pending: true,
        value: "unused".into(),
    };

    let capped = vm.run_slice(
        &mut host,
        &mut natives,
        RunBudget {
            maximum_instructions: 1_000,
            maximum_host_calls: 0,
            fiber_quantum: 1,
        },
    );
    assert_eq!(capped.stop, erabasic_vm::VmRunStop::BudgetExhausted);
    assert_eq!(host.calls, 0);

    let suspended = vm.run_slice(
        &mut host,
        &mut natives,
        RunBudget {
            maximum_instructions: 1_000,
            maximum_host_calls: 1,
            fiber_quantum: 1,
        },
    );
    let request = suspended
        .events
        .iter()
        .find_map(|event| match event {
            VmEvent::HostPending {
                fiber: waiting,
                request,
            } if *waiting == fiber => Some(*request),
            _ => None,
        })
        .expect("runtime FORM user function must suspend at LOADTEXT");
    assert_eq!(host.calls, 1);
    vm.resume_host(
        request,
        HostReady {
            value: Some(VmValue::String("resumed".into())),
            writes: Vec::new(),
        },
    )
    .unwrap();
    let resumed = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    completed_without_fault(&resumed, fiber);
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("resumed".into()))
    );
}

#[test]
fn era_restart_discards_a_suspended_runtime_form_and_starts_cleanly() {
    let artifact = host_runtime_form_artifact();
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let results = named_key(&artifact, "RESULTS");
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let old = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = RuntimeFormHost {
        calls: 0,
        pending: true,
        value: "restarted".into(),
    };
    let suspended = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(
        suspended
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::HostPending { fiber, .. } if *fiber == old))
    );

    let state = vm.export_era_state();
    vm.reset_with_era_state(&state).unwrap();
    assert!(vm.fiber_status(old).is_none());
    assert_eq!(vm.fiber_ids().count(), 0);

    host.pending = false;
    let restarted = vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    completed_without_fault(&report, restarted);
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("restarted".into()))
    );
}

#[test]
fn instruction_step_treats_context_free_runtime_form_work_as_one_native_call() {
    let artifact = compile_source("@SYSTEM_TITLE\nRESULTS '= STRFORM(\"a{1 + 2}b\")\nRETURN\n");
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let strform_instruction = artifact
        .functions
        .iter()
        .find(|function| function.key == entry)
        .and_then(|function| {
            function.code.iter().position(|instruction| {
                Opcode::try_from(instruction.opcode) == Ok(Opcode::CallNative)
            })
        })
        .expect("compiled STRFORM call");
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut expanded = vm.request_pause().unwrap();
    for instruction in 0..=strform_instruction {
        vm.step(expanded.token, fiber, VmStepKind::Instruction)
            .unwrap();
        let report = vm.run_slice(
            &mut ReadyHost::default(),
            &mut natives,
            RunBudget::default(),
        );
        if instruction == strform_instruction {
            assert!(
                report.instructions > 1,
                "continuation work must complete behind the STRFORM after-hook"
            );
        } else {
            assert_eq!(report.instructions, 1);
        }
        expanded = report
            .events
            .iter()
            .find_map(|event| match event {
                VmEvent::DebugStopped(stop) => Some(stop.clone()),
                _ => None,
            })
            .expect("instruction debug stop");
    }
    let frame = vm
        .call_stack(expanded.token, fiber)
        .unwrap()
        .into_iter()
        .next()
        .expect("SYSTEM_TITLE frame");
    let operands = vm
        .operand_stack(expanded.token, fiber, frame.id, None, 16)
        .unwrap();
    assert!(
        operands
            .values
            .iter()
            .any(|operand| operand.value == VmValue::String("a3b".into())),
        "{operands:#?}"
    );
}

#[test]
fn strformcheck_catches_parse_and_expansion_failures_without_rolling_back_effects() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:0 = STRFORMCHECK("{SIDE()}")
RESULT:1 = STRFORMCHECK("{SIDE()} {FLAG:9999999}")
RESULT:2 = STRFORMCHECK("{")
RESULT:3 = STRFORMCHECK("{UNKNOWN_FORM_VARIABLE}")
RESULTS:0 '= "{SIDE()} {FLAG:9999999}"
RESULTS:1 '= "{STRFORMCHECK(RESULTS:0)}"
RESULT:4 = STRFORMCHECK(RESULTS:1)
RESULT:5 = FLAG:0
RETURN RESULT:0
@SIDE
#FUNCTION
FLAG:0 += 1
RETURNF FLAG:0
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    for (index, expected) in [(0, 1), (1, 0), (2, 0), (3, 0), (4, 1), (5, 3)] {
        assert_method_watch(&vm, &artifact, "RESULT", index, VmValue::Integer(expected));
    }
}

#[test]
fn strformcheck_outer_argument_failure_is_not_caught_by_its_own_checkpoint() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:0 = STRFORMCHECK(BAD_SOURCE())
FLAG:1 = 1
RETURN
@BAD_SOURCE
#FUNCTIONS
FLAG:0 += 1
RESULT:9 = FLAG:9999999
RETURNF "unused"
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    let fault = take_fault(report);
    assert!(matches!(
        fault.category,
        erabasic_vm::FaultCategory::Script(erabasic_vm::ScriptFaultKind::Bounds)
    ));
    assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(1));
    assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(0));
}

#[test]
fn runtime_form_user_calls_discard_extra_actuals_before_evaluation() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULTS:0 '= STRFORM("{TAKE(7, SIDE())}")
RESULTS:1 '= STRFORM("{GETMETH(\"TAKE\", , 8, SIDE())}")
RESULT:1 = FLAG:0
RETURN
@TAKE(ARG)
#FUNCTION
RETURNF ARG
@SIDE
#FUNCTION
FLAG:0 += 1
RETURNF FLAG:9999999
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert_eq!(report.stop, erabasic_vm::VmRunStop::Idle);
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_method_watch(&vm, &artifact, "RESULTS", 0, VmValue::String("7".into()));
    assert_method_watch(&vm, &artifact, "RESULTS", 1, VmValue::String("8".into()));
    assert_method_watch(&vm, &artifact, "RESULT", 1, VmValue::Integer(0));
}

#[test]
fn strformcheck_checkpoint_survives_input_wait_and_rejects_forged_snapshot_markers() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:0 = STRFORMCHECK("{WAIT_FAILURE()}")
FLAG:1 = 1
RETURN
@WAIT_FAILURE
#FUNCTION
INPUT
FLAG:0 += 1
RETURNF FLAG:9999999
"#,
        &method_options(true),
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let suspended = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    let request = suspended
        .events
        .iter()
        .find_map(|event| match event {
            VmEvent::HostPending { request, .. } => Some(*request),
            _ => None,
        })
        .expect("checked expansion must reach the real Host wait");
    let saved = serde_json::to_value(vm.snapshot(&natives).unwrap()).unwrap();
    for field in ["id", "work_depth", "value_depth", "owner_stack_depth"] {
        let mut corrupted = saved.clone();
        corrupted["fibers"][fiber.0.to_string()]["frames"][0]["runtime_form"]["checkpoints"][0]
            [field] = serde_json::json!(999_999);
        let mut rejected = PendingHost {
            stability: HostWaitStability::StableInput,
            rebound: Vec::new(),
        };
        assert!(
            Vm::restore_snapshot(
                validated(&artifact),
                VmConfig::default(),
                serde_json::from_value(corrupted).unwrap(),
                &mut rejected,
                &mut natives
            )
            .is_err(),
            "{field}"
        );
        assert!(
            rejected.rebound.is_empty(),
            "{field}: invalid checkpoint rebound Host"
        );
    }
    let mut restored = Vm::restore_snapshot(
        validated(&artifact),
        VmConfig::default(),
        serde_json::from_value(saved).unwrap(),
        &mut host,
        &mut natives,
    )
    .unwrap();
    restored.resume_host(request, HostReady::empty()).unwrap();
    let resumed = restored.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    completed_without_fault(&resumed, fiber);
    assert_method_watch(&restored, &artifact, "RESULT", 0, VmValue::Integer(0));
    assert_method_watch(&restored, &artifact, "FLAG", 0, VmValue::Integer(1));
    assert_method_watch(&restored, &artifact, "FLAG", 1, VmValue::Integer(1));
}

#[test]
fn strformcheck_does_not_catch_call_depth_resource_failure() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:0 = STRFORMCHECK("%RECURSE()%")
FLAG:1 = 1
RETURN
@RECURSE
#FUNCTIONS
RETURNF STRFORM("%RECURSE()%")
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(
        &artifact,
        VmConfig {
            maximum_call_depth: 4,
            ..VmConfig::default()
        },
    );
    let fault = take_fault(report);
    assert_eq!(fault.category, erabasic_vm::FaultCategory::ResourceLimit);
    assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(0));
}

#[test]
fn call_text_try_catches_binding_and_name_failure_but_not_callee_failure() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
TRYCCALLSTR "TAKE(\"bad type\")"
FLAG:0 = 99
CATCH
FLAG:0 = 1
ENDCATCH
TRYCALLSTR "MISSING, 7"
TRYCALLSTR "TAKE(UNKNOWN_VARIABLE)"
CALLSTR "TAKE(8)"
RETURN
@TAKE(ARG)
FLAG:1 = ARG
RETURN
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert_eq!(report.stop, erabasic_vm::VmRunStop::Idle);
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(1));
    assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(8));
    let artifact = compile_source_with_options(
        "@SYSTEM_TITLE\nTRYCALLSTR \"TAKE(7)\"\nRETURN\n@TAKE(ARG)\nFLAG:0 = FLAG:9999999\nRETURN\n",
        &method_options(true),
    );
    let (_, report) = run_entry(&artifact, VmConfig::default());
    assert!(matches!(
        take_fault(report).category,
        erabasic_vm::FaultCategory::Script(_)
    ));
}

#[test]
fn runtime_form_prefix_postfix_share_fixed_profile_boundary_results_and_warnings() {
    for snake in [false, true] {
        for (expression, initial, original_return, original_store, snake_return, snake_store) in [
            ("++FLAG:0", i64::MAX, i64::MIN, i64::MIN, i64::MAX, i64::MAX),
            ("--FLAG:0", i64::MIN, i64::MAX, i64::MAX, i64::MIN, i64::MIN),
            (
                "FLAG:0++",
                i64::MAX,
                i64::MAX,
                i64::MIN,
                i64::MAX - 1,
                i64::MAX,
            ),
            (
                "FLAG:0--",
                i64::MIN,
                i64::MIN,
                i64::MAX,
                i64::MIN + 1,
                i64::MIN,
            ),
        ] {
            let artifact = compile_source_with_options(
                &format!("@SYSTEM_TITLE\nRESULTS:0 '= STRFORM(\"{{{expression}}}\")\nRETURN\n"),
                &method_options(snake),
            );
            let mut vm = Vm::new(validated(&artifact), VmConfig::default());
            vm.write_variable(
                named_key(&artifact, "FLAG"),
                &[0],
                None,
                VmValue::Integer(initial),
            )
            .unwrap();
            let mut natives = NativeServiceRegistry::for_artifact(&artifact);
            let entry = artifact
                .functions
                .iter()
                .find(|function| function.name == "SYSTEM_TITLE")
                .unwrap()
                .key;
            let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
            let report = vm.run_slice(
                &mut ReadyHost::default(),
                &mut natives,
                RunBudget::default(),
            );
            completed_without_fault(&report, fiber);
            let (returned, stored) = if snake {
                (snake_return, snake_store)
            } else {
                (original_return, original_store)
            };
            assert_method_watch(
                &vm,
                &artifact,
                "RESULTS",
                0,
                VmValue::String(returned.to_string()),
            );
            assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(stored));
            assert_eq!(
                report
                    .events
                    .iter()
                    .filter(|event| matches!(event,
                VmEvent::Diagnostic { code, .. } if code == "compat.arithmetic.overflow"))
                    .count(),
                usize::from(snake)
            );
        }
    }
}

#[test]
fn runtime_form_mutation_preserves_index_value_order_and_character_and_ref_places() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
#DIM DYNAMIC NUMBERS, 3
FLAG:0 = 10
RESULTS:0 '= STRFORM("{++FLAG:(INDEX())}|{FLAG:0++}|{OBSERVE()}")
ADDVOIDCHARA
CFLAG:0:0 = 20
RESULTS:1 '= STRFORM("{CFLAG:0:0++}|{++CFLAG:0:0}")
NUMBERS:0 = 30
RESULTS:2 '= STRFORM("%CHANGE_REF(NUMBERS)%")
RESULT:2 = NUMBERS:0
RETURN
@INDEX
#FUNCTION
FLAG:1 = FLAG:1 * 10 + 1
RETURNF 0
@OBSERVE
#FUNCTION
FLAG:1 = FLAG:1 * 10 + 2
RETURNF FLAG:0
@CHANGE_REF(VALUES)
#FUNCTIONS
#DIM REF VALUES
RETURNF STRFORM("{VALUES:0++}|{++VALUES:0}")
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert_eq!(report.stop, erabasic_vm::VmRunStop::Idle);
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    for (index, expected) in [(0, "11|11|12"), (1, "20|22"), (2, "30|32")] {
        assert_method_watch(
            &vm,
            &artifact,
            "RESULTS",
            index,
            VmValue::String(expected.into()),
        );
    }
    assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(12));
    assert_method_watch(&vm, &artifact, "RESULT", 2, VmValue::Integer(32));
    assert_eq!(
        vm.read_variable(named_key(&artifact, "CFLAG"), &[0], Some(0)),
        Ok(VmValue::Integer(22))
    );
}

#[test]
fn existvar_evaluates_source_then_mode_then_source_only_for_nonzero_mode() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:0 = EXISTVAR(NAME_SOURCE(), MODE_VALUE())
RESULT:1 = FLAG:0
FLAG:0 = 0
RESULTS:0 '= STRFORM("{EXISTVAR(NAME_SOURCE(), MODE_VALUE())}")
RESULT:2 = FLAG:0
FLAG:0 = 0
FLAG:1 = 1
RESULT:3 = EXISTVAR(NAME_SOURCE(), MODE_VALUE())
RESULT:4 = FLAG:0
RETURN RESULT:0
@NAME_SOURCE
#FUNCTIONS
FLAG:0 = FLAG:0 * 10 + 1
RETURNF "FLAG"
@MODE_VALUE
#FUNCTION
FLAG:0 = FLAG:0 * 10 + 2
RETURNF FLAG:1
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    for (index, value) in [(0, 1), (1, 12), (2, 12), (3, 1), (4, 121)] {
        assert_method_watch(&vm, &artifact, "RESULT", index, VmValue::Integer(value));
    }
    assert_method_watch(&vm, &artifact, "RESULTS", 0, VmValue::String("1".into()));
}

#[test]
fn existvar_expression_probe_resolves_without_reading_cells_or_executing_terms() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
#DIM LOCAL_ONLY, 2
RESULT:0 = EXISTVAR("LOCAL_ONLY")
RESULT:1 = EXISTVAR("LOCAL_ONLY:999999", 1)
RESULT:2 = EXISTVAR("1 / 0", 1)
RESULT:3 = EXISTVAR("SIDE()", 1)
RESULT:4 = EXISTVAR("GETTIME()", 1)
RESULT:5 = EXISTVAR("FLAG:\"not a real key\"", 1)
RESULT:6 = EXISTVAR("", 1)
RESULT:7 = EXISTVAR("NO_SUCH_VARIABLE", 1)
RESULT:8 = EXISTVAR("1 +", 1)
CALL CHECK_REF, LOCAL_ONLY
RETURN RESULT:0
@SIDE
#FUNCTION
FLAG:0 += 1
RETURNF FLAG:99999999
@CHECK_REF(VALUES)
#DIM REF VALUES
RESULT:9 = EXISTVAR("VALUES:999999", 1)
RETURN RESULT:0
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    for (index, value) in [
        (0, 1),
        (1, 1),
        (2, 1),
        (3, 1),
        (4, 1),
        (5, 1),
        (6, 1),
        (7, 0),
        (8, 0),
        (9, 1),
    ] {
        assert_method_watch(&vm, &artifact, "RESULT", index, VmValue::Integer(value));
    }
    assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(0));
}

#[test]
fn existvar_catches_only_second_source_script_failure_and_preserves_effects() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:0 = EXISTVAR(NAME_SOURCE(), 1)
FLAG:1 = 1
RETURN
@NAME_SOURCE
#FUNCTIONS
FLAG:0 += 1
IF FLAG:0 == 2
RESULT:9 = FLAG:99999999
ENDIF
RETURNF "FLAG"
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_method_watch(&vm, &artifact, "RESULT", 0, VmValue::Integer(0));
    assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(2));
    assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(1));
    for source in [
        "@SYSTEM_TITLE\nRESULT = EXISTVAR(BAD_NAME(), 1)\nFLAG:1 = 1\nRETURN\n@BAD_NAME\n#FUNCTIONS\nRESULT:9 = FLAG:99999999\nRETURNF \"FLAG\"\n",
        "@SYSTEM_TITLE\nRESULT = EXISTVAR(\"FLAG\", BAD_MODE())\nFLAG:1 = 1\nRETURN\n@BAD_MODE\n#FUNCTION\nRETURNF FLAG:99999999\n",
    ] {
        let artifact = compile_source_with_options(source, &method_options(true));
        let (vm, report) = run_entry(&artifact, VmConfig::default());
        assert!(matches!(
            take_fault(report).category,
            erabasic_vm::FaultCategory::Script(_)
        ));
        assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(0));
    }
}

#[test]
fn existvar_second_source_wait_preserves_checkpoint_and_rejects_deleted_snapshot_state() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:0 = EXISTVAR(NAME_SOURCE(), 1)
RETURN RESULT:0
@NAME_SOURCE
#FUNCTIONS
FLAG:0 += 1
IF FLAG:0 == 2
INPUT
ENDIF
RETURNF "FLAG:999999"
"#,
        &method_options(true),
    );
    let entry = artifact
        .functions
        .iter()
        .find(|f| f.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let suspended = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    let request = suspended
        .events
        .iter()
        .find_map(|event| match event {
            VmEvent::HostPending { request, .. } => Some(*request),
            _ => None,
        })
        .unwrap();
    let saved = serde_json::to_value(vm.snapshot(&natives).unwrap()).unwrap();
    let mut bad = saved.clone();
    bad["fibers"][fiber.0.to_string()]["frames"][0]["existvar_checks"] = serde_json::json!([]);
    let mut rejected = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    assert!(
        Vm::restore_snapshot(
            validated(&artifact),
            VmConfig::default(),
            serde_json::from_value(bad).unwrap(),
            &mut rejected,
            &mut natives
        )
        .is_err()
    );
    assert!(rejected.rebound.is_empty());
    let mut restored = Vm::restore_snapshot(
        validated(&artifact),
        VmConfig::default(),
        serde_json::from_value(saved).unwrap(),
        &mut host,
        &mut natives,
    )
    .unwrap();
    restored.resume_host(request, HostReady::empty()).unwrap();
    let report = restored.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert_eq!(report.stop, erabasic_vm::VmRunStop::Idle);
    assert!(
        report.events.iter().any(|event| matches!(event,
            VmEvent::FiberCompleted { fiber: completed, value: Some(VmValue::Integer(1)) }
            if *completed == fiber
        )),
        "{report:?}"
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_method_watch(&restored, &artifact, "RESULT", 0, VmValue::Integer(1));
    assert_method_watch(&restored, &artifact, "FLAG", 0, VmValue::Integer(2));
}

#[test]
fn existvar_probe_preserves_rand_and_character_parse_policy_without_cell_reads() {
    for compatible_rand in [false, true] {
        for system_no_target in [false, true] {
            let mut options = method_options(true);
            options.compatible_rand = compatible_rand;
            options.system_no_target = system_no_target;
            let artifact = compile_source_with_options(
                r#"@SYSTEM_TITLE
RESULT:0 = EXISTVAR("RAND", 1)
RESULT:1 = EXISTVAR("RAND:0", 1)
RESULT:2 = EXISTVAR("RAND:(+0)", 1)
RESULT:3 = EXISTVAR("RAND:(-0)", 1)
RESULT:4 = EXISTVAR("RAND:(0 + 0)", 1)
RESULT:5 = EXISTVAR("CFLAG:0", 1)
RESULT:6 = EXISTVAR("CFLAG:0:0", 1)
RESULT:7 = EXISTVAR("CFLAG", 1)
RETURN RESULT:0
"#,
                &options,
            );
            assert_eq!(artifact.call_compatibility.compatible_rand, compatible_rand);
            assert_eq!(
                artifact.call_compatibility.system_no_target,
                system_no_target
            );
            let (vm, report) = run_entry(&artifact, VmConfig::default());
            assert!(
                !report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
                "{report:?}"
            );
            for (index, value) in [
                (0, i64::from(compatible_rand)),
                (1, i64::from(compatible_rand)),
                (2, i64::from(compatible_rand)),
                (3, 1),
                (4, 1),
                (5, i64::from(!system_no_target)),
                (6, 1),
                (7, 1),
            ] {
                assert_method_watch(&vm, &artifact, "RESULT", index, VmValue::Integer(value));
            }
        }
    }
}

fn pending_lease_snapshot_artifact() -> BytecodeArtifact {
    compile_source_with_options(
        r#"@SYSTEM_TITLE
#DIM DYNAMIC VALUES, 2
VALUES:0 = 5
RESULT:8 = RAND:1000000
RESULT:6 = GETMETH("VALUE_LEASE", , 1, 2)
IF FLAG:7
RESULT:8 = ABS(FLAG:7)
ENDIF
RESULT:10 = GETMETH("PAIR_LEASE", , VALUES, GETMETH("VALUE_LEASE", , 2, WAIT_LEASE()))
FLAG:9 = VALUES:0
RETURN
@PAIR_LEASE(ITEMS, RIGHT)
#FUNCTION
#DIM REF ITEMS
#DIM DYNAMIC RIGHT
ITEMS:0 += 4
RETURNF ITEMS:0 * 100 + RIGHT
@VALUE_LEASE(LEFT, RIGHT)
#FUNCTION
#DIM DYNAMIC LEFT
#DIM DYNAMIC RIGHT
RETURNF LEFT * 10 + RIGHT
@WAIT_LEASE
#FUNCTION
INPUT
RETURNF RESULT:0
@PROBE_TITLE
RESULT:10 = GETMETH("IDENTITY_LEASE", , EXISTVAR(PROBE_LEASE_SOURCE(), 1))
FLAG:9 = 1
RETURN
@IDENTITY_LEASE(VALUE)
#FUNCTION
#DIM DYNAMIC VALUE
RETURNF VALUE
@PROBE_LEASE_SOURCE
#FUNCTIONS
FLAG:0 += 1
IF FLAG:0 == 2
INPUT
ENDIF
RETURNF "FLAG:0"
@JUMP_TITLE
CALL JUMP_OWNER
FLAG:9 = 1
RETURN
@JUMP_OWNER
#DIM DYNAMIC VALUES, 2
VALUES:0 = 5
JUMPFORM JUMP_WAIT(VALUES)
FLAG:8 = 1
RETURN
@JUMP_WAIT(ITEMS)
#DIM REF ITEMS
INPUT
ITEMS:0 += 1
RESULT:10 = ITEMS:0
RETURN
@FAULT_TITLE
RESULT:10 = GETMETH("VALUE_LEASE", , 1, 1 + FLAG:99999999)
RETURN
@SNAPSHOT_WAIT
INPUT
RETURN
"#,
        &method_options(true),
    )
}

fn lease_snapshot_natives(
    artifact: &BytecodeArtifact,
) -> (
    NativeServiceRegistry,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct RestoreCounter(Arc<AtomicUsize>);
    impl NativeService for RestoreCounter {
        fn call(
            &mut self,
            _: NativeCallRequest,
        ) -> Result<NativeReady, erabasic_vm::ExecutionFailure> {
            Ok(NativeReady {
                value: Some(VmValue::Integer(0)),
                writes: Vec::new(),
            })
        }
        fn snapshot(&self) -> Result<Option<Vec<u8>>, String> {
            Ok(Some(vec![17]))
        }
        fn restore(&mut self, bytes: &[u8]) -> Result<(), String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            if bytes != [17] {
                return Err("invalid restore-counter state".into());
            }
            Ok(())
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let mut natives = NativeServiceRegistry::for_artifact_with_seed(artifact, 123_456);
    let key = artifact
        .native_imports
        .iter()
        .find(|native| native.import.name.eq_ignore_ascii_case("ABS"))
        .unwrap()
        .import
        .key;
    natives.register(key, RestoreCounter(Arc::clone(&calls)));
    (natives, calls)
}

#[test]
#[allow(clippy::too_many_lines)] // Keep each fixture and its lifecycle assertions together.
fn pending_call_snapshot_lease_list_must_match_validated_cfg_before_native_restore() {
    use std::sync::atomic::Ordering;
    let artifact = pending_lease_snapshot_artifact();
    for entry_name in ["SYSTEM_TITLE", "PROBE_TITLE"] {
        let entry = artifact
            .functions
            .iter()
            .find(|function| function.name == entry_name)
            .unwrap()
            .key;
        let (mut natives, _) = lease_snapshot_natives(&artifact);
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
        let mut host = PendingHost {
            stability: HostWaitStability::StableInput,
            rebound: Vec::new(),
        };
        let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
        let request = report
            .events
            .iter()
            .find_map(|event| match event {
                VmEvent::HostPending { request, .. } => Some(*request),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{entry_name}: {report:?}"));
        let snapshot = vm.snapshot(&natives).unwrap();
        let original = serde_json::to_value(&snapshot).unwrap();
        let frame = &original["fibers"][fiber.0.to_string()]["frames"][0];
        assert_eq!(
            frame["user_calls"].as_array().unwrap().len(),
            if entry_name == "SYSTEM_TITLE" { 2 } else { 1 }
        );
        assert_eq!(
            frame["existvar_checks"].as_array().unwrap().len(),
            usize::from(entry_name == "PROBE_TITLE")
        );
        let mut attacks = vec![
            "delete_calls",
            "delete_calls_and_stack",
            "duplicate_call",
            "operand_budget",
        ];
        if entry_name == "SYSTEM_TITLE" {
            attacks.extend(["rewind_progress", "forge_compatible_origin", "forge_ref"]);
        } else {
            attacks.extend(["delete_probe", "duplicate_probe", "forge_probe_origin"]);
        }
        for attack in attacks {
            let mut corrupted = original.clone();
            let frame = &mut corrupted["fibers"][fiber.0.to_string()]["frames"][0];
            match attack {
                "delete_calls" => frame["user_calls"] = serde_json::json!([]),
                "delete_calls_and_stack" => {
                    frame["user_calls"] = serde_json::json!([]);
                    frame["stack"] = serde_json::json!([]);
                }
                "duplicate_call" => {
                    let copy = frame["user_calls"][0].clone();
                    frame["user_calls"].as_array_mut().unwrap().push(copy);
                }
                "rewind_progress" => {
                    // These fields remain mutually consistent; only the verified IP
                    // proves that the first retained actual was already captured.
                    frame["user_calls"][1]["next_slot"] = serde_json::json!(0);
                    frame["user_calls"][1]["captured"] = serde_json::json!([]);
                }
                "forge_compatible_origin" => {
                    let current =
                        usize::try_from(frame["user_calls"][1]["resolve"].as_u64().unwrap())
                            .unwrap();
                    let code = &artifact
                        .functions
                        .iter()
                        .find(|function| function.key == entry)
                        .unwrap()
                        .code;
                    let earlier = code[..current]
                        .iter()
                        .position(|instruction| {
                            instruction.opcode == Opcode::ResolveUserCall as u16
                                && instruction.payload == code[current].payload
                        })
                        .expect("same-shape earlier call");
                    frame["user_calls"][1]["resolve"] = serde_json::json!(earlier);
                }
                "forge_ref" => {
                    frame["user_calls"][0]["captured"][0] =
                        serde_json::to_value(VmValue::IntegerPlace(Box::default())).unwrap();
                }
                "delete_probe" => frame["existvar_checks"] = serde_json::json!([]),
                "duplicate_probe" => {
                    let copy = frame["existvar_checks"][0].clone();
                    frame["existvar_checks"].as_array_mut().unwrap().push(copy);
                }
                "forge_probe_origin" => frame["existvar_checks"][0]["begin"] = serde_json::json!(0),
                "operand_budget" => {}
                _ => unreachable!(),
            }
            let corrupted: VmSnapshot = serde_json::from_value(corrupted).unwrap();
            let (mut rejected_natives, restored_calls) = lease_snapshot_natives(&artifact);
            let before = vm.encode_snapshot(&rejected_natives).unwrap();
            let mut rejected_host = PendingHost {
                stability: HostWaitStability::StableInput,
                rebound: Vec::new(),
            };
            let mut config = VmConfig::default();
            if attack == "operand_budget" {
                config.maximum_operand_stack = 1;
            }
            assert!(
                matches!(
                    Vm::restore_snapshot(
                        validated(&artifact),
                        config,
                        corrupted,
                        &mut rejected_host,
                        &mut rejected_natives
                    ),
                    Err(VmError::Snapshot(_))
                ),
                "{entry_name}/{attack}"
            );
            assert_eq!(
                restored_calls.load(Ordering::SeqCst),
                0,
                "{entry_name}/{attack}: Native restore was invoked"
            );
            assert!(rejected_host.rebound.is_empty(), "{entry_name}/{attack}");
            assert_eq!(
                vm.encode_snapshot(&rejected_natives).unwrap(),
                before,
                "{entry_name}/{attack}"
            );
        }
        let (mut restored_natives, restored_calls) = lease_snapshot_natives(&artifact);
        let mut restored = Vm::restore_snapshot(
            validated(&artifact),
            VmConfig::default(),
            snapshot,
            &mut host,
            &mut restored_natives,
        )
        .unwrap();
        assert_eq!(restored_calls.load(Ordering::SeqCst), 1);
        restored
            .write_variable(
                named_key(&artifact, "RESULT"),
                &[0],
                None,
                VmValue::Integer(3),
            )
            .unwrap();
        restored.resume_host(request, HostReady::empty()).unwrap();
        let report = restored.run_slice(
            &mut ReadyHost::default(),
            &mut restored_natives,
            RunBudget::default(),
        );
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{report:?}"
        );
        assert!(
            report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
            "{report:?}"
        );
        assert_method_watch(
            &restored,
            &artifact,
            "RESULT",
            10,
            VmValue::Integer(if entry_name == "SYSTEM_TITLE" { 923 } else { 1 }),
        );
        assert_method_watch(
            &restored,
            &artifact,
            "FLAG",
            9,
            VmValue::Integer(if entry_name == "SYSTEM_TITLE" { 9 } else { 1 }),
        );
        if entry_name == "PROBE_TITLE" {
            assert_method_watch(&restored, &artifact, "FLAG", 0, VmValue::Integer(2));
        }
    }
}

#[test]
fn pending_jump_snapshot_uses_validated_terminal_stack_and_keeps_local_ref_alive() {
    let artifact = pending_lease_snapshot_artifact();
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "JUMP_TITLE")
        .unwrap()
        .key;
    let (mut natives, _) = lease_snapshot_natives(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    let request = report
        .events
        .iter()
        .find_map(|event| match event {
            VmEvent::HostPending { request, .. } => Some(*request),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{report:?}"));
    let snapshot = vm.snapshot(&natives).unwrap();
    let json = serde_json::to_value(&snapshot).unwrap();
    let owner = &json["fibers"][fiber.0.to_string()]["frames"][1];
    let function = serde_json::from_value(owner["function"].clone()).unwrap();
    let instruction = usize::try_from(owner["instruction"].as_u64().unwrap()).unwrap();
    let validated = validated(&artifact);
    assert!(
        validated
            .operand_stacks()
            .before(function, instruction)
            .is_none()
    );
    assert!(
        validated
            .operand_stacks()
            .terminal_user_call(function, instruction - 1)
            .is_some()
    );
    let mut restored = Vm::restore_snapshot(
        validated,
        VmConfig::default(),
        snapshot,
        &mut host,
        &mut natives,
    )
    .unwrap();
    restored.resume_host(request, HostReady::empty()).unwrap();
    let report = restored.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert_method_watch(&restored, &artifact, "RESULT", 10, VmValue::Integer(6));
    assert_method_watch(&restored, &artifact, "FLAG", 8, VmValue::Integer(0));
    assert_method_watch(&restored, &artifact, "FLAG", 9, VmValue::Integer(1));
}

#[test]
fn faulted_snapshot_keeps_partial_operand_diagnostics_but_no_active_leases() {
    let artifact = pending_lease_snapshot_artifact();
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "FAULT_TITLE")
        .unwrap()
        .key;
    let (mut natives, _) = lease_snapshot_natives(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    let fault = take_fault(report);
    assert_eq!(fault.code, VmFaultCode::Bounds);
    // A faulted primary is deliberately not a stable save point. The existing
    // VM contract can retain faulted secondary fibers beside a stable input root.
    assert!(vm.snapshot(&natives).is_err());
    let root = artifact
        .functions
        .iter()
        .find(|function| function.name == "SNAPSHOT_WAIT")
        .unwrap()
        .key;
    let primary = vm.spawn_entry(root, Vec::new()).unwrap();
    vm.set_primary_fiber(primary).unwrap();
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    vm.run_slice(&mut host, &mut natives, RunBudget::default());
    let snapshot = vm.snapshot(&natives).unwrap();
    let json = serde_json::to_value(&snapshot).unwrap();
    for frame in json["fibers"][fiber.0.to_string()]["frames"]
        .as_array()
        .unwrap()
    {
        assert_eq!(frame["user_calls"], serde_json::json!([]));
        assert_eq!(frame["existvar_checks"], serde_json::json!([]));
        assert!(frame["runtime_form"].is_null());
    }
    let restored = Vm::restore_snapshot(
        validated(&artifact),
        VmConfig::default(),
        snapshot,
        &mut host,
        &mut natives,
    )
    .unwrap();
    assert_eq!(
        restored.fiber_status(fiber),
        Some(FiberStatus::Faulted(fault))
    );
}

#[test]
fn strformcheck_catches_special_native_domains_and_retains_prior_effects() {
    for body in [
        "ARRAYREMOVE FLAG, -1, 1",
        "VARSET FLAG, 3, 0, 9999999",
        "ARRAYCOPY \"MISSING_ARRAY\", \"FLAG\"",
        "ADDCHARA 9999999",
        "DELCHARA -1",
        "RESULT:9 = CSVNAME(9999999) == \"unused\"",
        "RESULT:9 = SUMARRAY(RESULTS, 0, 1)",
        "RESULT:9 = FINDELEMENT(FLAG, \"wrong scalar type\", 0, 1)",
    ] {
        let source = format!(
            "@SYSTEM_TITLE\nRESULT:0 = STRFORMCHECK(\"{{FAILURE()}}\")\nRESULT:1 = FLAG:0\nRETURN\n@FAILURE\n#FUNCTION\nFLAG:0 += 1\n{body}\nFLAG:1 = 1\nRETURNF 99\n"
        );
        let artifact = compile_source_with_options(&source, &method_options(true));
        let (vm, report) = run_entry(&artifact, VmConfig::default());
        assert!(
            report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
            "{body}: {report:?}"
        );
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{body}: {report:?}"
        );
        assert_method_watch(&vm, &artifact, "RESULT", 0, VmValue::Integer(0));
        assert_method_watch(&vm, &artifact, "RESULT", 1, VmValue::Integer(1));
        assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(0));
    }
}

#[test]
fn strformcheck_randdata_failure_is_atomic_and_keeps_the_original_rng() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
DUMPRAND
FLAG:2 = RANDDATA:624
RESULT:0 = STRFORMCHECK("{BAD_STATE()}")
DUMPRAND
RESULT:1 = RANDDATA:624 == FLAG:2
RESULT:2 = FLAG:0
RETURN
@BAD_STATE
#FUNCTION
FLAG:0 += 1
RANDDATA:624 = 625
INITRAND
FLAG:1 = 1
RETURNF 99
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    for (index, expected) in [(0, 0), (1, 1), (2, 1)] {
        assert_method_watch(&vm, &artifact, "RESULT", index, VmValue::Integer(expected));
    }
    assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(0));
}

#[test]
fn strformcheck_does_not_catch_a_missing_random_provider() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:0 = STRFORMCHECK("{DUMP_STATE()}")
FLAG:1 = 1
RETURN
@DUMP_STATE
#FUNCTION
FLAG:0 += 1
DUMPRAND
RETURNF 99
"#,
        &method_options(true),
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut NativeServiceRegistry::default(),
        RunBudget::default(),
    );
    let fault = take_fault(report);
    assert_eq!(
        fault.category,
        erabasic_vm::FaultCategory::InternalInvariant
    );
    assert_eq!(fault.code, erabasic_vm::VmFaultCode::Native);
    assert_eq!(fault.message, "random native service is not registered");
    assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(1));
    assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(0));
}

#[test]
fn strformcheck_preserves_special_native_success_sentinels() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:0 = STRFORMCHECK("{NO_MATCH()}")
RESULT:1 = FLAG:0
RETURN RESULT:0
@NO_MATCH
#FUNCTION
ARRAYREMOVE FLAG, 9999999, 1
PICKUPCHARA -1
FLAG:0 = GETCHARA(9999999)
RETURNF FLAG:0
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_method_watch(&vm, &artifact, "RESULT", 0, VmValue::Integer(1));
    assert_method_watch(&vm, &artifact, "RESULT", 1, VmValue::Integer(-1));
}

#[test]
fn arrayshift_extreme_offsets_do_not_panic_inside_a_checked_method() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
FLAG:0 = -9223372036854775807 - 1
RESULT:0 = STRFORMCHECK("{SHIFT()}")
RESULT:1 = TFLAG:0
RETURN RESULT:0
@SHIFT
#FUNCTION
TFLAG:0 = 3
ARRAYSHIFT TFLAG, FLAG:0, 9, 0, 1
RETURNF TFLAG:0
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_method_watch(&vm, &artifact, "RESULT", 0, VmValue::Integer(1));
    assert_method_watch(&vm, &artifact, "RESULT", 1, VmValue::Integer(9));
}
// Append to the existing tests/vm/strform.rs; not a new test module.

#[test]
fn call_text_six_modes_accept_both_argument_syntaxes_and_preserve_jump_return() {
    for command in [
        "CALLSTR",
        "JUMPSTR",
        "TRYCALLSTR",
        "TRYJUMPSTR",
        "TRYCCALLSTR",
        "TRYCJUMPSTR",
    ] {
        for call in ["TAKE(7)", "TAKE, 7"] {
            let catch = if command.starts_with("TRYC") && !command.starts_with("TRYCALL") {
                "CATCH\nFLAG:2 = 99\nENDCATCH\n"
            } else {
                ""
            };
            let source = format!(
                "@SYSTEM_TITLE\nCALL OUTER\nFLAG:9 = 1\nRETURN\n@OUTER\n{command} {quoted}\nFLAG:1 = 1\n{catch}FLAG:3 = 1\nRETURN\n@TAKE(ARG)\nFLAG:0 = ARG\nRETURN\n",
                quoted = serde_json::to_string(call).unwrap(),
            );
            let artifact = compile_source_with_options(&source, &method_options(true));
            let (vm, report) = run_entry(&artifact, VmConfig::default());
            assert!(
                report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
                "{command} {call}: {report:?}"
            );
            assert!(
                !report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
                "{command} {call}: {report:?}"
            );
            let jumps = command.contains("JUMP");
            for (index, expected) in [
                (0, 7),
                (1, i64::from(!jumps)),
                (2, 0),
                (3, i64::from(!jumps)),
                (9, 1),
            ] {
                assert_method_watch(&vm, &artifact, "FLAG", index, VmValue::Integer(expected));
            }
        }
    }
}

#[test]
fn blank_call_text_jump_is_successful_fallthrough_including_catch_mode() {
    for command in ["JUMPSTR", "TRYJUMPSTR", "TRYCJUMPSTR"] {
        let catch = if command == "TRYCJUMPSTR" {
            "CATCH\nFLAG:1 = 99\nENDCATCH\n"
        } else {
            ""
        };
        let artifact = compile_source_with_options(
            &format!("@SYSTEM_TITLE\n{command} \"   \"\nFLAG:0 = 1\n{catch}FLAG:2 = 2\nRETURN\n"),
            &method_options(true),
        );
        let (vm, report) = run_entry(&artifact, VmConfig::default());
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{report:?}"
        );
        assert!(
            report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
            "{report:?}"
        );
        for (index, value) in [(0, 1), (1, 0), (2, 2)] {
            assert_method_watch(&vm, &artifact, "FLAG", index, VmValue::Integer(value));
        }
    }
}

#[test]
fn snake_statement_extra_actuals_skip_side_effects_and_original_dynamic_calls_are_strict() {
    for (statement, method, continued) in [
        ("CALL TAKE, 7, SIDE()", false, 1),
        ("CALLFORM TAKE(7, SIDE())", false, 1),
        ("TRYCALLFORM TAKE(7, SIDE())", false, 1),
        ("CALLFORMF TAKE(7, SIDE())", true, 1),
        ("TRYCALLFORMF TAKE(7, SIDE())", true, 1),
        (
            "TRYCALLLIST\nFUNC MISSING, SIDE()\nFUNC TAKE, 7, SIDE()\nENDFUNC",
            false,
            1,
        ),
        (
            "TRYJUMPLIST\nFUNC MISSING, SIDE()\nFUNC TAKE, 7, SIDE()\nENDFUNC",
            false,
            0,
        ),
    ] {
        let kind = if method { "#FUNCTION\n" } else { "" };
        let returned = if method { "RETURNF ARG" } else { "RETURN" };
        let source = format!(
            "@SYSTEM_TITLE\n{statement}\nFLAG:2 = 1\nRETURN\n@TAKE(ARG)\n{kind}FLAG:1 = ARG\n{returned}\n@SIDE\n#FUNCTION\nFLAG:0 += 1\nRETURNF FLAG:9999999\n"
        );
        let artifact = compile_source_with_options(&source, &method_options(true));
        let (vm, report) = run_entry(&artifact, VmConfig::default());
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{statement}: {report:?}"
        );
        assert!(
            report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
            "{report:?}"
        );
        for (index, value) in [(0, 0), (1, 7), (2, continued)] {
            assert_method_watch(&vm, &artifact, "FLAG", index, VmValue::Integer(value));
        }
    }
    // Dynamic resolution keeps the strict original policy observable at execution.
    let artifact = compile_source_with_options(
        "@SYSTEM_TITLE\nCALLFORM TAKE(7, SIDE())\nFLAG:2 = 1\nRETURN\n@TAKE(ARG)\nFLAG:1 = ARG\nRETURN\n@SIDE\n#FUNCTION\nFLAG:0 += 1\nRETURNF 8\n",
        &method_options(false),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(matches!(
        take_fault(report).category,
        erabasic_vm::FaultCategory::Script(erabasic_vm::ScriptFaultKind::Argument)
    ));
    for index in 0..3 {
        assert_method_watch(&vm, &artifact, "FLAG", index, VmValue::Integer(0));
    }
}

#[test]
fn call_text_jump_keeps_caller_local_ref_alive_through_recursive_forwarding() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
CALL OWNER
FLAG:9 = 1
RETURN
@OWNER
#LOCALSIZE 2
LOCAL:0 = 40
JUMPSTR "FORWARD(LOCAL:SIDE())"
FLAG:8 = 99
RETURN
@FORWARD(VALUES)
#DIM REF VALUES
CALLSTR "RECURSE(VALUES, 2, SIDE())"
FLAG:1 = VALUES:0
RETURN
@RECURSE(VALUES, ARG)
#DIM REF VALUES
IF ARG > 0
CALLSTR "RECURSE(VALUES, ARG - 1)"
ENDIF
VALUES:0 += 1
RETURN
@SIDE
#FUNCTION
FLAG:0 += 1
RETURNF FLAG:9999999
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    for (index, value) in [(0, 0), (1, 43), (8, 0), (9, 1)] {
        assert_method_watch(&vm, &artifact, "FLAG", index, VmValue::Integer(value));
    }
}

#[test]
fn checked_form_child_failure_preserves_ref_writes_and_allows_a_fresh_call() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
#DIM DYNAMIC VALUES, 2
VALUES:0 = 10
RESULT:0 = STRFORMCHECK("{FAIL_WITH_REF(VALUES)}")
RESULT:1 = GOOD_WITH_REF(VALUES)
FLAG:9 = VALUES:0
RETURN
@FAIL_WITH_REF(ITEMS)
#FUNCTION
#DIM REF ITEMS
ITEMS:0 += 1
RETURNF ITEMS:9999999
@GOOD_WITH_REF(ITEMS)
#FUNCTION
#DIM REF ITEMS
ITEMS:0 += 2
RETURNF ITEMS:0
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert_method_watch(&vm, &artifact, "RESULT", 0, VmValue::Integer(0));
    assert_method_watch(&vm, &artifact, "RESULT", 1, VmValue::Integer(13));
    assert_method_watch(&vm, &artifact, "FLAG", 9, VmValue::Integer(13));
}

fn checked_input_runtime() -> (
    RuntimeVm,
    BytecodeArtifact,
    FiberId,
    erabasic_vm::VmHostRequest,
) {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:10 = 73
RESULT:10 = STRFORMCHECK("{WAITING_METHOD()}")
FLAG:9 = 1
RETURN
@WAITING_METHOD
#FUNCTION
FLAG:0 += 1
INPUT
FLAG:1 += 1
RETURNF 7
"#,
        &method_options(true),
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut runtime = RuntimeVm::new(validated(&artifact), VmConfig::default());
    let fiber = runtime.spawn_entry(entry, Vec::new()).unwrap();
    let report = runtime.drive(RunBudget::default(), VmDriveMode::Normal);
    let request = report
        .events
        .into_iter()
        .find_map(|event| match event {
            VmPortEvent::HostCall(request) => Some(request),
            _ => None,
        })
        .expect("checked method must reach INPUT through the real runtime port");
    let prepared = runtime
        .validate_host_completion(
            request.id,
            VmHostCompletion::Pending {
                stability: HostWaitStability::StableInput,
                rebind_payload: Vec::new(),
            },
        )
        .unwrap();
    runtime.commit_host_completion(prepared).unwrap();
    assert_eq!(
        runtime.fiber_status(fiber),
        Some(FiberStatus::WaitingHost(request.id))
    );
    (runtime, artifact, fiber, request)
}

#[test]
fn checked_form_async_input_success_and_typed_failures_keep_their_categories() {
    use erabasic_vm::{ExecutionFailure, FaultCategory, ScriptFaultKind};
    for category in [
        None,
        Some(FaultCategory::Script(ScriptFaultKind::Operation)),
        Some(FaultCategory::HostContract),
        Some(FaultCategory::Permission),
        Some(FaultCategory::ResourceLimit),
        Some(FaultCategory::Cancellation),
    ] {
        let (mut runtime, artifact, fiber, request) = checked_input_runtime();
        let completion = category.map_or_else(
            || VmHostCompletion::Ready(HostReady::empty()),
            |category| {
                VmHostCompletion::Error(ExecutionFailure::classified(
                    category,
                    VmFaultCode::Host,
                    "same legacy host message",
                ))
            },
        );
        let prepared = runtime
            .validate_host_completion(request.id, completion)
            .unwrap();
        runtime.commit_host_completion(prepared).unwrap();
        let report = runtime.drive(RunBudget::default(), VmDriveMode::Normal);
        let catchable =
            category.is_some_and(|category| matches!(category, FaultCategory::Script(_)));
        if category.is_none() || catchable {
            assert!(
                matches!(runtime.fiber_status(fiber), Some(FiberStatus::Completed(_))),
                "{category:?}: {report:?}"
            );
            assert_method_watch(
                runtime.vm(),
                &artifact,
                "RESULT",
                10,
                VmValue::Integer(i64::from(!catchable)),
            );
            assert_method_watch(runtime.vm(), &artifact, "FLAG", 9, VmValue::Integer(1));
        } else {
            let Some(FiberStatus::Faulted(fault)) = runtime.fiber_status(fiber) else {
                panic!("{category:?}: {report:?}");
            };
            assert_eq!(Some(fault.category), category);
            assert_method_watch(runtime.vm(), &artifact, "RESULT", 10, VmValue::Integer(73));
            assert_method_watch(runtime.vm(), &artifact, "FLAG", 9, VmValue::Integer(0));
        }
        assert_method_watch(runtime.vm(), &artifact, "FLAG", 0, VmValue::Integer(1));
        assert_method_watch(
            runtime.vm(),
            &artifact,
            "FLAG",
            1,
            VmValue::Integer(i64::from(category.is_none())),
        );
        assert!(
            runtime
                .validate_host_completion(request.id, VmHostCompletion::Ready(HostReady::empty()))
                .is_err()
        );
    }
}

#[test]
fn cancelling_checked_input_does_not_complete_the_checker_or_accept_late_input() {
    let (mut runtime, artifact, fiber, request) = checked_input_runtime();
    runtime.cancel_fiber(fiber).unwrap();
    assert_eq!(runtime.fiber_status(fiber), Some(FiberStatus::Cancelled));
    assert!(
        runtime
            .validate_host_completion(request.id, VmHostCompletion::Ready(HostReady::empty()))
            .is_err()
    );
    runtime.drive(RunBudget::default(), VmDriveMode::Normal);
    assert_method_watch(runtime.vm(), &artifact, "RESULT", 10, VmValue::Integer(73));
    assert_method_watch(runtime.vm(), &artifact, "FLAG", 9, VmValue::Integer(0));
}

#[test]
fn checked_form_does_not_catch_malformed_native_ready_value_or_write() {
    struct MalformedReady(bool);
    impl NativeService for MalformedReady {
        fn call(
            &mut self,
            _request: NativeCallRequest,
        ) -> Result<NativeReady, erabasic_vm::ExecutionFailure> {
            Ok(if self.0 {
                NativeReady::value(VmValue::String("wrong return type".into()))
            } else {
                NativeReady {
                    value: Some(VmValue::Integer(1)),
                    writes: vec![erabasic_vm::HostWrite {
                        target: erabasic_vm::PlaceDescriptor {
                            variable: SymbolKey::derive("test", b"missing-host-write"),
                            ..Default::default()
                        },
                        value: VmValue::Integer(2),
                    }],
                }
            })
        }
    }
    let artifact = compile_source_with_options(
        "@SYSTEM_TITLE\nRESULT:10 = 73\nRESULT:10 = STRFORMCHECK(\"{ABS(FLAG:0)}\")\nFLAG:9 = 1\nFLAG:8 = ABS(FLAG:0)\nRETURN\n",
        &method_options(true),
    );
    let key = artifact
        .native_imports
        .iter()
        .find(|native| native.import.name.eq_ignore_ascii_case("abs"))
        .unwrap()
        .import
        .key;
    for wrong_type in [true, false] {
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        natives.register(key, MalformedReady(wrong_type));
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        let entry = artifact
            .functions
            .iter()
            .find(|function| function.name == "SYSTEM_TITLE")
            .unwrap()
            .key;
        vm.spawn_entry(entry, Vec::new()).unwrap();
        let report = vm.run_slice(
            &mut ReadyHost::default(),
            &mut natives,
            RunBudget::default(),
        );
        assert_eq!(
            take_fault(report).category,
            erabasic_vm::FaultCategory::HostContract
        );
        assert_method_watch(&vm, &artifact, "RESULT", 10, VmValue::Integer(73));
        assert_method_watch(&vm, &artifact, "FLAG", 9, VmValue::Integer(0));
    }
}

#[test]
fn call_text_snapshot_rejects_deleted_child_origin_and_forged_root_before_native_restore() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
#LOCALSIZE 2
LOCAL:0 = 5
RESULT:9 = RAND:1000000
CALLSTR "WAIT_CALL(LOCAL)"
FLAG:9 = 1
RETURN
@WAIT_CALL(VALUES)
#DIM REF VALUES
INPUT
VALUES:0 += 1
RETURN
"#,
        &method_options(true),
    );
    let mut natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 1234);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(matches!(
        vm.fiber_status(fiber),
        Some(FiberStatus::WaitingHost(_))
    ));
    let saved = serde_json::to_value(vm.snapshot(&natives).unwrap()).unwrap();
    let frames = &saved["fibers"][fiber.0.to_string()]["frames"];
    assert_eq!(frames.as_array().unwrap().len(), 2);
    assert!(!frames[1]["user_call"].is_null());
    for attack in [
        "delete_child_origin",
        "forge_bytecode_origin",
        "delete_call_root",
        "forge_value_root",
        "forge_jump_mode",
    ] {
        let mut corrupted = saved.clone();
        let frames = &mut corrupted["fibers"][fiber.0.to_string()]["frames"];
        match attack {
            "delete_child_origin" => frames[1]["user_call"] = serde_json::Value::Null,
            "forge_bytecode_origin" => {
                frames[1]["user_call"]["origin"] =
                    serde_json::json!({"Bytecode": {"resolve": 0, "invoke": 0}});
            }
            "delete_call_root" => frames[0]["runtime_form"] = serde_json::Value::Null,
            "forge_value_root" => {
                frames[0]["runtime_form"]["completion"] =
                    serde_json::json!({"Value": BytecodeType::String});
            }
            "forge_jump_mode" => {
                frames[0]["runtime_form"]["completion"]["Call"]["spec"]["mode"] =
                    serde_json::to_value(erabasic_bytecode::CallTextMode::Jump).unwrap();
            }
            _ => unreachable!(),
        }
        let snapshot: VmSnapshot = serde_json::from_value(corrupted).unwrap();
        let mut rejected_natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 9876);
        let before = vm.encode_snapshot(&rejected_natives).unwrap();
        let mut rejected_host = PendingHost {
            stability: HostWaitStability::StableInput,
            rebound: Vec::new(),
        };
        assert!(
            Vm::restore_snapshot(
                validated(&artifact),
                VmConfig::default(),
                snapshot,
                &mut rejected_host,
                &mut rejected_natives
            )
            .is_err(),
            "{attack}"
        );
        assert!(rejected_host.rebound.is_empty(), "{attack}");
        assert_eq!(
            vm.encode_snapshot(&rejected_natives).unwrap(),
            before,
            "{attack}"
        );
    }
}

#[test]
fn original_profile_rejects_snake_call_text_and_checked_form_at_load() {
    for body in ["CALLSTR \"TARGET()\"", "RESULT = STRFORMCHECK(\"plain\")"] {
        let report = analyze_project(
            AnalysisInput {
                project_data: project_data(),
                sources: vec![ProjectSource {
                    relative_path: "profile-gate.erb".into(),
                    payload: SourcePayload::Utf8(format!(
                        "@SYSTEM_TITLE\n{body}\nRETURN\n@TARGET\nRETURN\n"
                    )),
                }],
            },
            &method_options(false),
            &ExtensionRegistry::default(),
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.reference_level >= 2),
            "{body}: {:#?}",
            report.diagnostics
        );
    }
}

#[test]
fn call_text_try_catches_only_argument_reduction_missing_target_and_binding_stages() {
    for (text, caught) in [
        ("TAKE(1 +)", true),
        ("MISSING(7)", true),
        ("TAKE(\"bad\")", true),
        ("TAKE(UNKNOWN_VARIABLE)", true),
        ("TAKE(1, UNKNOWN_VARIABLE)", true),
        ("TAKE(SIDE(), UNKNOWN_VARIABLE)", true),
        ("METHOD(UNKNOWN_VARIABLE)", true),
        ("TAKE(UNKNOWN_METHOD())", true),
        ("TAKE(1 + \"bad\")", true),
        ("METHOD()", false),
        ("TAKE(FLAG:9999999)", false),
        ("BROKEN()", false),
        ("TAKE(\"unterminated)", false),
    ] {
        let artifact = compile_source_with_options(
            &format!(
                "@SYSTEM_TITLE\nTRYCCALLSTR {}\nFLAG:0 = 99\nCATCH\nFLAG:1 = 1\nENDCATCH\nFLAG:2 = 1\nRETURN\n@TAKE(ARG)\nRETURN\n@METHOD\n#FUNCTION\nRETURNF 1\n@BROKEN\nRESULT = FLAG:9999999\nRETURN\n@SIDE\n#FUNCTION\nFLAG:8 += 1\nRETURNF 1\n",
                serde_json::to_string(text).unwrap(),
            ),
            &method_options(true),
        );
        let (vm, report) = run_entry(&artifact, VmConfig::default());
        if caught {
            assert!(
                !report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
                "{text}: {report:?}"
            );
            assert!(
                report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
                "{report:?}"
            );
            assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(1));
            assert_method_watch(&vm, &artifact, "FLAG", 2, VmValue::Integer(1));
        } else {
            assert!(
                matches!(
                    take_fault(report).category,
                    erabasic_vm::FaultCategory::Script(_)
                ),
                "{text}"
            );
            assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(0));
            assert_method_watch(&vm, &artifact, "FLAG", 2, VmValue::Integer(0));
        }
        assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(0));
        assert_method_watch(&vm, &artifact, "FLAG", 8, VmValue::Integer(0));
    }
}

#[test]
#[allow(clippy::too_many_lines)] // Keep each fixture and its lifecycle assertions together.
fn checked_native_failure_restores_checkpoint_and_failed_rollback_is_uncatchable() {
    use std::sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    };
    struct MutatingFailure {
        state: Arc<AtomicU8>,
        fail_restore: bool,
    }
    impl NativeService for MutatingFailure {
        fn call(
            &mut self,
            _request: NativeCallRequest,
        ) -> Result<NativeReady, erabasic_vm::ExecutionFailure> {
            self.state.store(9, Ordering::SeqCst);
            Err(erabasic_vm::ExecutionFailure::script(
                erabasic_vm::ScriptFaultKind::Operation,
                VmFaultCode::Native,
                "script domain failure",
            ))
        }
        fn snapshot(&self) -> Result<Option<Vec<u8>>, String> {
            Ok(Some(vec![self.state.load(Ordering::SeqCst)]))
        }
        fn restore(&mut self, bytes: &[u8]) -> Result<(), String> {
            if self.fail_restore {
                return Err("rollback deliberately unavailable".into());
            }
            let [value] = bytes else {
                return Err("invalid test snapshot".into());
            };
            self.state.store(*value, Ordering::SeqCst);
            Ok(())
        }
    }
    let artifact = compile_source_with_options(
        "@SYSTEM_TITLE\nIF FLAG:7\nRESULT:8 = ABS(FLAG:0)\nENDIF\nRESULT:10 = 73\nRESULT:10 = STRFORMCHECK(\"{ABS(FLAG:0)}\")\nFLAG:9 = 1\nRETURN\n",
        &method_options(true),
    );
    let key = artifact
        .native_imports
        .iter()
        .find(|native| native.import.name.eq_ignore_ascii_case("abs"))
        .unwrap()
        .import
        .key;
    for fail_restore in [false, true] {
        let state = Arc::new(AtomicU8::new(1));
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        natives.register(
            key,
            MutatingFailure {
                state: Arc::clone(&state),
                fail_restore,
            },
        );
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        let entry = artifact
            .functions
            .iter()
            .find(|function| function.name == "SYSTEM_TITLE")
            .unwrap()
            .key;
        vm.spawn_entry(entry, Vec::new()).unwrap();
        let report = vm.run_slice(
            &mut ReadyHost::default(),
            &mut natives,
            RunBudget::default(),
        );
        if fail_restore {
            assert_eq!(
                take_fault(report).category,
                erabasic_vm::FaultCategory::HostContract
            );
            assert_eq!(state.load(Ordering::SeqCst), 9);
            assert_method_watch(&vm, &artifact, "RESULT", 10, VmValue::Integer(73));
        } else {
            assert!(
                !report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
                "{report:?}"
            );
            assert!(
                report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
                "{report:?}"
            );
            assert_eq!(state.load(Ordering::SeqCst), 1);
            assert_method_watch(&vm, &artifact, "RESULT", 10, VmValue::Integer(0));
        }
        assert_method_watch(
            &vm,
            &artifact,
            "FLAG",
            9,
            VmValue::Integer(i64::from(!fail_restore)),
        );
    }
}

fn run_checked_variable_domain_case(local_setup: &str, failing_statement: &str) {
    let source = format!(
        r#"@SYSTEM_TITLE
#DIM DYNAMIC VALUES, 2
VALUES:0 = 10
FLAG:4 = 17
RESULT:10 = 73
RESULT:10 = STRFORMCHECK("{{VARIABLE_FAILURE(VALUES)}}")
RESULT:11 = RECOVERED_VARIABLE_METHOD(VALUES)
RESULT:12 = CHECK_LOCKED
FLAG:9 = VALUES:0
RETURN
@VARIABLE_FAILURE(ITEMS)
#FUNCTION
#DIM REF ITEMS
{local_setup}
ITEMS:0 += 1
{failing_statement}
FLAG:8 = 1
RETURNF 99
@RECOVERED_VARIABLE_METHOD(ITEMS)
#FUNCTION
#DIM REF ITEMS
ITEMS:0 += 2
RETURNF ITEMS:0
"#
    );
    let artifact = compile_with_header(
        "#DIM CONST CHECK_LOCKED = 7\n",
        &source,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{failing_statement}: {report:?}"
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{failing_statement}: {report:?}"
    );
    for (index, value) in [(8, 0), (10, 0), (11, 13), (12, 7)] {
        assert_method_watch(&vm, &artifact, "RESULT", index, VmValue::Integer(value));
    }
    for (index, value) in [(4, 17), (8, 0), (9, 13)] {
        assert_method_watch(&vm, &artifact, "FLAG", index, VmValue::Integer(value));
    }
}

#[test]
fn checked_user_method_catches_dynamic_variable_read_and_named_index_domains() {
    for (setup, statement) in [
        (
            "#DIMS REFERENCE\nREFERENCE '= \"MISSING_CHECK_VARIABLE\"",
            "RESULT:8 = GETVAR(REFERENCE)",
        ),
        (
            "#DIMS KEY\nKEY '= \"MISSING_CHECK_FLAG_NAME\"",
            "RESULT:8 = FLAG:KEY",
        ),
        (
            "#DIMS REFERENCE\nREFERENCE '= \"FLAG:9999999\"",
            "RESULT:8 = GETVAR(REFERENCE)",
        ),
    ] {
        run_checked_variable_domain_case(setup, statement);
    }
}

#[test]
fn checked_user_method_catches_read_only_negative_and_out_of_range_setvar_domains() {
    for reference in ["CHECK_LOCKED", "FLAG:-1", "FLAG:9999999"] {
        let setup = format!("#DIMS REFERENCE\nREFERENCE '= \"{reference}\"");
        run_checked_variable_domain_case(&setup, "RESULT:8 = SETVAR(REFERENCE, 99)");
    }
}

// Exercise real script scopes. Candidate oracle captures are separate from these
// Rust contract assertions and remain pending until the authorized matrix runs.
fn run_nested_checkpoint_contract(
    source: &str,
    expected_integers: &[(&str, u64, i64)],
    expected_string: Option<(&str, u64, &str)>,
) {
    let artifact = compile_source_with_options(source, &method_options(true));
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    for &(name, index, value) in expected_integers {
        assert_method_watch(&vm, &artifact, name, index, VmValue::Integer(value));
    }
    if let Some((name, index, value)) = expected_string {
        assert_method_watch(&vm, &artifact, name, index, VmValue::String(value.into()));
    }
}

#[test]
fn nearest_existvar_checkpoint_precedes_enclosing_strformcheck_in_bytecode_and_form() {
    // nearest-existvar-bytecode
    run_nested_checkpoint_contract(
        r#"@SYSTEM_TITLE
RESULT:10 = 73
RESULT:20 = STRFORMCHECK("{PROBE()}{AFTER()}")
RESULT:21 = STRFORMCHECK("fresh")
FLAG:9 = 1
RETURN RESULT
@PROBE
#FUNCTION
RESULT:10 = EXISTVAR(NAME_SOURCE(), 1)
FLAG:1 += 1
RETURNF 7
@NAME_SOURCE
#FUNCTIONS
FLAG:0 += 1
IF FLAG:0 == 2
RESULT:19 = FLAG:9999999
ENDIF
RETURNF "FLAG"
@AFTER
#FUNCTION
FLAG:2 += 1
RETURNF 8
"#,
        &[
            ("RESULT", 10, 0),
            ("RESULT", 20, 1),
            ("RESULT", 21, 1),
            ("FLAG", 0, 2),
            ("FLAG", 1, 1),
            ("FLAG", 2, 1),
            ("FLAG", 9, 1),
        ],
        None,
    );
    // nearest-existvar-form
    run_nested_checkpoint_contract(
        r#"@SYSTEM_TITLE
RESULT:10 = 73
RESULT:20 = STRFORMCHECK("{PROBE()}{AFTER()}")
RESULT:21 = STRFORMCHECK("fresh")
FLAG:9 = 1
RETURN RESULT
@PROBE
#FUNCTION
RESULTS:1 '= STRFORM("{EXISTVAR(NAME_SOURCE(), 1)}")
FLAG:1 += 1
RETURNF 7
@NAME_SOURCE
#FUNCTIONS
FLAG:0 += 1
IF FLAG:0 == 2
RESULT:19 = FLAG:9999999
ENDIF
RETURNF "FLAG"
@AFTER
#FUNCTION
FLAG:2 += 1
RETURNF 8
"#,
        &[
            ("RESULT", 10, 73),
            ("RESULT", 20, 1),
            ("RESULT", 21, 1),
            ("FLAG", 0, 2),
            ("FLAG", 1, 1),
            ("FLAG", 2, 1),
            ("FLAG", 9, 1),
        ],
        Some(("RESULTS", 1, "0")),
    );
    // nearest-existvar-same-frame
    run_nested_checkpoint_contract(
        r#"@SYSTEM_TITLE
RESULT:10 = 73
RESULT:20 = STRFORMCHECK("{EXISTVAR(NAME_SOURCE(), 1)}{AFTER()}")
RESULT:21 = STRFORMCHECK("fresh")
FLAG:9 = 1
RETURN RESULT
@NAME_SOURCE
#FUNCTIONS
FLAG:0 += 1
IF FLAG:0 == 2
RESULT:19 = FLAG:9999999
ENDIF
RETURNF "FLAG"
@AFTER
#FUNCTION
FLAG:2 += 1
RETURNF 8
"#,
        &[
            ("RESULT", 10, 73),
            ("RESULT", 20, 1),
            ("RESULT", 21, 1),
            ("FLAG", 0, 2),
            ("FLAG", 2, 1),
            ("FLAG", 9, 1),
        ],
        None,
    );
}

#[test]
fn nearest_strformcheck_checkpoint_precedes_enclosing_existvar_in_bytecode_and_form() {
    // nearest-strformcheck-bytecode
    run_nested_checkpoint_contract(
        r#"@SYSTEM_TITLE
RESULT:10 = 73
RESULT:20 = 73
RESULT:20 = EXISTVAR(NAME_SOURCE(), 1)
RESULT:21 = STRFORMCHECK("fresh")
FLAG:9 = 1
RETURN RESULT
@NAME_SOURCE
#FUNCTIONS
FLAG:0 += 1
IF FLAG:0 == 2
RESULT:10 = STRFORMCHECK("{BAD()}")
FLAG:1 += 1
ENDIF
RETURNF "FLAG"
@BAD
#FUNCTION
FLAG:2 += 1
RETURNF FLAG:9999999
"#,
        &[
            ("RESULT", 10, 0),
            ("RESULT", 20, 1),
            ("RESULT", 21, 1),
            ("FLAG", 0, 2),
            ("FLAG", 1, 1),
            ("FLAG", 2, 1),
            ("FLAG", 9, 1),
        ],
        None,
    );
    // nearest-strformcheck-form
    run_nested_checkpoint_contract(
        r#"@SYSTEM_TITLE
RESULT:10 = 73
RESULT:20 = 73
RESULTS:1 '= STRFORM("{EXISTVAR(NAME_SOURCE(), 1)}")
RESULT:21 = STRFORMCHECK("fresh")
FLAG:9 = 1
RETURN RESULT
@NAME_SOURCE
#FUNCTIONS
FLAG:0 += 1
IF FLAG:0 == 2
RESULT:10 = STRFORMCHECK("{BAD()}")
FLAG:1 += 1
ENDIF
RETURNF "FLAG"
@BAD
#FUNCTION
FLAG:2 += 1
RETURNF FLAG:9999999
"#,
        &[
            ("RESULT", 10, 0),
            ("RESULT", 20, 73),
            ("RESULT", 21, 1),
            ("FLAG", 0, 2),
            ("FLAG", 1, 1),
            ("FLAG", 2, 1),
            ("FLAG", 9, 1),
        ],
        Some(("RESULTS", 1, "1")),
    );
    // nearest-strformcheck-same-frame
    run_nested_checkpoint_contract(
        r#"@SYSTEM_TITLE
RESULTS:0 '= "{BAD()}"
RESULT:20 = EXISTVAR(STRFORM("{STRFORMCHECK(RESULTS:0)}"), 1)
RESULT:21 = STRFORMCHECK("fresh")
FLAG:9 = 1
RETURN RESULT
@BAD
#FUNCTION
FLAG:2 += 1
RETURNF FLAG:9999999
"#,
        &[
            ("RESULT", 20, 1),
            ("RESULT", 21, 1),
            ("FLAG", 2, 2),
            ("FLAG", 9, 1),
        ],
        None,
    );
}

#[test]
#[allow(clippy::too_many_lines)] // Keep each fixture and its lifecycle assertions together.
fn outer_check_catches_failed_inner_parameters_in_order_without_entering_inner_scope() {
    // parameter-failure-existvar-first-source
    run_nested_checkpoint_contract(
        r#"@SYSTEM_TITLE
RESULT:10 = 73
RESULT:19 = 61
RESULT:20 = STRFORMCHECK("{CHILD()}{AFTER()}")
RESULT:21 = STRFORMCHECK("fresh")
FLAG:9 = 1
RETURN RESULT
@CHILD
#FUNCTION
RESULT:10 = EXISTVAR(NAME_SOURCE(), MODE_SOURCE())
FLAG:1 += 1
RETURNF 7
@NAME_SOURCE
#FUNCTIONS
FLAG:0 = FLAG:0 * 10 + 1
RESULT:19 = FLAG:9999999
RETURNF "FLAG"
@MODE_SOURCE
#FUNCTION
FLAG:0 = FLAG:0 * 10 + 2
RETURNF 1
@AFTER
#FUNCTION
FLAG:2 += 1
RETURNF 8
"#,
        &[
            ("RESULT", 10, 73),
            ("RESULT", 19, 61),
            ("RESULT", 20, 0),
            ("RESULT", 21, 1),
            ("FLAG", 0, 1),
            ("FLAG", 1, 0),
            ("FLAG", 2, 0),
            ("FLAG", 9, 1),
        ],
        None,
    );
    // parameter-failure-existvar-mode
    run_nested_checkpoint_contract(
        r#"@SYSTEM_TITLE
RESULT:10 = 73
RESULT:19 = 61
RESULT:20 = STRFORMCHECK("{CHILD()}{AFTER()}")
RESULT:21 = STRFORMCHECK("fresh")
FLAG:9 = 1
RETURN RESULT
@CHILD
#FUNCTION
RESULT:10 = EXISTVAR(NAME_SOURCE(), MODE_SOURCE())
FLAG:1 += 1
RETURNF 7
@NAME_SOURCE
#FUNCTIONS
FLAG:0 = FLAG:0 * 10 + 1
RETURNF "FLAG"
@MODE_SOURCE
#FUNCTION
FLAG:0 = FLAG:0 * 10 + 2
RESULT:19 = FLAG:9999999
RETURNF 1
@AFTER
#FUNCTION
FLAG:2 += 1
RETURNF 8
"#,
        &[
            ("RESULT", 10, 73),
            ("RESULT", 19, 61),
            ("RESULT", 20, 0),
            ("RESULT", 21, 1),
            ("FLAG", 0, 12),
            ("FLAG", 1, 0),
            ("FLAG", 2, 0),
            ("FLAG", 9, 1),
        ],
        None,
    );
    // parameter-failure-strformcheck-source
    run_nested_checkpoint_contract(
        r#"@SYSTEM_TITLE
RESULT:10 = 73
RESULT:19 = 61
RESULT:20 = STRFORMCHECK("{CHILD()}{AFTER()}")
RESULT:21 = STRFORMCHECK("fresh")
FLAG:9 = 1
RETURN RESULT
@CHILD
#FUNCTION
RESULT:10 = STRFORMCHECK(NAME_SOURCE())
FLAG:1 += 1
RETURNF 7
@NAME_SOURCE
#FUNCTIONS
FLAG:0 = FLAG:0 * 10 + 1
RESULT:19 = FLAG:9999999
RETURNF "FLAG"
@MODE_SOURCE
#FUNCTION
FLAG:0 = FLAG:0 * 10 + 2
RETURNF 1
@AFTER
#FUNCTION
FLAG:2 += 1
RETURNF 8
"#,
        &[
            ("RESULT", 10, 73),
            ("RESULT", 19, 61),
            ("RESULT", 20, 0),
            ("RESULT", 21, 1),
            ("FLAG", 0, 1),
            ("FLAG", 1, 0),
            ("FLAG", 2, 0),
            ("FLAG", 9, 1),
        ],
        None,
    );
}

#[test]
fn checked_forms_classify_root_and_nested_source_types_before_execution() {
    let forms = [
        r#"{1 + "x"}"#,
        r#"{-"x"}"#,
        r#"{FLAG:"x"}"#,
        r#"{1,"x"}"#,
        r#"{("x" ? 1 # 2)}"#,
        r"%1%",
        r#"{ABS("x")}"#,
    ];
    for form in forms {
        for nested in [false, true] {
            // A formatted expression nested in an interpolation must receive
            // exactly the same source check as the root template.
            let form = if nested {
                format!("%\\@ 1 ? {form} # unused \\@%")
            } else {
                form.to_owned()
            };
            let form = format!("{{EFFECT()}}{form}");
            let escaped = form.replace('\\', "\\\\").replace('"', "\\\"");
            let source = format!(
                "@SYSTEM_TITLE\nIF FLAG:99\nRESULT:9 = ABS(FLAG:99)\nENDIF\nRESULT:0 = STRFORMCHECK(\"{escaped}\")\nRESULT:1 = STRFORMCHECK(\"{{EFFECT()}}\")\nRETURN\n@ORDINARY\nRESULTS:0 '= STRFORM(\"{escaped}\")\nRETURN\n@EFFECT\n#FUNCTION\nFLAG:0 += 1\nRETURNF 7\n"
            );
            let artifact = compile_source_with_options(&source, &method_options(true));
            let (vm, report) = run_entry(&artifact, VmConfig::default());
            assert!(
                report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
                "{form}: {report:?}"
            );
            assert_method_watch(&vm, &artifact, "RESULT", 0, VmValue::Integer(0));
            assert_method_watch(&vm, &artifact, "RESULT", 1, VmValue::Integer(1));
            assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(1));
            let (vm, report) = run_method_case(&artifact, "ORDINARY", VmConfig::default());
            let fault = take_fault(report);
            assert_eq!(
                fault.category,
                erabasic_vm::FaultCategory::Script(erabasic_vm::ScriptFaultKind::Argument),
                "{form}: {fault:?}"
            );
            assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(0));
        }
    }
}

#[test]
fn checked_forms_distinguish_unknown_names_from_missing_native_providers() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
IF FLAG:99
RESULT:9 = ABS(FLAG:99)
ENDIF
RESULT:0 = STRFORMCHECK("{ABS(-3)}")
RESULT:1 = STRFORMCHECK("{UNKNOWN_PROVIDER_NAME(3)}")
FLAG:0 = 1
RETURN RESULT:0
"#,
        &method_options(true),
    );
    assert!(
        artifact
            .native_imports
            .iter()
            .any(|native| native.import.name.eq_ignore_ascii_case("ABS"))
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut NativeServiceRegistry::default(),
        RunBudget::default(),
    );
    let fault = take_fault(report);
    assert_eq!(fault.category, erabasic_vm::FaultCategory::HostContract);
    assert_eq!(fault.code, VmFaultCode::Native);
    assert!(fault.message.to_ascii_uppercase().contains("ABS"));
    assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(0));
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert_method_watch(&vm, &artifact, "RESULT", 0, VmValue::Integer(1));
    assert_method_watch(&vm, &artifact, "RESULT", 1, VmValue::Integer(0));
    assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(1));
}

#[test]
fn existvar_probe_node_budget_is_not_a_catchable_script_failure() {
    let expression = (0..24).fold("1".to_owned(), |inner, _| format!("ABS({inner})"));
    let source = format!(
        "@SYSTEM_TITLE\nRESULT = STRFORMCHECK(\"{{PROBE()}}\")\nFLAG:1 = 1\nRETURN\n@PROBE\n#FUNCTION\nFLAG:0 = 1\nRETURNF EXISTVAR(\"{expression}\", 1)\n"
    );
    let artifact = compile_source_with_options(&source, &method_options(true));
    let (vm, report) = run_entry(
        &artifact,
        VmConfig {
            maximum_operand_stack: 16,
            ..VmConfig::default()
        },
    );
    let fault = take_fault(report);
    assert_eq!(fault.category, erabasic_vm::FaultCategory::ResourceLimit);
    assert!(fault.message.contains("AST"), "{fault:?}");
    assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(1));
    assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(0));
}
