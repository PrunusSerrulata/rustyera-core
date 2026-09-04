use super::*;
#[test]
fn regexpmatch_supports_positive_boundaries_without_consuming_adjacent_tokens() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n#DIM GROUP_COUNT\n#DIMS MATCHES, 4\nRESULT:0 = REGEXPMATCH(\"[$TOKEN:A][$TOKEN:B]\", \"(?<=\\\\[\\\\$TOKEN:).*?(?=\\\\])\", GROUP_COUNT, MATCHES)\nRESULT:1 = GROUP_COUNT\nRESULTS:10 '= MATCHES:0\nRESULTS:11 '= MATCHES:1\nRETURN RESULT\n",
    );
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .unwrap()
        .key;
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
        vm.read_variable(result, &[0], None),
        Ok(VmValue::Integer(2))
    );
    assert_eq!(
        vm.read_variable(result, &[1], None),
        Ok(VmValue::Integer(1))
    );
    assert_eq!(
        (10..=11)
            .map(|index| vm.read_variable(results, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        ["A", "B"]
            .map(|value| VmValue::String(value.into()))
            .to_vec()
    );
}

#[test]
fn one_dimensional_array_operations_accept_an_indexed_reference() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         RELATION:0:0 = 4\n\
         RELATION:0:1 = 8\n\
         RELATION:0:2 = 12\n\
         RELATION:0:3 = 16\n\
         RESULT:0 = FINDELEMENT(RELATION:0:0, 12, 0, 4)\n\
         ARRAYREMOVE RELATION:0:0, 1, 1\n\
         RESULT:1 = RELATION:0:1\n\
         RETURN RESULT\n",
    );
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
        vm.read_variable(result, &[0], None).unwrap(),
        VmValue::Integer(2)
    );
    assert_eq!(
        vm.read_variable(result, &[1], None).unwrap(),
        VmValue::Integer(12)
    );
}

#[test]
fn conditional_form_trims_branch_edge_whitespace() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         RESULTS:0 = \\@ 0 ? unused # %\"魔力\"% \\@\n\
         RESULTS:1 = \\@ 1 ? \tkept\t # unused \\@\n\
         RETURN RESULT\n",
    );
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .unwrap()
        .key;
    let entry = artifact.functions[0].key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
        vm.read_variable(results, &[0], None).unwrap(),
        VmValue::String("魔力".into())
    );
    assert_eq!(
        vm.read_variable(results, &[1], None).unwrap(),
        VmValue::String("kept".into())
    );
}

pub(in super::super) const CHARACTER_SHADOW_SOURCE: &str = "@SHADOW\n\
     #DIMS NAME\n\
     #DIMS CALLNAME\n\
     #DIMS NICKNAME\n\
     #DIMS MASTERNAME\n\
     #DIMS CSTR\n\
     #DIM NO\n\
     #DIM BASE\n\
     #DIM CFLAG\n\
     #DIM TARGET\n\
     RETURN\n\
     @SYSTEM_TITLE\n\
     ADDCHARA 1\n\
     ADDCHARA 2\n\
     TARGET = 2\n\
     RESULTS:0 '= NAME:1\n\
     RESULTS:1 '= CALLNAME:1\n\
     RESULTS:2 '= NICKNAME:1\n\
     RESULTS:3 '= MASTERNAME:1\n\
     RESULTS:4 '= CSTR:1:0\n\
     RESULTS:5 '= ANAME(1)\n\
     RESULTS:6 '= ANAME(2, 2)\n\
     RESULTS:7 '= CHARACTER_ROW(1)\n\
     RESULTS:8 '= CHARACTER_ROW(2)\n\
     RESULTS:9 '= NAME\n\
     RESULTS:10 '= CSTR:1:1\n\
     RESULT:11 = NO:1\n\
     RESULT:12 = BASE:1:0\n\
     RESULT:13 = CFLAG:1:0\n\
     RETURN RESULT\n\
     @ANAME(CHARA_ID = -999, CHARA_NUM = 1, ARG_SHOW_GUEST_JOB_TITLE = 1)\n\
     #FUNCTIONS\n\
     #DIM DYNAMIC CHARA_ID\n\
     #DIM DYNAMIC CHARA_NUM\n\
     #DIM DYNAMIC ARG_SHOW_GUEST_JOB_TITLE\n\
     IF CSTR:(CHARA_ID):0 != \"\"\n\
         RETURNF @\"%CSTR:(CHARA_ID):0%\\@CHARA_NUM > 1 ? 们 # \\@\"\n\
     ENDIF\n\
     RETURNF @\"%NAME:(CHARA_ID)%\\@CHARA_NUM > 1 ? 们 # \\@\"\n\
     @CHARACTER_ROW(CHARA_ID)\n\
     #FUNCTIONS\n\
     #DIM DYNAMIC CHARA_ID\n\
     RETURNF @\"◆%ANAME(CHARA_ID)%（女性）\"\n";
