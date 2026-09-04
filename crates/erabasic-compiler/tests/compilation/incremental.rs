use super::*;

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
