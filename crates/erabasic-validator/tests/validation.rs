use erabasic_bytecode::{
    ArtifactManifest, BytecodeArtifact, BytecodeFunction, Digest, Opcode, SourceMap, SymbolKey,
    opcode,
};
use erabasic_csv::{CsvLoadOptions, ProjectFiles, load_project};
use erabasic_validator::{ValidationCode, ValidationContext, validate_bytecode};

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
        project_data: project_data(),
        globals: Vec::new(),
        native_imports: Vec::new(),
        host_imports: Vec::new(),
        functions: vec![BytecodeFunction {
            key: function_key,
            name: "BAD".into(),
            parameters: Vec::new(),
            result: None,
            imports: Vec::new(),
            code: vec![
                opcode::push_string("not an integer"),
                opcode::unary(1),
                opcode::return_value(false),
            ],
            max_stack: 1,
        }],
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
        project_data: project_data(),
        globals: Vec::new(),
        native_imports: Vec::new(),
        host_imports: Vec::new(),
        functions: vec![BytecodeFunction {
            key: function_key,
            name: "UNKNOWN".into(),
            parameters: Vec::new(),
            result: None,
            imports: Vec::new(),
            code: vec![erabasic_bytecode::EncodedInstruction {
                opcode: Opcode::Trap as u16 + 1,
                payload: Vec::new(),
            }],
            max_stack: 0,
        }],
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
fn call_host_is_the_only_host_boundary_opcode() {
    assert_eq!(Opcode::CallHost as u16, 35);
    assert_ne!(Opcode::CallNative as u16, Opcode::CallHost as u16);
}

#[test]
fn rejects_snapshot_vm_abi_mismatch() {
    let function_key = SymbolKey::derive("test.function", b"version");
    let mut artifact = BytecodeArtifact {
        manifest: ArtifactManifest::new(Digest::default()),
        project_data: project_data(),
        globals: Vec::new(),
        native_imports: Vec::new(),
        host_imports: Vec::new(),
        functions: vec![BytecodeFunction {
            key: function_key,
            name: "VERSION".into(),
            parameters: Vec::new(),
            result: None,
            imports: Vec::new(),
            code: vec![opcode::return_value(false)],
            max_stack: 0,
        }],
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
