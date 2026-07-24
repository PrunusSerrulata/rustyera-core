use erabasic_bytecode::{
    ArtifactManifest, BytecodeArtifact, BytecodeFunction, BytecodeGlobal, BytecodePersistence,
    BytecodeStorage, BytecodeType, CapabilityFallback, Digest, HostCapability, HostEffect,
    HostImport, HostSnapshotCapability, Opcode, OperationContract, OperationDebugPolicy,
    OperationHotReloadPolicy, OperationPersistence, OperationSnapshotPolicy, OperationState,
    OperationWaitPolicy, RuntimeImport, SourceMap, SymbolKey, TransactionPolicy, opcode,
};
use erabasic_csv::{CsvLoadOptions, ProjectFiles, load_project};
use erabasic_validator::{
    ValidationCode, ValidationContext, validate_bytecode, validate_compiler_output,
};

fn project_data() -> erabasic_data::ProjectData {
    load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
        .data
        .expect("default project data should load")
}

#[test]
fn rejects_stack_type_mismatches_before_vm_execution() {
    let function_key = SymbolKey::derive("test.function", b"bad");
    let mut artifact = BytecodeArtifact {
        manifest: ArtifactManifest::new(Digest::default()),
        call_compatibility: erabasic_bytecode::BytecodeCallCompatibility::default(),
        project_data: project_data(),
        globals: Vec::new(),
        native_imports: Vec::new(),
        host_imports: Vec::new(),
        functions: vec![BytecodeFunction {
            key: function_key,
            name: "BAD".into(),
            kind: erabasic_bytecode::BytecodeFunctionKind::Normal,
            parameters: Vec::new(),
            result: None,
            labels: Vec::new(),
            imports: Vec::new(),
            code: vec![
                opcode::push_string("not an integer"),
                opcode::unary(1),
                opcode::return_value(false),
            ],
            max_stack: 1,
        }],
        event_groups: Vec::new(),
        source_map: SourceMap::default(),
    };
    artifact.refresh_ids().unwrap();
    let report = validate_bytecode(
        artifact.clone().into_unvalidated(),
        &ValidationContext::for_artifact(&artifact),
    );
    assert!(report.value.is_none());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == ValidationCode::TypeMismatch)
    );
}

#[test]
fn rejects_unknown_opcodes() {
    let function_key = SymbolKey::derive("test.function", b"unknown");
    let mut artifact = BytecodeArtifact {
        manifest: ArtifactManifest::new(Digest::default()),
        call_compatibility: erabasic_bytecode::BytecodeCallCompatibility::default(),
        project_data: project_data(),
        globals: Vec::new(),
        native_imports: Vec::new(),
        host_imports: Vec::new(),
        functions: vec![BytecodeFunction {
            key: function_key,
            name: "UNKNOWN".into(),
            kind: erabasic_bytecode::BytecodeFunctionKind::Normal,
            parameters: Vec::new(),
            result: None,
            labels: Vec::new(),
            imports: Vec::new(),
            code: vec![erabasic_bytecode::EncodedInstruction {
                opcode: Opcode::Trap as u16 + 1,
                payload: Vec::new().into(),
            }],
            max_stack: 0,
        }],
        event_groups: Vec::new(),
        source_map: SourceMap::default(),
    };
    artifact.refresh_ids().unwrap();
    let report = validate_bytecode(
        artifact.clone().into_unvalidated(),
        &ValidationContext::for_artifact(&artifact),
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == ValidationCode::UnknownOpcode)
    );
}

#[test]
fn compiler_output_validation_defers_identity_checks_only() {
    let function_key = SymbolKey::derive("test.function", b"compiler-output");
    let artifact = BytecodeArtifact {
        manifest: ArtifactManifest::new(Digest::default()),
        call_compatibility: erabasic_bytecode::BytecodeCallCompatibility::default(),
        project_data: project_data(),
        globals: Vec::new(),
        native_imports: Vec::new(),
        host_imports: Vec::new(),
        functions: vec![BytecodeFunction {
            key: function_key,
            name: "VALID".into(),
            kind: erabasic_bytecode::BytecodeFunctionKind::Normal,
            parameters: Vec::new(),
            result: None,
            labels: Vec::new(),
            imports: Vec::new(),
            code: vec![opcode::return_value(false)],
            max_stack: 0,
        }],
        event_groups: Vec::new(),
        source_map: SourceMap::default(),
    };
    let context = ValidationContext::for_artifact(&artifact);

    let compiler_report = validate_compiler_output(artifact.clone(), &context);
    assert!(
        compiler_report.is_valid(),
        "{:#?}",
        compiler_report.diagnostics
    );

    let untrusted_report = validate_bytecode(artifact.into_unvalidated(), &context);
    assert!(untrusted_report.value.is_none());
    assert!(
        untrusted_report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == ValidationCode::InvalidOperand)
    );
}

#[test]
fn accepts_a_builtin_array_disabled_by_variable_size() {
    let function_key = SymbolKey::derive("test.function", b"disabled-array");
    let variable_key = SymbolKey::derive("test.variable", b"A");
    let mut data = project_data();
    let schema = data.schema.variables.get_mut("A").expect("builtin A");
    assert!(schema.can_forbid);
    schema.dimensions = vec![0];
    let mut artifact = BytecodeArtifact {
        manifest: ArtifactManifest::new(Digest::default()),
        call_compatibility: erabasic_bytecode::BytecodeCallCompatibility::default(),
        project_data: data,
        globals: vec![BytecodeGlobal {
            key: variable_key,
            name: "A".into(),
            value_type: BytecodeType::Integer,
            dimensions: vec![0],
            mutable: true,
            storage: BytecodeStorage::Project,
            persistence: BytecodePersistence::GameSave,
            initial_values: Vec::new(),
            owner: None,
        }],
        native_imports: Vec::new(),
        host_imports: Vec::new(),
        functions: vec![BytecodeFunction {
            key: function_key,
            name: "VALID".into(),
            kind: erabasic_bytecode::BytecodeFunctionKind::Normal,
            parameters: Vec::new(),
            result: None,
            labels: Vec::new(),
            imports: Vec::new(),
            code: vec![opcode::return_value(false)],
            max_stack: 0,
        }],
        event_groups: Vec::new(),
        source_map: SourceMap::default(),
    };
    artifact.refresh_ids().unwrap();

    let report = validate_bytecode(
        artifact.clone().into_unvalidated(),
        &ValidationContext::for_artifact(&artifact),
    );

    assert!(report.value.is_some(), "{:#?}", report.diagnostics);
    assert!(report.diagnostics.is_empty(), "{:#?}", report.diagnostics);
}

#[test]
fn call_host_is_the_only_host_boundary_opcode() {
    assert_eq!(Opcode::CallHost as u16, 35);
    assert_ne!(Opcode::CallNative as u16, Opcode::CallHost as u16);
}

#[test]
fn total_variable_limit_counts_each_function_storage_group_independently() {
    let make_artifact = |same_owner: bool| {
        let first_function = SymbolKey::derive("test.function", b"first-frame");
        let second_function = SymbolKey::derive("test.function", b"second-frame");
        let functions = [first_function, second_function]
            .into_iter()
            .enumerate()
            .map(|(index, key)| BytecodeFunction {
                key,
                name: format!("FRAME_{index}"),
                kind: erabasic_bytecode::BytecodeFunctionKind::Normal,
                parameters: Vec::new(),
                result: None,
                labels: Vec::new(),
                imports: Vec::new(),
                code: vec![opcode::return_value(false)],
                max_stack: 0,
            })
            .collect();
        let mut artifact = BytecodeArtifact {
            manifest: ArtifactManifest::new(Digest::default()),
            call_compatibility: erabasic_bytecode::BytecodeCallCompatibility::default(),
            project_data: project_data(),
            globals: [first_function, second_function]
                .into_iter()
                .enumerate()
                .map(|(index, owner)| BytecodeGlobal {
                    key: SymbolKey::derive(
                        "test.variable",
                        format!("frame-local-{index}").as_bytes(),
                    ),
                    name: format!("LOCAL_{index}"),
                    value_type: BytecodeType::Integer,
                    dimensions: vec![6],
                    mutable: true,
                    storage: BytecodeStorage::FunctionLocal,
                    persistence: BytecodePersistence::None,
                    initial_values: Vec::new(),
                    owner: Some(if same_owner { first_function } else { owner }),
                })
                .collect(),
            native_imports: Vec::new(),
            host_imports: Vec::new(),
            functions,
            event_groups: Vec::new(),
            source_map: SourceMap::default(),
        };
        artifact.refresh_ids().unwrap();
        artifact
    };

    let separate_frames = make_artifact(false);
    let mut context = ValidationContext::for_artifact(&separate_frames);
    context.limits.maximum_total_variable_elements = 10;
    let report = validate_bytecode(separate_frames.into_unvalidated(), &context);
    assert!(report.value.is_some(), "{:#?}", report.diagnostics);

    let oversized_frame = make_artifact(true);
    let mut context = ValidationContext::for_artifact(&oversized_frame);
    context.limits.maximum_total_variable_elements = 10;
    let report = validate_bytecode(oversized_frame.into_unvalidated(), &context);
    assert!(report.value.is_none());
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == ValidationCode::ResourceLimit
            && diagnostic.message.contains("variable storage exceeds")
    }));
}

#[test]
fn rejects_snapshot_vm_abi_mismatch() {
    let function_key = SymbolKey::derive("test.function", b"version");
    let mut artifact = BytecodeArtifact {
        manifest: ArtifactManifest::new(Digest::default()),
        call_compatibility: erabasic_bytecode::BytecodeCallCompatibility::default(),
        project_data: project_data(),
        globals: Vec::new(),
        native_imports: Vec::new(),
        host_imports: Vec::new(),
        functions: vec![BytecodeFunction {
            key: function_key,
            name: "VERSION".into(),
            kind: erabasic_bytecode::BytecodeFunctionKind::Normal,
            parameters: Vec::new(),
            result: None,
            labels: Vec::new(),
            imports: Vec::new(),
            code: vec![opcode::return_value(false)],
            max_stack: 0,
        }],
        event_groups: Vec::new(),
        source_map: SourceMap::default(),
    };
    artifact.manifest.program_version.vm_abi += 1;
    artifact.refresh_ids().unwrap();
    let report = validate_bytecode(
        artifact.clone().into_unvalidated(),
        &ValidationContext::for_artifact(&artifact),
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == ValidationCode::UnsupportedVersion)
    );
}

#[test]
fn rejects_contradictory_persisted_operation_contracts() {
    let contract = OperationContract {
        state: OperationState::Pure,
        transaction: TransactionPolicy::ReadOnly,
        candidate: erabasic_bytecode::CandidatePolicy::ReadOnly,
        persistence: OperationPersistence::None,
        snapshot: OperationSnapshotPolicy::Included,
        hot_reload: OperationHotReloadPolicy::Preserve,
        wait: OperationWaitPolicy::Immediate,
        capability_fallback: CapabilityFallback::NotApplicable,
        debug: OperationDebugPolicy::Pure,
        portability: erabasic_bytecode::OperationPortability::Portable,
    };
    assert!(
        !OperationContract {
            candidate: erabasic_bytecode::CandidatePolicy::CloneCommit,
            ..contract
        }
        .is_coherent()
    );
    let mut artifact = BytecodeArtifact {
        manifest: ArtifactManifest::new(Digest::default()),
        call_compatibility: erabasic_bytecode::BytecodeCallCompatibility::default(),
        project_data: project_data(),
        globals: Vec::new(),
        native_imports: Vec::new(),
        host_imports: vec![HostImport {
            import: RuntimeImport {
                key: SymbolKey::derive("test.host", b"pure"),
                namespace: "test.host".into(),
                name: "pure".into(),
                abi_version: 1,
                parameters: Vec::new(),
                result: None,
            },
            effect: HostEffect {
                pure: false,
                ..contract.effect()
            },
            capability: HostCapability::System,
            snapshot_capability: HostSnapshotCapability::StableWait,
            contract,
        }],
        functions: Vec::new(),
        event_groups: Vec::new(),
        source_map: SourceMap::default(),
    };
    artifact.refresh_ids().unwrap();
    let report = validate_bytecode(
        artifact.clone().into_unvalidated(),
        &ValidationContext::for_artifact(&artifact),
    );
    assert!(report.value.is_none());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == ValidationCode::InvalidOperationContract)
    );
}
