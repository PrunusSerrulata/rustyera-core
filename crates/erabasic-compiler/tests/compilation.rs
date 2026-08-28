use erabasic_analyzer::{
    AnalysisInput, AnalyzerOptions, ExtensionRegistry, ProjectSource, SourcePayload,
    analyze_project, builtin_function_names, builtin_instruction_names,
};
use erabasic_bytecode::{
    BytecodeType, DecodeLimits, HostCapability, HostEffect, HostSnapshotCapability, Opcode,
    apply_patch, decode_artifact, encode_artifact,
};
use erabasic_compiler::{
    CompilerDiagnosticCode, CompilerDiagnosticSeverity, CompilerOptions, ExecutionBinding,
    HostBinding, compile_owned_validated_project_with_artifact, compile_project,
    compile_project_with_artifact, compile_validated_project_with_artifact, default_host_registry,
};
use erabasic_csv::{CsvLoadOptions, ProjectFiles, load_project};
use erabasic_validator::{ValidationContext, validate_bytecode};

fn analyze(text: &str) -> erabasic_analyzer::AnalyzedProject {
    let data = load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
        .data
        .expect("default project data should load");
    let report = analyze_project(
        AnalysisInput {
            project_data: data,
            sources: vec![ProjectSource {
                relative_path: "main.erb".into(),
                payload: SourcePayload::Utf8(text.into()),
            }],
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
    report.project.expect("analysis should produce a project")
}

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
        &ValidationContext::for_artifact(&artifact),
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
fn expression_methods_use_typed_lazy_bytecode_in_expressions_and_statements() {
    use erabasic_bytecode::{MethodArgumentSpec, MethodCallSpec, MethodResult};

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
        .filter(|instruction| instruction.opcode == Opcode::ResolveMethod as u16)
        .map(|instruction| MethodCallSpec::decode(&instruction.payload).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(specs.len(), 2);
    assert_eq!(specs[0].result, MethodResult::Integer);
    assert!(matches!(
        specs[0].arguments.as_slice(),
        [
            MethodArgumentSpec::Variable(_),
            MethodArgumentSpec::Omitted,
            MethodArgumentSpec::Value(BytecodeType::Integer)
        ]
    ));
    assert_eq!(specs[1].result, MethodResult::String);
    for opcode in [
        Opcode::SelectMethodArgument,
        Opcode::CaptureMethodArgument,
        Opcode::InvokeMethod,
    ] {
        assert!(
            code.iter()
                .any(|instruction| instruction.opcode == opcode as u16)
        );
    }
    let encoded = encode_artifact(&artifact).unwrap();
    let decoded = decode_artifact(&encoded, &DecodeLimits::default()).unwrap();
    let validation = validate_bytecode(decoded, &ValidationContext::for_artifact(&artifact));
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
    let validation = validate_bytecode(decoded, &ValidationContext::for_artifact(&artifact));
    assert!(validation.is_valid(), "{:#?}", validation.diagnostics);
    assert_eq!(validation.value.unwrap().into_inner(), artifact);
}

#[test]
fn printdata_lowers_to_lazy_skip_random_selection_and_valid_stack_control() {
    let project = analyze(
        "@SYSTEM_TITLE\nPRINTDATAKW\nDATAFORM first={1}\nDATALIST\nDATAFORM second={2}\nDATAFORM third={3}\nENDLIST\nENDDATA\nRETURN\n",
    );
    let report = compile_project(
        &project,
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    assert!(report.diagnostics.is_empty(), "{:#?}", report.diagnostics);
    let artifact = report.artifact.expect("PRINTDATA should compile");
    let code = &artifact.functions[0].code;
    assert!(
        code.iter()
            .any(|instruction| instruction.opcode == Opcode::Dup as u16)
    );
    assert!(
        code.iter()
            .any(|instruction| instruction.opcode == Opcode::Pop as u16)
    );
    let validation = validate_bytecode(
        artifact.clone().into_unvalidated(),
        &ValidationContext::for_artifact(&artifact),
    );
    assert!(validation.is_valid(), "{:#?}", validation.diagnostics);
}

#[test]
fn dynamic_call_uses_vm_resolution_before_argument_evaluation() {
    let project = analyze(
        "@SYSTEM_TITLE\nLOCAL = 1\nTRYCALLFORM MISSING(1 / LOCAL:1)\nCALLFORM TARGET_{LOCAL}(4)\nRETURN\n@TARGET_1(ARG)\nRESULT = ARG\nRETURN\n",
    );
    let report = compile_project(
        &project,
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    assert!(report.diagnostics.is_empty(), "{:#?}", report.diagnostics);
    let artifact = report.artifact.expect("dynamic calls should compile");
    let code = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("entry function")
        .code
        .as_slice();
    assert!(
        code.iter()
            .any(|instruction| instruction.opcode == Opcode::ResolveFunction as u16)
    );
    assert!(
        code.iter()
            .any(|instruction| instruction.opcode == Opcode::InvokeDynamic as u16)
    );
}

#[test]
fn width_free_form_interpolation_stays_inside_bytecode() {
    let project = analyze(
        "@SYSTEM_TITLE\n#DIMS TEXT\nLOCAL = 7\nTEXT '= \"ok\"\nCALLFORM TARGET_{LOCAL}\nRESULTS = @\"value={LOCAL} %TEXT%\"\nRETURN\n@TARGET_7\nRETURN\n",
    );
    let report = compile_project(
        &project,
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    assert!(report.diagnostics.is_empty(), "{:#?}", report.diagnostics);
    let artifact = report.artifact.expect("width-free forms should compile");
    assert!(
        artifact
            .functions
            .iter()
            .flat_map(|function| &function.code)
            .any(|instruction| instruction.opcode == Opcode::ToString as u16)
    );
    assert!(artifact.native_imports.iter().all(|native| !matches!(
        native.import.name.as_str(),
        "format_integer" | "format_string"
    )));

    let project = analyze("@SYSTEM_TITLE\nLOCAL = 7\nRESULTS = @\"{LOCAL, 3}\"\nRETURN\n");
    let report = compile_project(
        &project,
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    assert!(report.diagnostics.is_empty(), "{:#?}", report.diagnostics);
    let artifact = report.artifact.expect("width-bearing forms should compile");
    assert!(
        artifact
            .native_imports
            .iter()
            .any(|native| native.import.name == "format_integer")
    );
}

#[test]
fn input_wait_and_getkey_bindings_preserve_dynamic_stability() {
    let registry = default_host_registry();
    for name in ["TINPUT", "TONEINPUTS", "TWAIT", "FORCEWAIT"] {
        let binding = registry.resolve(name).expect("input binding should exist");
        assert_eq!(binding.namespace, "rustyera.input");
        assert_eq!(binding.capability, HostCapability::Input);
        assert!(binding.effect.may_suspend);
        assert_eq!(
            binding.snapshot_capability,
            HostSnapshotCapability::StableWait
        );
    }
    let getkey = registry.resolve("GETKEY").expect("GETKEY binding");
    assert_eq!(getkey.namespace, "rustyera.input");
    assert_eq!(getkey.capability, HostCapability::Input);
    assert!(getkey.effect.may_suspend);
    // GETKEY waits for a frontend-owned primitive-input sample, but the value
    // is returned to the VM and does not mutate canonical runtime state.
    assert!(!getkey.effect.mutates_runtime);
    assert_eq!(getkey.snapshot_capability, HostSnapshotCapability::Never);

    let artifact = compile_project(
        &analyze(
            "@SYSTEM_TITLE\nTINPUT 100, 0\nTONEINPUTS 100, \"N\"\nTWAIT 100, 0\nFORCEWAIT\nRESULT = GETKEY(65)\nRETURN\n",
        ),
        &CompilerOptions::default(),
        &registry,
        None,
    )
    .artifact
    .expect("input fixture should compile");
    let import = |name: &str| {
        artifact
            .host_imports
            .iter()
            .find(|import| import.import.name == name)
            .unwrap_or_else(|| panic!("missing host import {name}"))
    };
    assert_eq!(
        import("tinput").import.parameters,
        [BytecodeType::Integer, BytecodeType::Integer]
    );
    assert_eq!(
        import("toneinputs").import.parameters,
        [BytecodeType::Integer, BytecodeType::String]
    );
    assert_eq!(
        import("twait").import.parameters,
        [BytecodeType::Integer, BytecodeType::Integer]
    );
    assert!(import("forcewait").import.parameters.is_empty());
    assert_eq!(import("getkey").import.parameters, [BytecodeType::Integer]);
    assert_eq!(import("getkey").import.result, Some(BytecodeType::Integer));
}

#[test]
fn runtime_owned_save_menus_are_stable_input_operations() {
    let registry = default_host_registry();
    for name in ["SAVEGAME", "LOADGAME"] {
        let binding = registry.resolve(name).expect("system menu binding");
        assert_eq!(binding.namespace, "rustyera.system");
        assert_eq!(binding.capability, HostCapability::System);
        assert_eq!(
            binding.snapshot_capability,
            HostSnapshotCapability::StableWait
        );
        assert_eq!(
            binding.contract.wait,
            erabasic_bytecode::OperationWaitPolicy::StableInput
        );
    }
}

#[test]
fn skip_scope_commands_cross_the_host_boundary() {
    let project = analyze("@SYSTEM_TITLE\nNOSKIP\nENDNOSKIP\nRETURN\n");
    let report = compile_project(
        &project,
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    assert!(report.diagnostics.is_empty(), "{:#?}", report.diagnostics);
    let artifact = report.artifact.expect("skip scope should compile");
    let names = artifact
        .host_imports
        .iter()
        .map(|import| import.import.name.as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"noskip"));
    assert!(names.contains(&"endnoskip"));
    assert!(
        artifact
            .functions
            .iter()
            .flat_map(|function| &function.code)
            .all(|instruction| {
                instruction.opcode != Opcode::CallNative as u16
                    || !matches!(instruction.payload.as_slice(), b"NOSKIP" | b"ENDNOSKIP")
            })
    );
}

#[test]
fn mutable_arguments_are_lowered_as_places_in_hir_v3() {
    let project = analyze("@SYSTEM_TITLE\nSWAP FLAG:0, FLAG:1\nRETURN\n");
    let arguments = project.program.functions[0]
        .lines
        .iter()
        .find_map(|line| match &line.kind {
            erabasic_hir::HirStatementKind::Instruction { arguments, .. }
                if !arguments.is_empty() =>
            {
                Some(arguments)
            }
            _ => None,
        })
        .expect("SWAP arguments");
    assert!(
        arguments
            .iter()
            .all(|argument| matches!(argument, erabasic_hir::HirArgument::Place(_)))
    );

    let artifact = compile_project(
        &project,
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    )
    .artifact
    .expect("place fixture should compile");
    assert!(
        artifact.functions[0]
            .code
            .iter()
            .any(|instruction| instruction.opcode == Opcode::MakePlace as u16)
    );
}

#[test]
fn event_dispatch_metadata_and_persistent_locals_survive_container_round_trip() {
    let artifact = compile_project(
        &analyze(
            "@EVENTFIRST\n#PRI\nLOCAL:0 = 1\nRETURN\n@EVENTFIRST\n#LATER\n#SINGLE\nLOCAL:0 += 1\nRETURN\n",
        ),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    )
    .artifact
    .expect("event fixture should compile");
    let group = artifact.event_groups.first().expect("EVENTFIRST group");
    assert_eq!(group.name, "EVENTFIRST");
    assert_eq!(group.priority.len(), 1);
    assert!(group.normal.is_empty());
    assert_eq!(group.later.len(), 1);
    assert!(group.later[0].single);
    assert!(artifact.globals.iter().any(|global| {
        global.name.eq_ignore_ascii_case("LOCAL")
            && global.storage == erabasic_bytecode::BytecodeStorage::FunctionPersistent
    }));

    let bytes = encode_artifact(&artifact).expect("artifact encoding");
    let decoded = decode_artifact(&bytes, &DecodeLimits::default()).expect("artifact decoding");
    assert_eq!(decoded.into_inner().event_groups, artifact.event_groups);
}

#[test]
fn every_analyzer_builtin_has_one_explicit_execution_class() {
    let registry = default_host_registry();
    for name in builtin_instruction_names()
        .into_iter()
        .chain(builtin_function_names())
    {
        let classification = registry
            .classification(&name)
            .unwrap_or_else(|| panic!("missing execution catalog entry for {name}"));
        match classification {
            ExecutionBinding::Host(binding) => {
                assert!(
                    binding.contract.is_coherent(),
                    "incoherent Host contract for {name}"
                );
                assert_eq!(
                    binding.effect,
                    binding.contract.effect(),
                    "stale Host effect for {name}"
                );
                assert_eq!(
                    binding.snapshot_capability,
                    binding.contract.snapshot_capability(),
                    "stale Host snapshot classification for {name}"
                );
            }
            ExecutionBinding::Native(contract) => {
                assert!(
                    contract.is_coherent(),
                    "incoherent Native contract for {name}"
                );
            }
            ExecutionBinding::ExpressionMethod { .. } | ExecutionBinding::Unsupported { .. } => {}
        }
    }
    assert!(matches!(
        registry.classification("GETTEXTBOX"),
        Some(ExecutionBinding::Host(binding)) if binding.namespace == "rustyera.input"
    ));
    for (name, namespace) in [
        ("PRINTFORMK", "rustyera.text"),
        ("BEGIN", "rustyera.system"),
        ("SAVETEXT", "rustyera.storage"),
        ("SPRITECREATE", "rustyera.graphics"),
        ("GETSECOND", "rustyera.clock"),
    ] {
        assert!(matches!(
            registry.classification(name),
            Some(ExecutionBinding::Host(binding)) if binding.namespace == namespace
        ));
    }
    assert!(matches!(
        registry.classification("RAND"),
        Some(ExecutionBinding::Native(_))
    ));
    assert!(matches!(
        registry.classification("CONVERT"),
        Some(ExecutionBinding::Native(_))
    ));
    assert!(matches!(
        registry.classification("VARSIZE"),
        Some(ExecutionBinding::Host(binding)) if binding.namespace == "rustyera.system"
    ));
    assert!(matches!(
        registry.classification("GETMETH"),
        Some(ExecutionBinding::ExpressionMethod {
            result: erabasic_bytecode::MethodResult::Integer
        })
    ));
    assert!(matches!(
        registry.classification("CALLSHARP"),
        Some(ExecutionBinding::Host(binding))
            if binding.namespace == "rustyera.extension" && binding.name == "callsharp"
    ));
}

#[test]
fn callsharp_compiles_to_the_versioned_extension_host_abi() {
    let report = compile_project(
        &analyze("@SYSTEM_TITLE\nCALLSHARP LAUNCH_BROWSER(\"https://example.invalid\")\nRETURN\n"),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );

    let artifact = report.artifact.expect("CALLSHARP should compile");
    assert!(artifact.host_imports.iter().any(|import| {
        import.import.namespace == "rustyera.extension"
            && import.import.name == "callsharp"
            && import.import.abi_version == 1
            && import.import.parameters == [erabasic_bytecode::BytecodeType::String]
    }));
}

#[test]
fn ignored_shadow_function_does_not_emit_unbound_reference_storage() {
    let data = load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
        .data
        .expect("default project data should load");
    let analysis = analyze_project(
        AnalysisInput {
            project_data: data,
            sources: vec![ProjectSource {
                relative_path: "main.erb".into(),
                payload: SourcePayload::Utf8(
                    "@SYSTEM_TITLE\n#DIM VALUES, 2\nCALL DUP(VALUES)\nRETURN\n@DUP(VALUES)\n#DIM REF VALUES, 0\nRETURN\n@DUP(VALUES)\n#DIM REF VALUES, 0\nRETURN\n"
                        .into(),
                ),
            }],
        },
        &AnalyzerOptions::default(),
        &ExtensionRegistry::default(),
    );
    assert!(analysis.project.is_some(), "{:#?}", analysis.diagnostics);

    let report = compile_project(
        &analysis.project.unwrap(),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    assert!(report.artifact.is_some(), "{:#?}", report.diagnostics);
}

#[test]
fn candidate_contract_distinguishes_frozen_clock_from_storage_and_putform() {
    let registry = default_host_registry();
    assert_eq!(
        registry.resolve("GETTIME").unwrap().contract.candidate,
        erabasic_bytecode::CandidatePolicy::FrozenClock
    );
    assert_eq!(
        registry.resolve("PUTFORM").unwrap().contract.candidate,
        erabasic_bytecode::CandidatePolicy::CloneCommit
    );
    assert_eq!(
        registry.resolve("BARSTR").unwrap().contract.candidate,
        erabasic_bytecode::CandidatePolicy::ReadOnly
    );
    assert_eq!(
        registry.resolve("SAVEDATA").unwrap().contract.candidate,
        erabasic_bytecode::CandidatePolicy::Forbidden
    );
    assert_eq!(
        registry.resolve("WAIT").unwrap().contract.candidate,
        erabasic_bytecode::CandidatePolicy::Forbidden
    );
}

#[test]
fn parallelism_does_not_change_artifact_bytes() {
    let project = analyze("@SYSTEM_TITLE\nCALL HELPER\nRETURN\n@HELPER\nRESULT = 1 + 2\nRETURN\n");
    let registry = default_host_registry();
    let one = compile_project(
        &project,
        &CompilerOptions {
            jobs: Some(1),
            ..CompilerOptions::default()
        },
        &registry,
        None,
    )
    .artifact
    .expect("single-thread compilation should succeed");
    let four = compile_project(
        &project,
        &CompilerOptions {
            jobs: Some(4),
            ..CompilerOptions::default()
        },
        &registry,
        None,
    )
    .artifact
    .expect("parallel compilation should succeed");
    assert_eq!(one, four);
    assert_eq!(
        encode_artifact(&one).unwrap(),
        encode_artifact(&four).unwrap()
    );
}

#[test]
fn incremental_patch_matches_a_clean_build() {
    let registry = default_host_registry();
    let first = analyze("@SYSTEM_TITLE\nCALL HELPER\nRETURN\n@HELPER\nRESULT = 1\nRETURN\n");
    let initial = compile_project(&first, &CompilerOptions::default(), &registry, None);
    assert_eq!(
        initial.incremental_state.base_artifact_id(),
        initial
            .artifact
            .as_ref()
            .map(|artifact| artifact.manifest.artifact_id)
    );
    let second = analyze("@SYSTEM_TITLE\nCALL HELPER\nRETURN\n@HELPER\nRESULT = 2\nRETURN\n");
    let incremental = compile_project(
        &second,
        &CompilerOptions::default(),
        &registry,
        Some(&initial.incremental_state),
    );
    assert!(
        incremental.diagnostics.is_empty(),
        "{:#?}",
        incremental.diagnostics
    );
    assert_eq!(incremental.stats.compiled_functions, 1);
    assert_eq!(incremental.stats.reused_functions, 1);
    let patch = incremental
        .patch
        .as_ref()
        .expect("a patch should be emitted");
    assert_eq!(patch.changed_functions.len(), 1);
    let patched = apply_patch(initial.artifact.as_ref().unwrap(), patch).unwrap();
    let target = incremental.artifact.as_ref().unwrap();
    assert_eq!(patched, *target);
    assert_eq!(
        encode_artifact(&patched).unwrap(),
        encode_artifact(target).unwrap()
    );
}

#[test]
fn compact_incremental_cache_reuses_the_exact_active_artifact() {
    let registry = default_host_registry();
    let first = analyze("@SYSTEM_TITLE\nCALL HELPER\nRETURN\n@HELPER\nRESULT = 1\nRETURN\n");
    let initial =
        compile_project_with_artifact(&first, &CompilerOptions::default(), &registry, None, None);
    let initial_artifact = initial.artifact.as_ref().unwrap();
    let second = analyze("@SYSTEM_TITLE\nCALL HELPER\nRETURN\n@HELPER\nRESULT = 2\nRETURN\n");
    let incremental = compile_project_with_artifact(
        &second,
        &CompilerOptions::default(),
        &registry,
        Some(&initial.incremental_state),
        Some(initial_artifact),
    );

    assert!(incremental.diagnostics.is_empty());
    assert_eq!(incremental.stats.compiled_functions, 1);
    assert_eq!(incremental.stats.reused_functions, 1);
    let patched = apply_patch(initial_artifact, incremental.patch.as_ref().unwrap()).unwrap();
    assert_eq!(Some(&patched), incremental.artifact.as_ref());
}

#[test]
fn compact_incremental_cache_without_its_artifact_falls_back_to_a_complete_patch() {
    let registry = default_host_registry();
    let first = analyze("@SYSTEM_TITLE\nCALL HELPER\nRETURN\n@HELPER\nRESULT = 1\nRETURN\n");
    let initial =
        compile_project_with_artifact(&first, &CompilerOptions::default(), &registry, None, None);
    let second = analyze("@SYSTEM_TITLE\nCALL HELPER\nRETURN\n@HELPER\nRESULT = 2\nRETURN\n");
    let incremental = compile_project(
        &second,
        &CompilerOptions::default(),
        &registry,
        Some(&initial.incremental_state),
    );

    assert!(incremental.diagnostics.is_empty());
    assert_eq!(incremental.stats.compiled_functions, 2);
    let patch = incremental.patch.as_ref().unwrap();
    assert_eq!(patch.changed_functions.len(), 2);
    let patched = apply_patch(initial.artifact.as_ref().unwrap(), patch).unwrap();
    assert_eq!(Some(&patched), incremental.artifact.as_ref());
}

#[test]
fn source_only_changes_keep_the_execution_identity() {
    let registry = default_host_registry();
    let first = compile_project(
        &analyze("@SYSTEM_TITLE\nRESULT = 1\nRETURN\n"),
        &CompilerOptions::default(),
        &registry,
        None,
    )
    .artifact
    .unwrap();
    let second = compile_project(
        &analyze("; moved line\n@SYSTEM_TITLE\nRESULT = 1\nRETURN\n"),
        &CompilerOptions::default(),
        &registry,
        None,
    )
    .artifact
    .unwrap();
    assert_eq!(
        first.manifest.program_version.execution_id,
        second.manifest.program_version.execution_id
    );
    assert_ne!(first.manifest.artifact_id, second.manifest.artifact_id);
    let first_fingerprints = first
        .source_map
        .entries
        .iter()
        .filter_map(|entry| first.source_map.statement_fingerprint(entry))
        .collect::<std::collections::BTreeSet<_>>();
    let second_fingerprints = second
        .source_map
        .entries
        .iter()
        .filter_map(|entry| second.source_map.statement_fingerprint(entry))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(first_fingerprints, second_fingerprints);
}

#[test]
fn structured_if_else_has_valid_split_entry_control_flow() {
    let project = analyze(
        "@SYSTEM_TITLE\nRESULT = 1\nIF RESULT\nPRINTL yes\nELSE\nPRINTL no\nENDIF\nRETURN\n",
    );
    let report = compile_project(
        &project,
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    assert!(report.artifact.is_some(), "{:#?}", report.diagnostics);
}

#[test]
fn structured_for_loop_reduces_control_state_to_one_branch_value() {
    let project = analyze("@SYSTEM_TITLE\nFOR COUNT:0, 0, 2\nRESULT += 1\nNEXT\nRETURN\n");
    let report = compile_project(
        &project,
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    assert!(report.artifact.is_some(), "{:#?}", report.diagnostics);
}

#[test]
fn structured_repeat_loop_lowers_to_executable_counter_bytecode() {
    let project = analyze("@SYSTEM_TITLE\nREPEAT 3\nRESULT += COUNT\nREND\nRETURN\n");
    let report = compile_project(
        &project,
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    let artifact = report
        .artifact
        .unwrap_or_else(|| panic!("{:#?}", report.diagnostics));
    let function = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE");

    assert!(
        function
            .code
            .iter()
            .any(|instruction| instruction.opcode == Opcode::ForStart as u16)
    );
    assert!(
        function
            .code
            .iter()
            .any(|instruction| instruction.opcode == Opcode::ForNext as u16)
    );
    assert!(
        !artifact
            .native_imports
            .iter()
            .any(|import| { import.import.name.eq_ignore_ascii_case("control_repeat") })
    );
}

#[test]
fn source_map_resolves_utf8_byte_locations() {
    let text = "@SYSTEM_TITLE\nPRINTL 中文\nRETURN\n";
    let artifact = compile_project(
        &analyze(text),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    )
    .artifact
    .unwrap();
    let function = &artifact.functions[0];
    let location = artifact
        .source_map
        .resolve(function.key, 0)
        .expect("first instruction should have a source mapping");
    assert_eq!(location.relative_path, "main.erb");
    assert_eq!(location.line, 2);
    assert_eq!(location.byte_column, "PRINTL ".len() as u64);
    assert_eq!(artifact.source_map.sources[0].byte_len, text.len() as u64);
}

#[test]
fn method_statement_stores_clock_result_in_result_variable() {
    let artifact = compile_project(
        &analyze("@SYSTEM_TITLE\nGETMILLISECOND\nRETURN\n"),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    )
    .artifact
    .expect("clock METHOD statement should compile");
    let opcodes: Vec<_> = artifact.functions[0]
        .code
        .iter()
        .map(|instruction| instruction.opcode)
        .collect();
    assert!(opcodes.contains(&(Opcode::CallHost as u16)));
    assert!(opcodes.contains(&(Opcode::StoreVariable as u16)));
}

#[test]
fn image_style_host_binding_still_uses_the_single_call_host_opcode() {
    let mut registry = default_host_registry();
    assert!(registry.register(
        "PRINTL",
        HostBinding {
            namespace: "game.graphics".into(),
            name: "show_image".into(),
            abi_version: 1,
            effect: HostEffect {
                pure: false,
                may_suspend: false,
                may_error: true,
                mutates_runtime: true,
            },
            capability: HostCapability::Graphics,
            snapshot_capability: HostSnapshotCapability::StableWait,
            contract: erabasic_bytecode::OperationContract {
                state: erabasic_bytecode::OperationState::Presentation,
                transaction: erabasic_bytecode::TransactionPolicy::CloneCommit,
                candidate: erabasic_bytecode::CandidatePolicy::CloneCommit,
                persistence: erabasic_bytecode::OperationPersistence::RuntimeOnly,
                snapshot: erabasic_bytecode::OperationSnapshotPolicy::Included,
                hot_reload: erabasic_bytecode::OperationHotReloadPolicy::Preserve,
                wait: erabasic_bytecode::OperationWaitPolicy::Immediate,
                capability_fallback: erabasic_bytecode::CapabilityFallback::CanonicalProjection,
                debug: erabasic_bytecode::OperationDebugPolicy::Forbidden,
                portability: erabasic_bytecode::OperationPortability::Portable,
            },
        },
    ));
    let artifact = compile_project(
        &analyze("@SYSTEM_TITLE\nPRINTL portrait.png\nRETURN\n"),
        &CompilerOptions::default(),
        &registry,
        None,
    )
    .artifact
    .unwrap();
    assert_eq!(artifact.host_imports[0].import.namespace, "game.graphics");
    assert_eq!(artifact.host_imports[0].import.name, "show_image");
    assert_eq!(
        artifact.functions[0]
            .code
            .iter()
            .filter(|instruction| instruction.opcode == Opcode::CallHost as u16)
            .count(),
        1
    );
}

#[test]
fn mixed_media_lengths_lower_to_deterministic_value_unit_pairs() {
    let artifact = compile_project(
        &analyze("@SYSTEM_TITLE\nPRINT_RECT 10px, 20, 30px, 40\nPRINT_SPACE 25px\nRETURN\n"),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    )
    .artifact
    .expect("mixed media program should compile");
    let rectangle = artifact
        .host_imports
        .iter()
        .find(|import| import.import.name.eq_ignore_ascii_case("print_rect"))
        .expect("PRINT_RECT host import");
    let space = artifact
        .host_imports
        .iter()
        .find(|import| import.import.name.eq_ignore_ascii_case("print_space"))
        .expect("PRINT_SPACE host import");
    assert_eq!(rectangle.import.parameters.len(), 8);
    assert_eq!(space.import.parameters.len(), 2);
}

#[test]
fn large_project_recompiles_only_the_changed_function() {
    let script = |changed: i64| {
        let mut text = String::new();
        for index in 0..512 {
            write!(
                text,
                "@FUNCTION_{index}\nRESULT = {}\nRETURN\n",
                if index == 300 { changed } else { index }
            )
            .unwrap();
        }
        text
    };
    let registry = default_host_registry();
    let initial = compile_project(
        &analyze(&script(1)),
        &CompilerOptions {
            jobs: Some(4),
            ..CompilerOptions::default()
        },
        &registry,
        None,
    );
    let updated = compile_project(
        &analyze(&script(2)),
        &CompilerOptions {
            jobs: Some(4),
            ..CompilerOptions::default()
        },
        &registry,
        Some(&initial.incremental_state),
    );
    assert!(updated.diagnostics.is_empty(), "{:#?}", updated.diagnostics);
    assert_eq!(updated.stats.total_functions, 512);
    assert_eq!(updated.stats.compiled_functions, 1);
    assert_eq!(updated.stats.reused_functions, 511);
}
use std::fmt::Write as _;

#[test]
fn compatibility_identity_invalidates_function_cache_and_artifact() {
    use erabasic_compat::{CompatibilityIdentity, CompatibilityProfileId};
    let registry = default_host_registry();
    let reference = analyze("@SYSTEM_TITLE\nCALL HELPER\nRETURN\n@HELPER\nRESULT = 1\nRETURN\n");
    let first = compile_project_with_artifact(
        &reference,
        &CompilerOptions::default(),
        &registry,
        None,
        None,
    );
    let original = first.artifact.as_ref().unwrap();
    let mut snake = reference.clone();
    snake.program.compatibility =
        CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake);
    let changed = compile_project_with_artifact(
        &snake,
        &CompilerOptions::default(),
        &registry,
        Some(&first.incremental_state),
        Some(original),
    );
    assert!(changed.artifact.is_some(), "{:?}", changed.diagnostics);
    assert_eq!(changed.stats.reused_functions, 0);
    let target = changed.artifact.as_ref().unwrap();
    assert_eq!(target.manifest.compatibility, snake.program.compatibility);
    assert_ne!(
        original.manifest.program_version.execution_id,
        target.manifest.program_version.execution_id
    );
    let same = compile_project_with_artifact(
        &snake,
        &CompilerOptions::default(),
        &registry,
        Some(&changed.incremental_state),
        Some(target),
    );
    assert_eq!(same.stats.compiled_functions, 0);
    assert_eq!(same.stats.reused_functions, 2);
    assert!(apply_patch(original, &erabasic_bytecode::create_patch(original, target)).is_err());
}

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
        &ValidationContext::for_artifact(&artifact),
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
