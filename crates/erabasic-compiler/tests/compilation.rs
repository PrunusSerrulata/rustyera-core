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
use erabasic_validator::validate_bytecode;

fn analyze(text: &str) -> erabasic_analyzer::AnalyzedProject {
    analyze_with_options("main.erb", text, &AnalyzerOptions::analysis_mode())
}

fn analyze_snake(text: &str) -> erabasic_analyzer::AnalyzedProject {
    analyze_with_options(
        "snake.erb",
        text,
        &AnalyzerOptions {
            compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
                erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
            ),
            ..AnalyzerOptions::analysis_mode()
        },
    )
}

fn analyze_with_options(
    relative_path: &str,
    text: &str,
    options: &AnalyzerOptions,
) -> erabasic_analyzer::AnalyzedProject {
    let data = load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
        .data
        .expect("default project data should load");
    let report = analyze_project(
        AnalysisInput {
            project_data: data,
            sources: vec![ProjectSource {
                relative_path: relative_path.into(),
                payload: SourcePayload::Utf8(text.into()),
            }],
        },
        options,
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

fn omitted_host_calls(artifact: &erabasic_bytecode::BytecodeArtifact) -> Vec<(String, Vec<usize>)> {
    let mut calls = Vec::new();
    for function in &artifact.functions {
        for instruction in &function.code {
            if instruction.opcode != Opcode::CallHost as u16 || instruction.payload.len() == 7 {
                continue;
            }
            let import_index = u32::from_le_bytes(
                instruction.payload[..4]
                    .try_into()
                    .expect("Host import index"),
            ) as usize;
            let import_key = function.imports[import_index].key;
            let host = artifact
                .host_imports
                .iter()
                .find(|host| host.import.key == import_key)
                .expect("Host import resolves");
            let count = usize::from(u16::from_le_bytes(
                instruction.payload[7..9]
                    .try_into()
                    .expect("omission count"),
            ));
            let omissions = (0..count)
                .map(|index| {
                    usize::from(u16::from_le_bytes(
                        instruction.payload[9 + index * 2..11 + index * 2]
                            .try_into()
                            .expect("omitted argument index"),
                    ))
                })
                .collect::<Vec<_>>();
            calls.push((host.import.name.clone(), omissions));
        }
    }
    calls
}

#[path = "compilation/dynamic_lowering.rs"]
mod dynamic_lowering;
#[path = "compilation/host_contracts.rs"]
mod host_contracts;
#[path = "compilation/incremental.rs"]
mod incremental;
#[path = "compilation/lowering_basics.rs"]
mod lowering_basics;
