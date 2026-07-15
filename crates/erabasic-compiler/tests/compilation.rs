use erabasic_analyzer::{
    AnalysisInput, AnalyzerOptions, ExtensionRegistry, ProjectSource, SourcePayload,
    analyze_project, builtin_function_names, builtin_instruction_names,
};
use erabasic_bytecode::{
    BytecodeType, DecodeLimits, HostCapability, HostEffect, HostSnapshotCapability, Opcode,
    apply_patch, decode_artifact, encode_artifact,
};
use erabasic_compiler::{
    CompilerOptions, ExecutionBinding, HostBinding, compile_project, default_host_registry,
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
    assert!(getkey.effect.mutates_runtime);
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
fn every_analyzer_builtin_has_one_explicit_execution_class() {
    let registry = default_host_registry();
    for name in builtin_instruction_names()
        .into_iter()
        .chain(builtin_function_names())
    {
        assert!(
            registry.classification(&name).is_some(),
            "missing execution catalog entry for {name}"
        );
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
        Some(ExecutionBinding::Native)
    ));
    assert!(matches!(
        registry.classification("CALLSHARP"),
        Some(ExecutionBinding::Unsupported { .. })
    ));
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
