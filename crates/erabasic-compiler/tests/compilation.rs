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
    HostBinding, compile_project, default_host_registry,
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
            ExecutionBinding::Unsupported { .. } => {}
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
        Some(ExecutionBinding::Unsupported { .. })
    ));
    assert!(matches!(
        registry.classification("CALLSHARP"),
        Some(ExecutionBinding::Unsupported { .. })
    ));
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
        .map(|entry| entry.statement_fingerprint)
        .collect::<std::collections::BTreeSet<_>>();
    let second_fingerprints = second
        .source_map
        .entries
        .iter()
        .map(|entry| entry.statement_fingerprint)
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
