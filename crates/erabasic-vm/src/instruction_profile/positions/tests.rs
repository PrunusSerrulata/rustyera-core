use super::*;

pub(in crate::instruction_profile) fn maximum_snapshot() -> PositionSnapshot {
    let key = |index: usize| {
        (
            GenerationId(u64::MAX),
            SymbolKey::derive("profile", &index.to_le_bytes()),
            usize::MAX,
        )
    };
    PositionSnapshot {
        active: false,
        started_at_dispatches: u64::MAX.to_string(),
        ended_at_dispatches: u64::MAX.to_string(),
        incomplete: true,
        dropped_samples: u64::MAX.to_string(),
        counts: (0..MAXIMUM_POSITIONS)
            .map(|index| PositionCount {
                position: PositionIdentity::from(key(index)),
                samples: u64::MAX.to_string(),
            })
            .collect(),
        locations: (0..MAXIMUM_LOCATIONS)
            .map(|index| PositionLocation {
                position: PositionIdentity::from(key(index)),
                name: "\0".repeat(64),
                path: Some("\0".repeat(160)),
                path_truncated: true,
                line: Some(u64::MAX.to_string()),
            })
            .collect(),
    }
}

#[test]
fn position_windows_exclude_startup_and_checkpoint_dispatches_and_reset_capacity() {
    let mut profile = PositionProfile::default();
    let function = SymbolKey::derive("profile", b"test");
    profile.sample(GenerationId(1), function, 0);
    assert!(profile.counts.is_empty());
    profile.boundary(true, 100);
    for instruction in 0..=MAXIMUM_POSITIONS {
        profile.sample(GenerationId(1), function, instruction);
    }
    assert_eq!(profile.counts.len(), MAXIMUM_POSITIONS);
    assert_eq!(profile.dropped_samples, 1);
    assert!(profile.incomplete);
    profile.sample(GenerationId(1), function, 0);
    assert_eq!(profile.counts[&(GenerationId(1), function, 0)], 2);
    profile.boundary(false, 200);
    profile.sample(GenerationId(1), function, 0);
    assert_eq!(profile.counts[&(GenerationId(1), function, 0)], 2);
    assert_eq!(profile.ended_at_dispatches, 200);
    profile.boundary(true, 300);
    assert!(profile.counts.is_empty());
    assert!(!profile.incomplete);
    assert_eq!(profile.dropped_samples, 0);
    profile.sample(GenerationId(2), function, 1);
    assert_eq!(profile.counts[&(GenerationId(2), function, 1)], 1);
}

#[test]
fn position_overflow_is_explicit_and_clone_does_not_inherit_window() {
    let mut profile = crate::instruction_profile::InstructionProfile::default();
    let function = SymbolKey::derive("profile", b"test");
    profile.positions.boundary(true, 0);
    profile
        .positions
        .counts
        .insert((GenerationId(1), function, 0), u64::MAX);
    profile.positions.sample(GenerationId(1), function, 0);
    assert!(profile.positions.incomplete);
    let clone = profile.clone();
    assert!(!clone.positions.active);
    assert!(clone.positions.counts.is_empty());
    profile.positions.incomplete = false;
    profile.dispatches = u64::MAX;
    profile.observe(GenerationId(1), function, 0);
    assert!(profile.positions.incomplete);
}

fn artifact() -> erabasic_bytecode::BytecodeArtifact {
    crate::interpreter::literal_groupmatch_tests::compile_cursor_fixture(format!(
        "@SYSTEM_TITLE\n{}RETURN\n",
        "RESULT = 42\n".repeat(80)
    ))
}

fn validated(
    artifact: &erabasic_bytecode::BytecodeArtifact,
) -> erabasic_validator::ValidatedArtifact {
    let result = erabasic_validator::validate_bytecode(
        artifact.clone().into_unvalidated(),
        &erabasic_validator::ValidationContext::for_artifact(artifact),
    );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    result.value.unwrap()
}

#[test]
fn position_snapshot_resolves_indices_not_byte_offsets_and_missing_locations() {
    let artifact = artifact();
    let function = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut vm = Vm::new(validated(&artifact), crate::VmConfig::default());
    let generation = vm.current_generation;
    let program = &vm.generations[&generation];
    let code = &program.function(function).unwrap().code;
    let instruction = (0..code.len())
        .find(|&index| {
            program
                .source_location(function, index)
                .is_some_and(|source| source.line == 3)
        })
        .unwrap();
    let byte_offset: u64 = code[..instruction]
        .iter()
        .map(erabasic_bytecode::EncodedInstruction::encoded_len)
        .sum();
    assert_ne!(byte_offset, instruction as u64);
    vm.instruction_profile_boundary(true);
    vm.instruction_profile
        .positions
        .sample(generation, function, instruction);
    vm.instruction_profile
        .positions
        .sample(generation, function, usize::MAX);
    vm.instruction_profile
        .positions
        .sample(GenerationId(u64::MAX), function, 0);
    vm.instruction_profile_boundary(false);
    let snapshot = vm.instruction_profile.positions.snapshot(&vm);
    assert!(!snapshot.active);
    assert_eq!(snapshot.locations.len(), 3);
    assert_eq!(snapshot.locations[0].name, "SYSTEM_TITLE");
    assert_eq!(snapshot.locations[0].line.as_deref(), Some("3"));
    assert_eq!(snapshot.locations[0].path.as_deref(), Some("main.erb"));
    assert!(!snapshot.locations[0].path_truncated);
    assert_eq!(snapshot.locations[1].name, "SYSTEM_TITLE");
    assert!(snapshot.locations[1].path.is_none());
    assert!(snapshot.locations[1].line.is_none());
    assert_eq!(snapshot.locations[2].name, "<reclaimed generation>");
    assert!(snapshot.locations[2].path.is_none());
    assert!(snapshot.locations[2].line.is_none());
}

#[test]
fn position_projection_ranks_real_top64_and_bounds_unicode_scalars() {
    let original = artifact();
    for length in [160, 161] {
        let mut artifact = original.clone();
        artifact.functions[0].name = "界".repeat(65);
        artifact.source_map.sources[0].relative_path = "😀".repeat(length);
        artifact.refresh_ids().unwrap();
        let mut vm = Vm::new(validated(&artifact), crate::VmConfig::default());
        let function = artifact.functions[0].key;
        let generation = vm.current_generation;
        let code_len = artifact.functions[0].code.len();
        assert!(code_len > MAXIMUM_LOCATIONS);
        vm.instruction_profile_boundary(true);
        for instruction in 0..code_len {
            vm.instruction_profile
                .positions
                .counts
                .insert((generation, function, instruction), instruction as u64 + 1);
        }
        let snapshot = vm.instruction_profile.positions.snapshot(&vm);
        assert_eq!(snapshot.locations.len(), MAXIMUM_LOCATIONS);
        for (rank, location) in snapshot.locations.iter().enumerate() {
            assert_eq!(
                location.position.instruction,
                (code_len - rank - 1).to_string()
            );
            assert_eq!(location.name, "界".repeat(64));
            if location.path.is_some() {
                assert_eq!(location.path.as_deref(), Some("😀".repeat(160).as_str()));
                assert_eq!(location.path_truncated, length > 160);
            }
        }
        assert!(
            snapshot
                .locations
                .iter()
                .any(|location| location.path.is_some())
        );
    }
}

#[test]
fn position_observe_keeps_existing_phase_when_action_starts_between_intervals() {
    let mut profile = crate::instruction_profile::InstructionProfile::default();
    let function = SymbolKey::derive("profile", b"phase");
    for _ in 0..1000 {
        profile.observe(GenerationId(1), function, 0);
    }
    profile.positions.boundary(true, profile.dispatches);
    for _ in 0..23 {
        profile.observe(GenerationId(1), function, 1);
    }
    assert!(profile.positions.counts.is_empty());
    profile.observe(GenerationId(1), function, 2);
    assert_eq!(profile.positions.counts[&(GenerationId(1), function, 2)], 1);
    profile.positions.boundary(false, profile.dispatches);
    for _ in 0..1024 {
        profile.observe(GenerationId(1), function, 3);
    }
    assert_eq!(profile.counts[&(GenerationId(1), function)], 2);
    assert_eq!(profile.positions.counts.len(), 1);
    profile.positions.boundary(true, profile.dispatches);
    assert!(profile.positions.counts.is_empty());
    assert_eq!(profile.positions.started_at_dispatches, 2048);
}

struct NoHost;
impl crate::VmHost for NoHost {
    fn call(&mut self, _: crate::HostCallRequest) -> crate::HostCallResult {
        panic!("profile fixture has no host calls")
    }
}

#[test]
fn real_vm_clone_replacement_and_snapshot_restore_reset_diagnostic_identity_only() {
    let artifact = artifact();
    let mut vm = Vm::new(validated(&artifact), crate::VmConfig::default());
    let mut natives = crate::NativeServiceRegistry::for_artifact(&artifact);
    let before = vm.encode_unrestricted_snapshot(&natives).unwrap();
    vm.instruction_profile_boundary(true);
    for _ in 0..1024 {
        vm.instruction_profile
            .observe(vm.current_generation, artifact.functions[0].key, 1);
    }
    let instance = vm.instruction_profile.instance;
    let after = vm.encode_unrestricted_snapshot(&natives).unwrap();
    assert_eq!(after, before, "diagnostic state must not be persisted");
    let snapshot = crate::VmSnapshot::decode(&after, 64 * 1024 * 1024).unwrap();
    let restored = Vm::restore_snapshot(
        validated(&artifact),
        crate::VmConfig::default(),
        snapshot,
        &mut NoHost,
        &mut natives,
    )
    .unwrap();
    let clone = vm.clone();
    let replacement = Vm::new(validated(&artifact), crate::VmConfig::default());
    let identities: std::collections::BTreeSet<_> = [&vm, &clone, &replacement, &restored]
        .into_iter()
        .map(|vm| vm.instruction_profile.instance)
        .collect();
    assert_eq!(identities.len(), 4);
    for reset in [&clone, &replacement, &restored] {
        assert_ne!(reset.instruction_profile.instance, instance);
        assert_eq!(reset.instruction_profile.dispatches, 0);
        assert!(!reset.instruction_profile.positions.active);
        assert!(reset.instruction_profile.positions.counts.is_empty());
    }
    assert!(vm.instruction_profile.positions.active);
    vm.instruction_profile_boundary(false);
    let stopped = serde_json::to_value(vm.instruction_profile_snapshot()).unwrap();
    assert_eq!(
        stopped,
        serde_json::to_value(vm.instruction_profile_snapshot()).unwrap()
    );
}
