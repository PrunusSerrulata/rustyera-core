use super::*;

#[test]
fn column_options_lower_to_validated_private_native_steps() {
    let project = analyze(
        "@SYSTEM_TITLE\nDT_COLUMN_OPTIONS \"t\", \"c\", DEFAULT, 12, DEFAULT, \"text\"\nRETURN\n",
    );
    let report = compile_project(
        &project,
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    let artifact = report.artifact.expect("column options compile");
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
        .native_imports
        .iter()
        .map(|native| native.import.name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        names,
        [
            "dt__column_resolve",
            "dt__column_check_int",
            "dt__column_apply_int",
            "dt__column_check_str",
            "dt__column_apply_str"
        ]
        .into_iter()
        .collect()
    );
    assert!(
        artifact
            .native_imports
            .iter()
            .all(|native| native.contract.debug
                == erabasic_bytecode::OperationDebugPolicy::Forbidden)
    );
    assert!(artifact.host_imports.is_empty());
}

#[test]
fn owned_compile_moves_source_indices_into_the_validated_artifact() {
    let project = analyze("@SYSTEM_TITLE\nPRINTL one\nPRINTL two\nRETURN\n");
    let line_starts = project.program.sources[0].line_starts.as_ptr();

    let report = compile_owned_validated_project_with_artifact(
        project,
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
        None,
    );

    let artifact = report
        .report
        .artifact
        .expect("owned project should compile");
    assert_eq!(report.source_ids, [erabasic_hir::SourceId(0)]);
    assert_eq!(
        artifact.artifact().source_map.sources[0]
            .line_starts
            .as_ptr(),
        line_starts
    );
}

#[test]
fn borrowed_and_owned_validated_compiles_are_observationally_identical() {
    let text = "@SYSTEM_TITLE\nPRINTL one\nPRINTL two\nRETURN\n";
    let borrowed_project = analyze(text);
    let borrowed = compile_validated_project_with_artifact(
        &borrowed_project,
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
        None,
    );
    let owned = compile_owned_validated_project_with_artifact(
        analyze(text),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
        None,
    );

    assert_eq!(owned.report, borrowed);
    assert_eq!(owned.source_ids, [erabasic_hir::SourceId(0)]);
    assert!(owned.diagnostic_sources.is_empty());
}

#[test]
fn frontend_observation_calls_emit_nonfatal_source_notices() {
    let report = compile_project(
        &analyze("@SYSTEM_TITLE\nRESULT = CLIENTWIDTH()\nRETURN\n"),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    assert!(report.artifact.is_some());
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == CompilerDiagnosticCode::FrontendObservation
            && diagnostic.severity == CompilerDiagnosticSeverity::Notice
            && diagnostic.location.is_some()
    }));
}

#[test]
fn snake_audio_imports_persist_exact_wait_and_fallback_contracts() {
    let report = compile_project(
        &analyze_snake(
            "@SYSTEM_TITLE\nPLAYSOUND \"tone.wav\", 2\nRESULT:0 = GETSOUNDORBGMINFO(0, 1)\nRESULT:1 = ISPLAYINGSOUND(0)\nRESULT:2 = ISPLAYINGBGM()\nRESULT:3 = SOUNDCONTROL(0, 0)\nRESULT:4 = BGMCONTROL(0)\nRETURN\n",
        ),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    let artifact = report.artifact.expect("snake audio fixture compiles");
    for host in artifact
        .host_imports
        .iter()
        .filter(|host| host.import.namespace == "rustyera.audio")
    {
        let name = host.import.name.as_str();
        let query = matches!(
            name,
            "getsoundorbgminfo" | "isplayingsound" | "isplayingbgm"
        );
        let selecting_play = name == "playsound";
        if query || selecting_play {
            assert_eq!(
                host.contract.state,
                erabasic_bytecode::OperationState::External
            );
            assert_eq!(
                host.contract.transaction,
                erabasic_bytecode::TransactionPolicy::Forbidden
            );
            assert_eq!(
                host.contract.persistence,
                erabasic_bytecode::OperationPersistence::RuntimeOnly
            );
            assert_eq!(
                host.contract.snapshot,
                erabasic_bytecode::OperationSnapshotPolicy::PendingBlocks
            );
            assert_eq!(
                host.contract.hot_reload,
                erabasic_bytecode::OperationHotReloadPolicy::ActiveBlocks
            );
            assert_eq!(
                host.contract.wait,
                erabasic_bytecode::OperationWaitPolicy::TransientExternal
            );
            assert_eq!(
                host.contract.capability_fallback,
                if query {
                    erabasic_bytecode::CapabilityFallback::Unsupported
                } else {
                    erabasic_bytecode::CapabilityFallback::IntentNoOp
                }
            );
            assert_eq!(
                host.contract.portability,
                if query {
                    erabasic_bytecode::OperationPortability::FrontendObservation
                } else {
                    erabasic_bytecode::OperationPortability::PlatformIntent
                }
            );
        } else {
            assert_eq!(
                host.contract.transaction,
                erabasic_bytecode::TransactionPolicy::BufferedEffect
            );
            assert_eq!(
                host.contract.wait,
                erabasic_bytecode::OperationWaitPolicy::Immediate
            );
        }
        assert!(host.contract.is_coherent(), "{name}");
    }
    let bytes = encode_artifact(&artifact).unwrap();
    let decoded = decode_artifact(&bytes, &DecodeLimits::default()).unwrap();
    assert_eq!(decoded.into_inner().host_imports, artifact.host_imports);
}

#[test]
fn scoped_scalar_initializers_and_array_declarations_compile() {
    let report = compile_project(
        &analyze(
            "@SYSTEM_TITLE(ARG:0 = 2)\nVARI VALUE = ARG:0 + 1\nVARS TEXT = \"ok\"\nVARI ITEMS, 3\nITEMS:2 = VALUE\nRETURN\n",
        ),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );

    assert!(report.artifact.is_some(), "{:#?}", report.diagnostics);
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == CompilerDiagnosticSeverity::Error),
        "{:#?}",
        report.diagnostics
    );
    let artifact = report.artifact.expect("scoped declarations should compile");
    let function = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("entry function");
    assert!(
        function
            .code
            .iter()
            .any(|instruction| instruction.opcode == Opcode::Nop as u16),
        "the scoped array declaration should retain a source-mapped NOP"
    );
}

#[test]
fn folded_getnum_does_not_emit_a_native_call() {
    let artifact = compile_project(
        &analyze("@SYSTEM_TITLE\nRESULT = GETNUM(CFLAG, \"missing\")\nRETURN RESULT\n"),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    )
    .artifact
    .expect("constant GETNUM should compile");

    assert!(
        artifact
            .native_imports
            .iter()
            .all(|import| !import.import.name.eq_ignore_ascii_case("getnum"))
    );
}

#[test]
fn loop_control_outside_a_loop_compiles_to_a_runtime_trap() {
    let artifact = compile_project(
        &analyze("@SYSTEM_TITLE\nCONTINUE\nRETURN\n"),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    )
    .artifact
    .expect("deferred loop-control fault should compile");
    assert!(
        artifact.functions[0]
            .code
            .iter()
            .any(|instruction| instruction.opcode == Opcode::Trap as u16
                && instruction.payload.as_slice() == b"CONTINUE outside loop")
    );
}

#[test]
fn expression_methods_use_typed_lazy_bytecode_in_expressions_and_statements() {
    use erabasic_bytecode::{UserArgumentSpec, UserCallMode, UserCallSpec};

    let project = analyze(
        "@SYSTEM_TITLE\nRESULT = GETMETH(\"TARGET\", 7, FLAG:1,, -9223372036854775807 - 1)\nGETMETHS \"TEXT\", \"fallback\", STR:2\nRETURN\n",
    );
    let report = compile_project(
        &project,
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let artifact = report.artifact.expect("typed method calls should compile");
    assert!(artifact.native_imports.iter().all(|import| !matches!(
        import.import.name.to_ascii_uppercase().as_str(),
        "GETMETH" | "GETMETHS"
    )));
    let code = &artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .code;
    let specs = code
        .iter()
        .filter(|instruction| instruction.opcode == Opcode::ResolveUserCall as u16)
        .map(|instruction| UserCallSpec::decode(&instruction.payload).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(specs.len(), 2);
    assert_eq!(specs[0].mode, UserCallMode::MethodInteger);
    assert!(matches!(
        specs[0].arguments.as_slice(),
        [
            UserArgumentSpec::Variable(_),
            UserArgumentSpec::Omitted,
            UserArgumentSpec::Value(BytecodeType::Integer)
        ]
    ));
    assert_eq!(specs[1].mode, UserCallMode::MethodString);
    for opcode in [
        Opcode::GuardUserArgument,
        Opcode::AdvanceUserArgument,
        Opcode::AbandonUserCall,
        Opcode::SelectUserArgument,
        Opcode::CaptureUserArgument,
        Opcode::InvokeUserCall,
    ] {
        assert!(
            code.iter()
                .any(|instruction| instruction.opcode == opcode as u16)
        );
    }
    let encoded = encode_artifact(&artifact).unwrap();
    let decoded = decode_artifact(&encoded, &DecodeLimits::default()).unwrap();
    let validation = validate_bytecode(
        decoded,
        &erabasic_compiler::runtime_native_validation_context(&artifact, &default_host_registry()),
    );
    assert!(validation.is_valid(), "{:?}", validation.diagnostics);
}

#[test]
fn continuation_replacement_keeps_compiler_source_maps_in_bounds() {
    let data = load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
        .data
        .expect("default project data should load");
    let source = "@SYSTEM_TITLE\n{\nRESULT += 1\n    + 2\n    + 3\n}\n";
    let mut options = AnalyzerOptions::analysis_mode();
    options.continuation_separator = " \t".into();
    let analysis = analyze_project(
        AnalysisInput {
            project_data: data,
            sources: vec![ProjectSource {
                relative_path: "main.erb".into(),
                payload: SourcePayload::Utf8(source.into()),
            }],
        },
        &options,
        &ExtensionRegistry::default(),
    );
    assert!(
        !analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.reference_level >= 2),
        "{:#?}",
        analysis.diagnostics
    );
    let report = compile_project(
        &analysis.project.expect("analysis should produce a project"),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    assert!(report.artifact.is_some(), "{:#?}", report.diagnostics);
    assert!(
        !report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == CompilerDiagnosticCode::Validation
                && diagnostic.message.contains("source-map entry is outside")
        }),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn gdrawsprite_preserves_the_color_matrix_array_place() {
    let artifact = compile_project(
        &analyze(
            "@SYSTEM_TITLE\n#DIM MATRIX, 5, 5\nGDRAWSPRITE 1, \"sprite\", 0, 0, 1, 1, MATRIX:0:0\nRETURN\n",
        ),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    )
    .artifact
    .expect("GDRAWSPRITE with a color matrix should compile");
    let import = artifact
        .host_imports
        .iter()
        .find(|import| import.import.name == "gdrawsprite")
        .expect("GDRAWSPRITE host import");
    assert_eq!(
        import.import.parameters,
        [
            BytecodeType::Integer,
            BytecodeType::String,
            BytecodeType::Integer,
            BytecodeType::Integer,
            BytecodeType::Integer,
            BytecodeType::Integer,
            BytecodeType::IntegerPlace,
        ]
    );
}

#[test]
fn static_user_argument_policy_preserves_lazy_lowering_and_warm_analysis_warnings() {
    let text = "@SYSTEM_TITLE\nCALL TAKE, 1, SIDE(), 1 / 0\nRESULT = METH(2, SIDE(), 9223372036854775807 + 1)\nRETURN\n@TAKE(ARG)\nRETURN\n@METH(ARG)\n#FUNCTION\nRETURNF ARG\n@SIDE\n#FUNCTION\nFLAG:0 += 1\nRETURNF 9\n";
    let analyze_snake = || {
        analyze_project(
            AnalysisInput {
                project_data: load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
                    .data
                    .unwrap(),
                sources: vec![ProjectSource {
                    relative_path: "user-arity.erb".into(),
                    payload: SourcePayload::Utf8(text.into()),
                }],
            },
            &AnalyzerOptions {
                compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
                    erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
                ),
                ..AnalyzerOptions::analysis_mode()
            },
            &ExtensionRegistry::default(),
        )
    };
    let first_analysis = analyze_snake();
    let first_diagnostics = first_analysis.diagnostics.clone();
    assert_eq!(
        first_diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == erabasic_analyzer::AnalyzerDiagnosticCode::ExcessUserArguments
            })
            .count(),
        2
    );
    let first = compile_project(
        &first_analysis.project.unwrap(),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    assert!(first.diagnostics.is_empty(), "{:?}", first.diagnostics);
    let artifact = first
        .artifact
        .as_ref()
        .expect("snake extra arguments should compile");
    let title = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap();
    let side = artifact
        .functions
        .iter()
        .find(|function| function.name == "SIDE")
        .unwrap();
    assert!(title.imports.iter().all(|import| import.key != side.key));
    assert_eq!(
        title
            .code
            .iter()
            .filter(|instruction| { instruction.opcode == Opcode::Call as u16 })
            .count(),
        2,
        "only the two retained user calls may be emitted"
    );
    let warm_analysis = analyze_snake();
    assert_eq!(
        warm_analysis.diagnostics, first_diagnostics,
        "load warnings must not depend on whether function bytecode will be reused"
    );
    let mut warm_project = warm_analysis.project.unwrap();
    let warm = compile_project(
        &warm_project,
        &CompilerOptions::default(),
        &default_host_registry(),
        Some(&first.incremental_state),
    );
    assert!(warm.artifact.is_some(), "{:?}", warm.diagnostics);
    assert_eq!(warm.stats.compiled_functions, 0);
    assert_eq!(warm.stats.reused_functions, artifact.functions.len());
    // A policy change must not hit a warm function compiled under the warning mode.
    warm_project.program.call_compatibility.user_argument_policy =
        erabasic_compat::UserCallArgumentPolicy::RejectExcess;
    let strict = compile_project(
        &warm_project,
        &CompilerOptions::default(),
        &default_host_registry(),
        Some(&warm.incremental_state),
    );
    assert!(strict.artifact.is_none());
    assert!(strict.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == CompilerDiagnosticCode::InvalidHir
            && diagnostic.severity == CompilerDiagnosticSeverity::Error
    }));
}

#[test]
fn reference_static_calls_and_direct_methods_keep_excess_argument_errors() {
    for statement in ["CALL TAKE, 1, 2", "RESULT = METH(1, 2)"] {
        let report = compile_project(
            &analyze(&format!(
                "@SYSTEM_TITLE\n{statement}\nRETURN\n@TAKE(ARG)\nRETURN\n@METH(ARG)\n#FUNCTION\nRETURNF ARG\n"
            )),
            &CompilerOptions::default(),
            &default_host_registry(),
            None,
        );
        assert!(report.artifact.is_none());
        assert!(report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == CompilerDiagnosticCode::InvalidHir
                && diagnostic.severity == CompilerDiagnosticSeverity::Error
        }));
    }
}

#[test]
fn static_call_ignores_a_final_argument_separator() {
    let report = compile_project(
        &analyze("@SYSTEM_TITLE\nCALL TARGET,\nRETURN\n@TARGET\nRETURN\n"),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    assert!(report.artifact.is_some(), "{:#?}", report.diagnostics);
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.severity == CompilerDiagnosticSeverity::Error }),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn static_call_target_stops_before_an_inline_comment() {
    let report = compile_project(
        &analyze(
            "@SYSTEM_TITLE\nCALL TARGET; inline comment\nCALL TARGET, ; comment after separator\nRETURN\n@TARGET\nRETURN\n",
        ),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    assert!(report.artifact.is_some(), "{:#?}", report.diagnostics);
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == CompilerDiagnosticCode::MissingImport),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn host_operations_use_only_call_host_and_round_trip() {
    let project =
        analyze("@SYSTEM_TITLE\nPRINTFORM value={1 + 2}\nINPUT 0\nRESULT = 4 + 5\nRETURN\n");
    let report = compile_project(
        &project,
        &CompilerOptions {
            jobs: Some(2),
            ..CompilerOptions::default()
        },
        &default_host_registry(),
        None,
    );
    assert!(report.diagnostics.is_empty(), "{:#?}", report.diagnostics);
    let artifact = report.artifact.expect("compilation should succeed");
    assert_eq!(artifact.host_imports.len(), 2);
    let host_call_count = artifact
        .functions
        .iter()
        .flat_map(|function| &function.code)
        .filter(|instruction| instruction.opcode == Opcode::CallHost as u16)
        .count();
    assert_eq!(host_call_count, 2);
    assert!(artifact.host_imports.iter().all(|import| {
        matches!(
            import.import.namespace.as_str(),
            "rustyera.text" | "rustyera.input"
        )
    }));

    let bytes = encode_artifact(&artifact).expect("artifact should encode");
    let decoded =
        decode_artifact(&bytes, &DecodeLimits::default()).expect("artifact should decode");
    let validation = validate_bytecode(
        decoded,
        &erabasic_compiler::runtime_native_validation_context(&artifact, &default_host_registry()),
    );
    assert!(validation.is_valid(), "{:#?}", validation.diagnostics);
    assert_eq!(validation.value.unwrap().into_inner(), artifact);
}
