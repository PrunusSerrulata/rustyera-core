use super::*;

#[test]
fn safe_sql_calls_emit_validated_v1_host_imports_with_physical_variadic_arity() {
    let project = analyze_snake(
        "@SYSTEM_TITLE\nSQL_CONNECT \"discarded\"\nRESULT:0 = SQL_CONNECT(\"db\")\nRESULT:1 = SQL_CONNECT(\"db\", )\nRESULT:2 = SQL_P_EXECUTE_NONQUERY(\"db\", \"SELECT @0\")\nRESULT:3 = SQL_P_EXECUTE_NONQUERY(\"db\", \"SELECT @0\", \"one\")\nRESULT:4 = SQL_P_EXECUTE_NONQUERY(\"db\", \"SELECT @0, @1\", , \"two\")\nRESULT:5 = SQL_P_EXECUTE_NONQUERY(\"db\", \"SELECT @0, @1\", \"one\", \"two\")\nSQL_P_EXECUTE_NONQUERY \"db\", \"SELECT @0, @1, @2\", \"one\", \"two\",\nSQL_DISCONNECT \"discarded\"\nRETURN\n",
    );
    let report = compile_project(
        &project,
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    assert!(report.diagnostics.is_empty(), "{:#?}", report.diagnostics);
    let artifact = report.artifact.expect("safe SQL calls should compile");
    let sql_imports = artifact
        .host_imports
        .iter()
        .filter(|host| host.import.namespace == "rustyera.sql")
        .collect::<Vec<_>>();
    assert_eq!(sql_imports.len(), 7);
    assert!(sql_imports.iter().all(|host| {
        host.import.abi_version == 1
            && host.capability == HostCapability::Sql
            && host.contract.state == erabasic_bytecode::OperationState::External
            && host.contract.transaction == erabasic_bytecode::TransactionPolicy::Forbidden
            && host.contract.persistence == erabasic_bytecode::OperationPersistence::ProjectDerived
            && host.contract.snapshot == erabasic_bytecode::OperationSnapshotPolicy::PendingBlocks
            && host.contract.hot_reload == erabasic_bytecode::OperationHotReloadPolicy::ActiveBlocks
            && host.contract.wait == erabasic_bytecode::OperationWaitPolicy::TransientExternal
            && host.contract.debug == erabasic_bytecode::OperationDebugPolicy::Forbidden
            && host.contract.portability == erabasic_bytecode::OperationPortability::Portable
            && host.contract.is_coherent()
    }));
    let parameter_shapes = sql_imports
        .iter()
        .filter(|host| host.import.name == "sql_p_execute_nonquery")
        .map(|host| host.import.parameters.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        parameter_shapes,
        [
            vec![BytecodeType::String, BytecodeType::String],
            vec![
                BytecodeType::String,
                BytecodeType::String,
                BytecodeType::String,
            ],
            vec![
                BytecodeType::String,
                BytecodeType::String,
                BytecodeType::Integer,
                BytecodeType::String,
            ],
            vec![
                BytecodeType::String,
                BytecodeType::String,
                BytecodeType::String,
                BytecodeType::String,
            ],
            vec![
                BytecodeType::String,
                BytecodeType::String,
                BytecodeType::String,
                BytecodeType::String,
                BytecodeType::Integer,
            ],
        ]
        .into_iter()
        .collect()
    );
    let omitted_sql_calls = omitted_host_calls(&artifact);
    assert!(
        omitted_sql_calls
            .iter()
            .any(|(name, omitted)| { name == "sql_p_execute_nonquery" && omitted == &[2] })
    );
    assert!(
        omitted_sql_calls
            .iter()
            .any(|(name, omitted)| { name == "sql_p_execute_nonquery" && omitted == &[4] })
    );

    let bytes = encode_artifact(&artifact).expect("SQL artifact should encode");
    let decoded = decode_artifact(&bytes, &DecodeLimits::default()).expect("SQL artifact decodes");
    let validation = validate_bytecode(
        decoded,
        &erabasic_compiler::runtime_native_validation_context(&artifact, &default_host_registry()),
    );
    assert!(validation.is_valid(), "{:#?}", validation.diagnostics);
}

#[test]
fn every_deferred_sql_call_reports_missing_capability_without_runtime_traps() {
    for (name, call, capability) in [
        (
            "SQL_CONNECTION_OPEN",
            "SQL_CONNECTION_OPEN \"db\"",
            "rustyera.sql.connection-open@future",
        ),
        (
            "SQL_READER_GET_FLOAT",
            "SQL_READER_GET_FLOAT 1, 0",
            "rustyera.sql.float@future",
        ),
        (
            "SQL_EXECUTE_SCALAR_FLOAT",
            "SQL_EXECUTE_SCALAR_FLOAT \"db\", \"SELECT 1\"",
            "rustyera.sql.float@future",
        ),
        (
            "SQL_P_EXECUTE_SCALAR_FLOAT",
            "SQL_P_EXECUTE_SCALAR_FLOAT \"db\", \"SELECT @0\", \"1\"",
            "rustyera.sql.float@future",
        ),
        (
            "SQL_ESCAPE",
            "SQL_ESCAPE \"text\"",
            "rustyera.sql.escape@future",
        ),
        (
            "SQL_IMPORT_DT_XML",
            "SQL_IMPORT_DT_XML \"db\", \"table\", \"schema.xml\", \"data.xml\"",
            "rustyera.sql.dt-xml@future",
        ),
        (
            "SQL_EXPORT_MAP_XML",
            "SQL_EXPORT_MAP_XML \"db\", \"table\", \"data.xml\"",
            "rustyera.sql.xml-export@future",
        ),
        (
            "SQL_EXPORT_DT_XML",
            "SQL_EXPORT_DT_XML \"db\", \"table\", \"schema.xml\", \"data.xml\"",
            "rustyera.sql.xml-export@future",
        ),
        (
            "SQL_IMPORT_XML_CUSTOM",
            "SQL_IMPORT_XML_CUSTOM \"db\", \"table\", \"data.xml\", \"row\", \"key\"",
            "rustyera.sql.custom-xml@future",
        ),
    ] {
        let source = format!("@SYSTEM_TITLE\n{call}\nRETURN\n");
        let report = compile_project(
            &analyze_snake(&source),
            &CompilerOptions::default(),
            &default_host_registry(),
            None,
        );
        assert!(report.artifact.is_none(), "{name} unexpectedly compiled");
        assert!(
            report.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == CompilerDiagnosticCode::MissingCapability
                    && diagnostic.message.contains(capability)
            }),
            "{name}: {:#?}",
            report.diagnostics
        );
        assert!(
            !report.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == CompilerDiagnosticCode::UnsupportedConstruct
                    || diagnostic.message.contains("unknown function")
            }),
            "{name}: {:#?}",
            report.diagnostics
        );
    }
}

#[test]
fn every_deferred_sql_name_has_a_deterministic_capability_classification() {
    let registry = default_host_registry();
    for name in [
        "SQL_CONNECTION_OPEN",
        "SQL_READER_GET_FLOAT",
        "SQL_EXECUTE_SCALAR_FLOAT",
        "SQL_P_EXECUTE_SCALAR_FLOAT",
        "SQL_ESCAPE",
        "SQL_IMPORT_DT_XML",
        "SQL_EXPORT_MAP_XML",
        "SQL_EXPORT_DT_XML",
        "SQL_IMPORT_XML_CUSTOM",
    ] {
        assert!(matches!(
            registry.classification(name),
            Some(ExecutionBinding::UnsupportedCapability { capability, reason })
                if capability.starts_with("rustyera.sql.") && !reason.is_empty()
        ));
    }
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
        &erabasic_compiler::runtime_native_validation_context(&artifact, &default_host_registry()),
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
            .any(|instruction| instruction.opcode == Opcode::ResolveUserCall as u16)
    );
    assert!(
        code.iter()
            .any(|instruction| instruction.opcode == Opcode::InvokeUserCall as u16)
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
            ExecutionBinding::BitArray
            | ExecutionBinding::ArrayMatch
            | ExecutionBinding::ExpressionMethod { .. }
            | ExecutionBinding::Unsupported { .. }
            | ExecutionBinding::UnsupportedCapability { .. } => {}
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
    for name in ["MATCHALL", "MATCHALLEX"] {
        assert!(matches!(
            registry.classification(name),
            Some(ExecutionBinding::ArrayMatch)
        ));
    }
}

#[test]
fn static_bit_and_match_openers_require_exact_staged_authorization() {
    let data = load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
        .data
        .unwrap();
    let analysis = analyze_project(
        AnalysisInput {
            project_data: data,
            sources: vec![ProjectSource {
                relative_path: "staged-auth.erb".into(),
                payload: SourcePayload::Utf8(
                    "@SYSTEM_TITLE\n#DIM BITS, 2\nRESULT:0 = BITGET(BITS, 0)\nRESULT:1 = MATCHALL(BITS, 0)\nRETURN\n"
                        .into(),
                ),
            }],
        },
        &AnalyzerOptions {
            compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
                erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
            ),
            ..AnalyzerOptions::analysis_mode()
        },
        &ExtensionRegistry::default(),
    );
    let artifact = compile_project(
        &analysis.project.expect("snake data calls analyze"),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    )
    .artifact
    .expect("snake data calls compile");
    let context =
        erabasic_compiler::runtime_native_validation_context(&artifact, &default_host_registry());

    let mut removed = artifact.clone();
    removed.runtime_staged_authorizations.clear();
    removed.refresh_ids().unwrap();
    let report = validate_bytecode(removed.into_unvalidated(), &context);
    assert!(report.value.is_none());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("lacks artifact authorization")),
        "{:#?}",
        report.diagnostics
    );

    let mut replaced = artifact.clone();
    let bit = replaced
        .runtime_staged_authorizations
        .iter_mut()
        .find(|family| family.name == "bitget")
        .unwrap();
    bit.shapes.clear();
    bit.key = bit.canonical_key();
    replaced.refresh_ids().unwrap();
    let report = validate_bytecode(replaced.into_unvalidated(), &context);
    assert!(report.value.is_none());
    assert!(
        report.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            erabasic_validator::ValidationCode::HostAbiMismatch
                | erabasic_validator::ValidationCode::InvalidOperand
        )),
        "{:#?}",
        report.diagnostics
    );
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
