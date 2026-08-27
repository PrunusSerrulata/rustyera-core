use super::language::CHARACTER_SHADOW_SOURCE;
use super::*;
fn character_shadowing_artifact() -> BytecodeArtifact {
    let loaded = load_project(
        &ProjectFiles {
            csv: vec![
                FrontendFile {
                    source_path: None,
                    relative_path: "Chara/Chara1.csv".into(),
                    payload: CsvFilePayload::Utf8(
                        "NO,1\nNAME,琉米爱尔\nCALLNAME,露米\nNICKNAME,小露\nMASTERNAME,主人甲\nBASE,0,123\nCFLAG,0,7\nCSTR,1,称号甲\n"
                            .into(),
                    ),
                },
                FrontendFile {
                    source_path: None,
                    relative_path: "Chara/Chara2.csv".into(),
                    payload: CsvFilePayload::Utf8(
                        "NO,2\nNAME,奥莉薇娅\nCALLNAME,奥莉\nNICKNAME,小奥\nMASTERNAME,主人乙\n"
                            .into(),
                    ),
                },
            ],
            erb: Vec::new(),
        },
        &CsvLoadOptions {
            search_subdirectories: true,
            ..CsvLoadOptions::default()
        },
    )
    .data
    .expect("character CSV fixtures should load");
    let mut artifact = compile_source_with_data(CHARACTER_SHADOW_SOURCE, loaded);
    let shadow = artifact
        .functions
        .iter()
        .find(|function| function.name == "SHADOW")
        .expect("SHADOW")
        .key;
    // Bytecode references variables by SymbolKey, so declaration order is not
    // semantic. Force the valid ordering captured in the EraFL snapshot.
    artifact.refresh_ids().expect("fixture ids should refresh");
    artifact
        .globals
        .sort_by_key(|definition| definition.owner != Some(shadow));
    artifact
}

fn assert_shadowed_definitions_precede_canonical(artifact: &BytecodeArtifact) {
    let shadow = artifact
        .functions
        .iter()
        .find(|function| function.name == "SHADOW")
        .expect("SHADOW")
        .key;
    for name in [
        "NAME",
        "CALLNAME",
        "NICKNAME",
        "MASTERNAME",
        "CSTR",
        "NO",
        "BASE",
        "CFLAG",
        "TARGET",
    ] {
        let local = artifact
            .globals
            .iter()
            .position(|global| global.owner == Some(shadow) && global.name == name)
            .unwrap_or_else(|| panic!("function-local {name}"));
        let canonical = artifact
            .globals
            .iter()
            .position(|global| global.owner.is_none() && global.name == name)
            .unwrap_or_else(|| panic!("canonical {name}"));
        assert!(
            local < canonical,
            "fixture must reproduce the declaration ordering that exposed {name} shadowing"
        );
    }
}

fn assert_canonical_character_name_lookup(vm: &Vm) {
    for name in [
        "NAME",
        "CALLNAME",
        "NICKNAME",
        "MASTERNAME",
        "CSTR",
        "NO",
        "BASE",
        "CFLAG",
    ] {
        let definition = vm
            .global_by_name(name)
            .unwrap_or_else(|| panic!("canonical {name}"));
        assert_eq!(definition.owner, None, "{name}");
        assert_eq!(definition.storage, BytecodeStorage::Character, "{name}");
    }
    let target = vm.global_by_name("TARGET").expect("canonical TARGET");
    assert_eq!(target.owner, None);
    assert_eq!(target.storage, BytecodeStorage::Project);
}

#[test]
fn character_csv_names_survive_same_named_function_locals() {
    let artifact = character_shadowing_artifact();
    assert_shadowed_definitions_precede_canonical(&artifact);
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .unwrap()
        .key;
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    assert_canonical_character_name_lookup(&vm);
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{:#?}",
        report.events
    );
    assert_eq!(
        (0..=10)
            .map(|index| vm.read_variable(results, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        [
            "琉米爱尔",
            "露米",
            "小露",
            "主人甲",
            "",
            "琉米爱尔",
            "奥莉薇娅们",
            "◆琉米爱尔（女性）",
            "◆奥莉薇娅（女性）",
            "奥莉薇娅",
            "称号甲",
        ]
        .map(|value| VmValue::String(value.into()))
        .to_vec()
    );
    let result = artifact
        .globals
        .iter()
        .find(|global| global.owner.is_none() && global.name == "RESULT")
        .expect("RESULT")
        .key;
    assert_eq!(
        (11..=13)
            .map(|index| vm.read_variable(result, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        [1, 123, 7].map(VmValue::Integer).to_vec()
    );
}
