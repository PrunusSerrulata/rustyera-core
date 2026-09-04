use super::*;

#[test]
fn html_queries_lower_to_validated_lazy_host_steps() {
    let project = analyze(
        r#"@SYSTEM_TITLE
RESULT:10 = HTML_STRINGLEN("<b>x</b>", FLAG_VALUE())
RESULT:11 = HTML_STRINGLINES("abc", WIDTH_VALUE())
HTML_STRINGLEN "x"
HTML_STRINGLINES "abc", WIDTH_VALUE()
RETURN
@FLAG_VALUE
#FUNCTION
RETURNF 1
@WIDTH_VALUE
#FUNCTION
RETURNF 2
"#,
    );
    let report = compile_project(
        &project,
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    let artifact = report.artifact.expect("HTML query lowering should compile");
    let validation = validate_bytecode(
        artifact.clone().into_unvalidated(),
        &erabasic_compiler::runtime_native_validation_context(&artifact, &default_host_registry()),
    );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let names = artifact
        .host_imports
        .iter()
        .map(|host| host.import.name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for name in [
        "html__measure_length",
        "html__length_unit",
        "html__lines_begin",
        "html__lines_more",
        "html__lines_step",
        "html__lines_end",
    ] {
        assert!(names.contains(name), "missing {name}");
        assert!(
            default_host_registry().classification(name).is_none(),
            "private query steps cannot be called by scripts"
        );
    }
    assert!(!names.contains("html_stringlen") && !names.contains("html_stringlines"));
}

#[test]
fn dynamic_form_and_list_calls_share_lazy_slots_and_explicit_method_discard() {
    use erabasic_bytecode::{UserCallMode, UserCallSpec};
    let project = analyze(
        "@SYSTEM_TITLE\nCALLFORMF METHOD_TARGET(3)\nTRYCALLFORM MISSING(1 / LOCAL:1)\nTRYCALLLIST\nFUNC MISSING, 1 / LOCAL\nFUNC LIST_TARGET, 7\nENDFUNC\nRETURN\n@METHOD_TARGET(ARG)\n#FUNCTION\nRETURNF ARG + 1\n@LIST_TARGET(ARG)\nRETURN\n",
    );
    let report = compile_project(
        &project,
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    let artifact = report.artifact.expect("dynamic form/list calls compile");
    let code = &artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .code;
    assert!(
        code.iter()
            .all(|instruction| !matches!(instruction.opcode, 36 | 37))
    );
    let specs = code
        .iter()
        .filter(|instruction| instruction.opcode == Opcode::ResolveUserCall as u16)
        .map(|instruction| UserCallSpec::decode(&instruction.payload).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(specs.len(), 4);
    assert_eq!(specs[0].mode, UserCallMode::MethodDiscard);
    assert!(
        specs[1..]
            .iter()
            .all(|spec| spec.mode == UserCallMode::Procedure && spec.allow_missing)
    );
    for (resolve, instruction) in code
        .iter()
        .enumerate()
        .filter(|(_, instruction)| instruction.opcode == Opcode::ResolveUserCall as u16)
    {
        let spec = UserCallSpec::decode(&instruction.payload).unwrap();
        if spec.allow_missing {
            assert_eq!(
                code[spec.missing_target as usize],
                erabasic_bytecode::opcode::abandon_user_call(u32::try_from(resolve).unwrap())
            );
        }
        assert_eq!(code[resolve + 1].opcode, Opcode::GuardUserArgument as u16);
    }
    let validation = validate_bytecode(
        artifact.clone().into_unvalidated(),
        &erabasic_compiler::runtime_native_validation_context(&artifact, &default_host_registry()),
    );
    assert!(validation.is_valid(), "{:?}", validation.diagnostics);
}

#[test]
fn complete_call_text_six_modes_lower_one_string_with_local_catch_status() {
    use erabasic_bytecode::{CallTextMode, CallTextSpec};
    // Requires the separately owned CALLSTR parser/catalog/analyzer hunks before execution.
    let data = load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
        .data
        .unwrap();
    let analysis = analyze_project(AnalysisInput { project_data: data, sources: vec![ProjectSource {
        relative_path: "call-text.erb".into(), payload: SourcePayload::Utf8(
            "@SYSTEM_TITLE\nCALLSTR \"TARGET(1)\"\nJUMPSTR \" \"\nTRYCALLSTR \"MISSING, 2\"\nTRYJUMPSTR \" \"\nTRYCCALLSTR \"TARGET(3)\"\nCATCH\nRESULT = 4\nENDCATCH\nTRYCJUMPSTR \" \"\nCATCH\nRESULT = 5\nENDCATCH\nRETURN\n@TARGET(ARG)\nRETURN\n".into()),
    }] }, &AnalyzerOptions { compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake), ..AnalyzerOptions::analysis_mode() }, &ExtensionRegistry::default());
    let project = analysis
        .project
        .expect("snake call-text parses and analyzes");
    let report = compile_project(
        &project,
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    let artifact = report.artifact.expect("complete call-text lowers");
    let code = &artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .code;
    let specs = code
        .iter()
        .filter(|instruction| instruction.opcode == Opcode::InvokeCallText as u16)
        .map(|instruction| CallTextSpec::decode(&instruction.payload).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        specs.iter().map(|spec| spec.mode).collect::<Vec<_>>(),
        vec![
            CallTextMode::Call,
            CallTextMode::Jump,
            CallTextMode::TryCall,
            CallTextMode::TryJump,
            CallTextMode::CatchCall,
            CallTextMode::CatchJump
        ]
    );
    for spec in specs {
        if spec.mode.has_catch() {
            assert_eq!(
                code[spec.catch_target as usize],
                erabasic_bytecode::opcode::push_integer(0)
            );
        } else {
            assert_eq!(spec.catch_target, 0);
        }
    }
    let validation = validate_bytecode(
        artifact.clone().into_unvalidated(),
        &erabasic_compiler::runtime_native_validation_context(&artifact, &default_host_registry()),
    );
    assert!(validation.is_valid(), "{:?}", validation.diagnostics);
}

fn analyze_call_dependency_sources(
    caller: &str,
    target: &str,
) -> erabasic_analyzer::AnalyzedProject {
    let data = load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
        .data
        .unwrap();
    let report = analyze_project(
        AnalysisInput {
            project_data: data,
            sources: [
                ("caller.erb", caller),
                ("target.erb", target),
                ("unrelated.erb", "@UNRELATED\nRETURN\n"),
            ]
            .into_iter()
            .map(|(path, text)| ProjectSource {
                relative_path: path.into(),
                payload: SourcePayload::Utf8(text.into()),
            })
            .collect(),
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|value| value.reference_level >= 2),
        "{:?}",
        report.diagnostics
    );
    report.project.unwrap()
}

#[test]
fn changed_callee_defaults_and_formals_recompile_only_dependent_callers() {
    let registry = default_host_registry();
    let options = CompilerOptions::default();
    for (caller, before, after) in [
        (
            "@SYSTEM_TITLE\nCALL TARGET\nRETURN\n",
            "@TARGET(ARG:0 = 1)\nRETURN\n",
            "@TARGET(ARG:0 = 2)\nRETURN\n",
        ),
        (
            "@SYSTEM_TITLE\nCALL TARGET\nRETURN\n",
            "@TARGET(ARG:0 = 1)\nRETURN\n",
            "@TARGET(ARG:0 = 1, ARG:1 = 2)\nRETURN\n",
        ),
        (
            "@SYSTEM_TITLE\nRESULT = TARGET()\nRETURN\n",
            "@TARGET(ARG:0 = 1)\n#FUNCTION\nRETURNF ARG\n",
            "@TARGET(ARG:0 = 2)\n#FUNCTION\nRETURNF ARG\n",
        ),
    ] {
        let initial = compile_project_with_artifact(
            &analyze_call_dependency_sources(caller, before),
            &options,
            &registry,
            None,
            None,
        );
        let project = analyze_call_dependency_sources(caller, after);
        let warm = compile_project_with_artifact(
            &project,
            &options,
            &registry,
            Some(&initial.incremental_state),
            initial.artifact.as_ref(),
        );
        let cold = compile_project(&project, &options, &registry, None);
        assert!(warm.artifact.is_some(), "{:?}", warm.diagnostics);
        assert_eq!(warm.artifact, cold.artifact);
        assert_eq!(warm.stats.compiled_functions, 2, "{caller}");
        assert_eq!(warm.stats.reused_functions, 1, "{caller}");
    }
}

#[test]
fn dynamic_callers_depend_on_possible_signatures_but_not_callee_bodies() {
    let registry = default_host_registry();
    let options = CompilerOptions::default();
    let caller = "@SYSTEM_TITLE\nRESULT = GETMETH(LOCALS, 0)\nRETURN\n";
    let before = "@TARGET(ARG:0 = 1)\n#FUNCTION\nRETURNF 1\n";
    let initial = compile_project_with_artifact(
        &analyze_call_dependency_sources(caller, before),
        &options,
        &registry,
        None,
        None,
    );
    for (after, compiled) in [
        ("@TARGET(ARG:0 = 2)\n#FUNCTION\nRETURNF 1\n", 2),
        ("@TARGET(ARG:0 = 1)\n#FUNCTION\nRETURNF 2\n", 1),
    ] {
        let project = analyze_call_dependency_sources(caller, after);
        let warm = compile_project_with_artifact(
            &project,
            &options,
            &registry,
            Some(&initial.incremental_state),
            initial.artifact.as_ref(),
        );
        let cold = compile_project(&project, &options, &registry, None);
        assert!(warm.artifact.is_some(), "{:?}", warm.diagnostics);
        assert_eq!(warm.artifact, cold.artifact);
        assert_eq!(warm.stats.compiled_functions, compiled);
        assert_eq!(warm.stats.reused_functions, 3 - compiled);
    }
}

#[test]
fn call_compatibility_is_part_of_incremental_semantic_identity() {
    let registry = default_host_registry();
    let options = CompilerOptions::default();
    let original = analyze_call_dependency_sources(
        "@SYSTEM_TITLE\nCALL TARGET, 1\nRETURN\n",
        "@TARGET(ARG)\nRETURN\n",
    );
    let initial = compile_project(&original, &options, &registry, None);
    for policy in 0..3 {
        let mut project = original.clone();
        match policy {
            0 => {
                project.program.call_compatibility.allow_event_as_normal =
                    !project.program.call_compatibility.allow_event_as_normal;
            }
            1 => {
                project.program.call_compatibility.allow_omitted_arguments =
                    !project.program.call_compatibility.allow_omitted_arguments;
            }
            _ => {
                project
                    .program
                    .call_compatibility
                    .auto_convert_integer_to_string = !project
                    .program
                    .call_compatibility
                    .auto_convert_integer_to_string;
            }
        }
        let warm = compile_project(
            &project,
            &options,
            &registry,
            Some(&initial.incremental_state),
        );
        let cold = compile_project(&project, &options, &registry, None);
        assert_eq!(warm.artifact, cold.artifact);
        assert_eq!(warm.stats.reused_functions, 0);
    }
}

#[test]
fn changing_a_reference_formal_rebuilds_its_call_contract() {
    let registry = default_host_registry();
    let options = CompilerOptions::default();
    let caller = "@SYSTEM_TITLE\nCALL TARGET, FLAG\nRETURN\n";
    let original =
        analyze_call_dependency_sources(caller, "@TARGET(ITEMS)\n#DIM REF ITEMS, 0\nRETURN\n");
    let initial = compile_project_with_artifact(&original, &options, &registry, None, None);
    assert!(initial.artifact.is_some(), "{:?}", initial.diagnostics);
    let project =
        analyze_call_dependency_sources(caller, "@TARGET(ITEMS)\n#DIM ITEMS, 1\nRETURN\n");
    let warm = compile_project_with_artifact(
        &project,
        &options,
        &registry,
        Some(&initial.incremental_state),
        initial.artifact.as_ref(),
    );
    let cold = compile_project(&project, &options, &registry, None);
    assert!(warm.artifact.is_some(), "{:?}", warm.diagnostics);
    assert_eq!(warm.artifact, cold.artifact);
    // Existing shared variable dependencies are deliberately unchanged by this task, so the
    // declaration change can invalidate more than the caller and target.
    assert!(warm.stats.compiled_functions >= 2);
}

#[test]
fn dynamic_native_families_follow_registry_across_warm_compilation_and_validation() {
    let project = analyze("@SYSTEM_TITLE\nRESULTS '= STRFORM(\"{ABS(-7)}\")\nRETURN\n");
    let options = CompilerOptions::default();
    let registry = default_host_registry();
    let first = compile_project(&project, &options, &registry, None);
    let artifact = first.artifact.as_ref().unwrap();
    assert!(
        !artifact
            .native_imports
            .iter()
            .any(|native| native.import.name == "abs")
    );
    let family = artifact
        .runtime_native_authorizations
        .iter()
        .find(|family| family.name == "abs")
        .unwrap();
    let accepted =
        erabasic_compiler::runtime_native_validation_context(artifact, &default_host_registry());
    let mut denied_context = accepted.clone();
    denied_context
        .runtime_native_authorizations
        .remove(&family.key);
    let denied = validate_bytecode(artifact.clone().into_unvalidated(), &denied_context);
    assert!(denied.value.is_none());
    assert!(
        denied.diagnostics.iter().any(
            |diagnostic| diagnostic.code == erabasic_validator::ValidationCode::HostAbiMismatch
        )
    );

    let mut weakened = artifact.clone();
    let family = weakened
        .runtime_native_authorizations
        .iter_mut()
        .find(|family| family.name == "getvar")
        .unwrap();
    family.contract = erabasic_bytecode::canonical_native_contract("abs");
    family.key = family.canonical_key();
    weakened.refresh_ids().unwrap();
    let report = validate_bytecode(
        weakened.clone().into_unvalidated(),
        &erabasic_compiler::runtime_native_validation_context(&weakened, &default_host_registry()),
    );
    assert!(
        report.value.is_none(),
        "trusted registry and canonical source contracts must reject artifact state-policy weakening"
    );

    let mut denied_registry = registry;
    denied_registry.register_execution(
        "ABS",
        ExecutionBinding::Unsupported {
            reason: "host withdrew dynamic grant".into(),
        },
    );
    let second = compile_project(
        &project,
        &options,
        &denied_registry,
        Some(&first.incremental_state),
    );
    let target = second.artifact.as_ref().unwrap();
    assert!(
        !target
            .runtime_native_authorizations
            .iter()
            .any(|family| family.name == "abs")
    );
    assert!(
        target
            .runtime_builtins
            .iter()
            .any(|symbol| symbol.name == "ABS")
    );
    assert_ne!(
        artifact.manifest.program_version.execution_id,
        target.manifest.program_version.execution_id
    );
    assert_eq!(
        apply_patch(artifact, second.patch.as_ref().unwrap()).unwrap(),
        *target
    );
}

#[test]
fn dynamic_host_grants_follow_registry_in_warm_cache_patch_and_container() {
    let project = analyze("@SYSTEM_TITLE\nRESULTS '= STRFORM(\"{GETKEY(7)}\")\nRETURN\n");
    let options = CompilerOptions::default();
    let registry = default_host_registry();
    let first = compile_project(&project, &options, &registry, None);
    let artifact = first.artifact.as_ref().unwrap();
    assert!(
        !artifact
            .host_imports
            .iter()
            .any(|host| host.import.name.eq_ignore_ascii_case("getkey"))
    );
    let family = artifact
        .runtime_host_authorizations
        .iter()
        .find(|family| family.name == "getkey")
        .unwrap();
    let mut context = erabasic_compiler::runtime_native_validation_context(artifact, &registry);
    context.runtime_host_authorizations.remove(&family.key);
    let denied = validate_bytecode(artifact.clone().into_unvalidated(), &context);
    assert!(denied.value.is_none());
    assert!(
        denied.diagnostics.iter().any(
            |diagnostic| diagnostic.code == erabasic_validator::ValidationCode::HostAbiMismatch
        )
    );
    let decoded = decode_artifact(
        &encode_artifact(artifact).unwrap(),
        &DecodeLimits::default(),
    )
    .unwrap()
    .into_inner();
    assert_eq!(&decoded, artifact);
    let mut denied_registry = registry;
    denied_registry.register_execution(
        "GETKEY",
        ExecutionBinding::Unsupported {
            reason: "withdrawn runtime Host grant".into(),
        },
    );
    let warm = compile_project(
        &project,
        &options,
        &denied_registry,
        Some(&first.incremental_state),
    );
    let target = warm.artifact.as_ref().unwrap();
    let cold = compile_project(&project, &options, &denied_registry, None);
    assert_eq!(cold.artifact.as_ref(), Some(target));
    assert!(
        !target
            .runtime_host_authorizations
            .iter()
            .any(|family| family.name == "getkey")
    );
    assert_ne!(
        artifact.manifest.program_version.execution_id,
        target.manifest.program_version.execution_id
    );
    assert_eq!(
        &apply_patch(artifact, warm.patch.as_ref().unwrap()).unwrap(),
        target
    );
}
