use super::*;

const ERAFL_TOOLTIP_ERB: &str = include_str!(
    "../../../../tools/runtime-tester/fixture-reference/erb/erafl-title-bonus-tooltip.erb"
);
const ERAFL_TOOLTIP_ERH: &str = include_str!(
    "../../../../tools/runtime-tester/fixture-reference/erb/erafl-title-bonus-tooltip.erh"
);
const ERAFL_TOOLTIP_XML: &str = include_str!(
    "../../../../tools/runtime-tester/fixture-reference/xml/CHARA_TITLE_BONUS_TOOLTIP.xml"
);

fn compile_erafl_tooltip_fixture() -> BytecodeArtifact {
    let analysis = analyze_project(
        AnalysisInput {
            project_data: project_data(),
            sources: vec![
                ProjectSource {
                    relative_path: "erafl-title-bonus-tooltip.erh".into(),
                    payload: SourcePayload::Utf8(ERAFL_TOOLTIP_ERH.into()),
                },
                ProjectSource {
                    relative_path: "erafl-title-bonus-tooltip.erb".into(),
                    payload: SourcePayload::Utf8(ERAFL_TOOLTIP_ERB.into()),
                },
            ],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );
    assert!(
        analysis.project.is_some() && analysis.diagnostics.is_empty(),
        "real eraFL tooltip analysis: {:#?}",
        analysis.diagnostics
    );
    let compilation = compile_project(
        &analysis.project.unwrap(),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    assert!(
        compilation.artifact.is_some(),
        "real eraFL tooltip compilation: {:#?}",
        compilation.diagnostics
    );
    compilation.artifact.unwrap()
}

#[derive(Default)]
struct EraflTooltipHost {
    loadtext_calls: usize,
}

impl VmHost for EraflTooltipHost {
    fn call(&mut self, request: HostCallRequest) -> HostCallResult {
        if request.import.name.eq_ignore_ascii_case("LOADTEXT") {
            assert_eq!(
                request.arguments.first(),
                Some(&VmValue::String("XML/CHARA_TITLE_BONUS_TOOLTIP.xml".into()))
            );
            self.loadtext_calls += 1;
            return HostCallResult::Ready(HostReady {
                value: Some(VmValue::String(ERAFL_TOOLTIP_XML.into())),
                writes: Vec::new(),
            });
        }
        HostCallResult::Error(
            format!(
                "unexpected eraFL tooltip fixture host call: {}",
                request.import.name
            )
            .into(),
        )
    }
}

#[test]
fn erafl_title_bonus_tooltips_replace_real_xml_value_tags() {
    assert!(
        ERAFL_TOOLTIP_XML.contains("name=\"暴击率上昇Ⅰ\"")
            && ERAFL_TOOLTIP_XML.contains("[$VALUE:CRITICAL_BONUS]")
            && ERAFL_TOOLTIP_ERB.contains(
                r#"TEXT = %REPLACE(TEXT, @"\\[\\$VALUE:%DISP_ID_NAME:LOCAL%\\]", @"%RESULTS%")%"#,
            ),
        "the regression must retain eraFL's real XML marker and replacement call"
    );
    let artifact = compile_erafl_tooltip_fixture();
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "ORACLE_ERAFL_TITLE_BONUS_TOOLTIP")
        .expect("ORACLE_ERAFL_TITLE_BONUS_TOOLTIP")
        .key;
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .expect("RESULTS")
        .key;
    let mut host = EraflTooltipHost::default();
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());

    assert_eq!(report.stop, erabasic_vm::VmRunStop::Idle);
    assert!(
        report.events.iter().any(
            |event| matches!(event, VmEvent::FiberCompleted { fiber: completed, value: None } if *completed == fiber)
        ),
        "{:#?}",
        report.events
    );
    assert_eq!(host.loadtext_calls, 1);
    assert_eq!(
        [
            vm.read_variable(results, &[90], None),
            vm.read_variable(results, &[91], None),
            vm.read_variable(results, &[92], None),
            vm.read_variable(results, &[93], None),
            vm.read_variable(results, &[94], None),
        ],
        [
            Ok(VmValue::String("属性:暴击率加成+4％".into())),
            Ok(VmValue::String("属性:暴击加成+8％".into())),
            Ok(VmValue::String(
                "【所持素質<魔術師>】<魔術師>导致的防御力惩罚降低5％，减伤值惩罚降低10％\\n【所持素質<神聖術師>】<神聖術師>导致的攻击力惩罚降低5％\\n【所持素質<魔人>】<魔人>导致的魔力惩罚降低5％\\n【未持有魔術系天赋】魔力计算惩罚降低15％".into()
            )),
            Ok(VmValue::String("Ax By C".into())),
            Ok(VmValue::String("a-b".into())),
        ]
    );
}
