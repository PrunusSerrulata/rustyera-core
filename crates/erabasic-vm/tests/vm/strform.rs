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
            ("METHOD_TRACE", 1),
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
            Opcode::ResolveMethod,
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
                let pending = &mut frame["method_calls"][0];
                match corruption {
                    "origin" => pending["resolve"] = serde_json::json!(usize::MAX),
                    "generation" => pending["method"]["generation"] = serde_json::json!(999),
                    "slot" => pending["captured"] = serde_json::json!(999),
                    "target" => {
                        pending["method"]["function"] = serde_json::to_value(entry).unwrap();
                    }
                    "capture" => {
                        let slot =
                            usize::try_from(pending["stack_index"].as_u64().unwrap()).unwrap() + 1;
                        frame["stack"][slot] =
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
                    "origin" => call["method"]["function"] = serde_json::to_value(entry).unwrap(),
                    "generation" => call["method"]["generation"] = serde_json::json!(999),
                    "slot" => call["next_slot"] = serde_json::json!(999),
                    "target" => call["method"]["bindings"] = serde_json::json!([]),
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
                opcode::push_string("DISCARDED_METHOD"),
                opcode::resolve_method(&erabasic_bytecode::MethodCallSpec {
                    result: erabasic_bytecode::MethodResult::Integer,
                    allow_missing: false,
                    missing_target: 0,
                    arguments: Vec::new(),
                }),
                erabasic_bytecode::EncodedInstruction::new(Opcode::Pop, Vec::new()),
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
        json["fibers"][fiber.0.to_string()]["frames"][0]["method_calls"]
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
fn unsupported_runtime_form_is_rejected_before_any_user_side_effect() {
    let artifact = compile_source(
        r#"@SYSTEM_TITLE
RESULT = 0
RESULTS:0 '= STRFORM("{BUMP()}|{RESULT++}")
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
    assert_eq!(fault.code, VmFaultCode::Native);
    assert!(
        fault
            .message
            .contains("increment expressions are unsupported")
    );
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
