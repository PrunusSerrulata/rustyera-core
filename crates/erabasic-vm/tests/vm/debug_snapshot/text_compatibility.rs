use super::*;
#[test]
fn break_at_the_end_of_a_selected_branch_does_not_enter_else() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         RESULT:0 = BREAK_BRANCH()\n\
         RETURN RESULT\n\
         @BREAK_BRANCH()\n\
         #FUNCTION\n\
         #DIM LCOUNT\n\
         IF 1\n\
             FOR LCOUNT, 0, 1\n\
                 BREAK\n\
             NEXT\n\
         ELSE\n\
             RETURNF 99\n\
         ENDIF\n\
         RETURNF 7\n",
    );

    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(7));
}

#[test]
fn native_tail_matches_the_reference_oracle_fixture() {
    let artifact = compile_source(
        "@ORACLE_NATIVE\n#DIMS PARTS, 4\n#DIMS JOINED, 4\n#DIMS REPEATED, 1\nRESULT:0 = 0\nSETBIT RESULT:0, 1, 3\nINVERTBIT RESULT:0, 1\nCLEARBIT RESULT:0, 3\nSPLIT \"a//b/\", \"/\", PARTS, RESULT:1\nRESULT:2 = STRCOUNT(\"ababa\", \"aba\")\nRESULT:3 = GETPALAMLV(499, 5)\nRESULTS:0 = %ESCAPE(\"a+b\")%\nJOINED:0 = %\"a\"%\nJOINED:1 = %\"b\"%\nJOINED:2 = %\"c\"%\nRESULT:4 = STRLENS(\"Ab\")\nRESULT:5 = STRLENSU(\"Aé\")\nRESULT:12 = STRLENSU(\"😀\")\nRESULT:6 = STRFINDU(\"aβc\", \"β\")\nRESULT:7 = ENCODETOUNI(\"β\")\nRESULT:8 = UNICODEBYTE(\"β\")\nRESULTS:1 = %CHARATU(\"aβ\", 1)%\nRESULTS:2 = %TOUPPER(\"Abc\")%\nRESULTS:3 = %TOLOWER(\"AbC\")%\nRESULTS:4 = %STRJOIN(JOINED, \"/\", 1, 2)%\nRESULTS:5 = %STRJOIN(JOINED)%\nRESULTS:6 = %UNICODE(946)%\nRESULT:9 = TOINT(\"12.9\")\nRESULT:10 = ISNUMERIC(\"0x10\")\nRESULT:11 = COLOR_FROMRGB(1, 2, 3)\nRESULTS:7 = %CONVERT(255, 16)%\nREPEATED:0 '= \"<p>----------------</p>\"\nRESULT:13 = FINDELEMENT(REPEATED, \"([^ ])\\\\1{15}\", 0, 1, 0)\nRETURN RESULT\n",
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
        Ok(VmValue::Integer(0))
    );
    assert_eq!(
        vm.read_variable(result, &[1], None),
        Ok(VmValue::Integer(4))
    );
    assert_eq!(
        vm.read_variable(result, &[2], None),
        Ok(VmValue::Integer(1))
    );
    assert_eq!(
        vm.read_variable(result, &[3], None),
        Ok(VmValue::Integer(1))
    );
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("a\\+b".into()))
    );
    for (index, expected) in [(4, 2), (5, 2), (6, 1), (7, 946), (8, 946)] {
        assert_eq!(
            vm.read_variable(result, &[index], None),
            Ok(VmValue::Integer(expected))
        );
    }
    for (index, expected) in [(9, 12), (10, 1), (11, 0x0001_0203)] {
        assert_eq!(
            vm.read_variable(result, &[index], None),
            Ok(VmValue::Integer(expected))
        );
    }
    assert_eq!(
        vm.read_variable(result, &[12], None),
        Ok(VmValue::Integer(2))
    );
    assert_eq!(
        vm.read_variable(result, &[13], None),
        Ok(VmValue::Integer(0))
    );
    for (index, expected) in [
        (1, "β"),
        (2, "ABC"),
        (3, "abc"),
        (4, "b/c"),
        (5, "a,b,c,"),
        (6, "β"),
        (7, "ff"),
    ] {
        assert_eq!(
            vm.read_variable(results, &[index], None),
            Ok(VmValue::String(expected.into()))
        );
    }
}

#[test]
fn unicode_positions_and_lengths_follow_reference_encodings() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULT:0 = STRLENSU(\"A😀\")\nRESULT:1 = STRFINDU(\"A😀B\", \"B\")\nRESULT:2 = STRLENS(\"Aé\")\nRESULTS:0 = %CHARATU(\"A😀B\", 1)%\nRETURN RESULT\n",
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
        Ok(VmValue::Integer(3))
    );
    assert_eq!(
        vm.read_variable(result, &[1], None),
        Ok(VmValue::Integer(2))
    );
    assert_eq!(
        vm.read_variable(result, &[2], None),
        Ok(VmValue::Integer(2))
    );
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("😀".into()))
    );
}

#[test]
fn legacy_string_width_keeps_narrow_formcell_text_out_of_the_truncation_path() {
    let mut data = project_data();
    data.static_data.legacy_encoding = erabasic_data::LegacyEncoding::ChineseHans;
    let artifact = compile_source_with_data(
        "@SYSTEM_TITLE\n\
         RESULT:0 = STRLENS(\"们\")\n\
         RESULTS:0 = %FORMCELL(\"们\", 2)%\n\
         RESULTS:1 = %SUBSTRING(\"甲乙\", 0, 3)%\n\
         RETURN RESULT\n\
         @FORMCELL(ARGS, ARG = -1)\n\
         #FUNCTIONS\n\
         #DIMS DYNAMIC LOCALSTR\n\
         #DIM DYNAMIC WIDTH\n\
         WIDTH = STRLENS(ARGS)\n\
         IF ARG > 0 && WIDTH > ARG\n\
             LOCALSTR = %SUBSTRING(ARGS, 0, ARG - 3)%\n\
             WIDTH = STRLENS(LOCALSTR)\n\
             LOCALSTR += \".\" * (ARG - WIDTH)\n\
         ELSE\n\
             LOCALSTR = %ARGS%\n\
         ENDIF\n\
         RETURNF LOCALSTR\n",
        data,
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
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
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("们".into()))
    );
    assert_eq!(
        vm.read_variable(results, &[1], None),
        Ok(VmValue::String("甲乙".into()))
    );
}

#[test]
fn runtime_draw_line_width_prevents_formcell_from_truncating_to_a_negative_count() {
    let mut data = project_data();
    data.static_data.legacy_encoding = erabasic_data::LegacyEncoding::ChineseHans;
    data.static_data.replace.draw_line_string = "-".into();
    let artifact = compile_source_with_data(
        "@SYSTEM_TITLE\n\
         RESULT:0 = STRLENS(DRAWLINESTR)\n\
         RESULTS:0 = %FORMCELL(\"温馨提示：选择物品时，按住空格可以临时进入多选模式。\", MAXWIDTH(), \"LEFT\")%\n\
         RETURN RESULT\n\
         @MAXWIDTH\n\
         #FUNCTION\n\
         RETURNF STRLENS(DRAWLINESTR)\n\
         @FORMCELL(ARGS, ARG = -1, ARGS:1 = \"LEFT\", ARG:1 = 0)\n\
         #FUNCTIONS\n\
         #DIMS DYNAMIC LOCALSTR,1\n\
         #DIM DYNAMIC FORMCELLTEMP,3\n\
         FORMCELLTEMP = STRLENS(ARGS)\n\
         IF ARG > 0 && FORMCELLTEMP > ARG\n\
             IF ARG:1\n\
                 LOCALSTR = %SUBSTRING(ARGS, 0, ARG)%\n\
             ELSE\n\
                 LOCALSTR = %SUBSTRING(ARGS, 0, ARG - 3)%\n\
                 FORMCELLTEMP = STRLENS(LOCALSTR)\n\
                 LOCALSTR += \".\" * (ARG - FORMCELLTEMP)\n\
             ENDIF\n\
             FORMCELLTEMP = STRLENS(LOCALSTR)\n\
         ELSE\n\
             LOCALSTR = %ARGS%\n\
             SIF ARG < 0\n\
                 ARG = 0\n\
         ENDIF\n\
         RETURNF @\"%LOCALSTR,ARG,LEFT%\"\n",
        data,
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
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
    let mut vm = RuntimeVm::new(validated(&artifact), VmConfig::default());
    vm.set_line_columns(198);
    let prepared = vm
        .prepare_runtime_state(VmRuntimeStateTransaction::ResetGameData)
        .unwrap();
    vm.commit_runtime_state(prepared).unwrap();
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.drive(RunBudget::default(), VmDriveMode::Normal);

    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, erabasic_vm::VmPortEvent::FiberFaulted(..))),
        "{:#?}",
        report.events
    );
    assert!(matches!(
        vm.fiber_status(fiber),
        Some(FiberStatus::Completed(_))
    ));
    assert_eq!(
        vm.vm().read_variable(result, &[0], None),
        Ok(VmValue::Integer(198))
    );
    let VmValue::String(cell) = vm.vm().read_variable(results, &[0], None).unwrap() else {
        panic!("FORMCELL must return a string");
    };
    assert!(cell.starts_with("温馨提示：选择物品时，按住空格可以临时进入多选模式。"));
    assert_eq!(UnicodeWidthStr::width(cell.as_str()), 198);
}
