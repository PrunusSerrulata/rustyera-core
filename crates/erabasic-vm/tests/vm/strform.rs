use super::*;
use std::fmt::Write as _;

const ERAFL_TITLE_ERB: &str =
    include_str!("../../../../tools/runtime-tester/fixture-reference/erb/strform-title.erb");
const ERAFL_TITLE_ERH: &str =
    include_str!("../../../../tools/runtime-tester/fixture-reference/erb/strform-title.erh");
const ERAFL_TITLE_XML: &str =
    include_str!("../../../../tools/runtime-tester/fixture-reference/xml/CHARA_TITLE.xml");
const ERAFL_TITLE_FLAG_CSV: &str =
    include_str!("../../../../tools/runtime-tester/fixture-reference/csv/FLAG.CSV");

fn compile_erafl_title_fixture() -> BytecodeArtifact {
    let data = load_project(
        &ProjectFiles {
            csv: vec![FrontendFile {
                source_path: None,
                relative_path: "FLAG.CSV".into(),
                payload: CsvFilePayload::Utf8(ERAFL_TITLE_FLAG_CSV.into()),
            }],
            erb: Vec::new(),
        },
        &CsvLoadOptions::default(),
    )
    .data
    .expect("the real eraFL FLAG row should load");
    let mut options = AnalyzerOptions::analysis_mode();
    options.system_save_in_binary = true;
    let analysis = analyze_project(
        AnalysisInput {
            project_data: data,
            sources: vec![
                ProjectSource {
                    relative_path: "strform-title.erh".into(),
                    payload: SourcePayload::Utf8(ERAFL_TITLE_ERH.into()),
                },
                ProjectSource {
                    relative_path: "strform-title.erb".into(),
                    payload: SourcePayload::Utf8(ERAFL_TITLE_ERB.into()),
                },
            ],
        },
        &options,
        &ExtensionRegistry::default(),
    );
    assert!(
        analysis.project.is_some() && analysis.diagnostics.is_empty(),
        "archive-derived analysis: {:#?}",
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
        "archive-derived compilation: {:#?}",
        compilation.diagnostics
    );
    compilation.artifact.unwrap()
}

#[derive(Default)]
struct ArchiveFixtureHost {
    loadtext_calls: usize,
}

impl VmHost for ArchiveFixtureHost {
    fn call(&mut self, request: HostCallRequest) -> HostCallResult {
        if request.import.name.eq_ignore_ascii_case("LOADTEXT") {
            assert_eq!(
                request.arguments.first(),
                Some(&VmValue::String("XML/CHARA_TITLE.xml".into()))
            );
            self.loadtext_calls += 1;
            return HostCallResult::Ready(HostReady {
                value: Some(VmValue::String(ERAFL_TITLE_XML.into())),
                writes: Vec::new(),
            });
        }
        HostCallResult::Error(
            format!(
                "unexpected archive fixture host call: {}",
                request.import.name
            )
            .into(),
        )
    }
}

fn named_key(artifact: &BytecodeArtifact, name: &str) -> SymbolKey {
    artifact
        .globals
        .iter()
        .find(|global| global.name == name)
        .unwrap_or_else(|| panic!("missing fixture global {name}"))
        .key
}

fn completed_without_fault(report: &erabasic_vm::VmRunReport, fiber: FiberId) {
    assert_eq!(report.stop, erabasic_vm::VmRunStop::Idle);
    assert!(
        report.events.iter().any(
            |event| matches!(event, VmEvent::FiberCompleted { fiber: completed, value: None } if *completed == fiber)
        ),
        "{:#?}",
        report.events
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{:#?}",
        report.events
    );
}

#[test]
fn erafl_archive_fixture_matches_the_reference_cli_termination_and_watches() {
    assert!(
        ERAFL_TITLE_ERB.contains("IF !TOINT(STRFORM(TITLE_ACTIVATE_CONDITION))"),
        "the regression must retain CHARA_TITLE.ERB line 30"
    );
    assert!(
        ERAFL_TITLE_ERB.contains("RETURNF NO:(ARG:0) < MAX_FIXED_CHARA"),
        "the regression must retain eraFL's real IS_UNIQUE_CHARA"
    );
    assert_eq!(
        ERAFL_TITLE_FLAG_CSV.trim(),
        "500,領地評判_商業",
        "the regression must retain the real eraFL Flag.csv mapping"
    );
    assert!(
        ERAFL_TITLE_XML.contains("{FLAG:領地評判_商業 >= 150}"),
        "the regression must retain the archive's real title requirement"
    );
    assert!(
        ERAFL_TITLE_XML.contains("{FLAG:領地評判_商業 >= 300}"),
        "the regression must retain the next real merchant-title boundary"
    );
    assert!(
        ERAFL_TITLE_ERB.contains("IF !TOINT(STRFORM(TITLE_REQCONDITION))"),
        "the regression must retain the faulting CHARA_TITLE.ERB line 66"
    );

    let artifact = compile_erafl_title_fixture();
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "ORACLE_STRFORM_TITLE")
        .expect("ORACLE_STRFORM_TITLE")
        .key;
    let result = named_key(&artifact, "RESULT");
    let results = named_key(&artifact, "RESULTS");
    let mut host = ArchiveFixtureHost::default();
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());

    completed_without_fault(&report, fiber);
    assert_eq!(host.loadtext_calls, 1);
    let watches = [
        ("RESULT:80", vm.read_variable(result, &[80], None)),
        ("RESULT:81", vm.read_variable(result, &[81], None)),
        ("RESULT:82", vm.read_variable(result, &[82], None)),
        ("RESULT:83", vm.read_variable(result, &[83], None)),
        ("RESULT:84", vm.read_variable(result, &[84], None)),
        ("RESULT:85", vm.read_variable(result, &[85], None)),
        ("RESULTS:80", vm.read_variable(results, &[80], None)),
        ("RESULTS:81", vm.read_variable(results, &[81], None)),
    ];
    assert_eq!(
        watches,
        [
            ("RESULT:80", Ok(VmValue::Integer(0))),
            ("RESULT:81", Ok(VmValue::Integer(1))),
            ("RESULT:82", Ok(VmValue::Integer(0))),
            ("RESULT:83", Ok(VmValue::Integer(1))),
            ("RESULT:84", Ok(VmValue::Integer(0))),
            ("RESULT:85", Ok(VmValue::Integer(1))),
            ("RESULTS:80", Ok(VmValue::String("0".into()))),
            ("RESULTS:81", Ok(VmValue::String("1".into()))),
        ],
        "these are the same completed termination watches asserted by both reference CLI smoke scripts"
    );
}

fn run_entry(artifact: &BytecodeArtifact, config: VmConfig) -> (Vm, erabasic_vm::VmRunReport) {
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let mut vm = Vm::new(validated(artifact), config);
    let mut natives = NativeServiceRegistry::for_artifact(artifact);
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    (vm, report)
}

fn compile_with_header(header: &str, source: &str, options: &AnalyzerOptions) -> BytecodeArtifact {
    compile_with_header_and_compiler(header, source, options, &CompilerOptions::default())
}

fn compile_with_header_and_compiler(
    header: &str,
    source: &str,
    options: &AnalyzerOptions,
    compiler: &CompilerOptions,
) -> BytecodeArtifact {
    let analysis = analyze_project(
        AnalysisInput {
            project_data: project_data(),
            sources: vec![
                ProjectSource {
                    relative_path: "runtime-form.erh".into(),
                    payload: SourcePayload::Utf8(header.into()),
                },
                ProjectSource {
                    relative_path: "runtime-form.erb".into(),
                    payload: SourcePayload::Utf8(source.into()),
                },
            ],
        },
        options,
        &ExtensionRegistry::default(),
    );
    assert!(analysis.project.is_some(), "{:#?}", analysis.diagnostics);
    let compilation = compile_project(
        &analysis.project.unwrap(),
        compiler,
        &default_host_registry(),
        None,
    );
    assert!(
        compilation.artifact.is_some(),
        "{:#?}",
        compilation.diagnostics
    );
    compilation.artifact.unwrap()
}

const METHOD_FIXTURE_HEADER: &str =
    include_str!("../../../../tools/runtime-tester/fixture-snake-methods/erb/methods.erh");
const METHOD_FIXTURE_SOURCE: &str =
    include_str!("../../../../tools/runtime-tester/fixture-snake-methods/erb/methods.erb");

fn method_options(snake: bool) -> AnalyzerOptions {
    let mut options = AnalyzerOptions::analysis_mode();
    if snake {
        options.compatibility = erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        );
    }
    options
}

fn run_method_case(
    artifact: &BytecodeArtifact,
    name: &str,
    config: VmConfig,
) -> (Vm, erabasic_vm::VmRunReport) {
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == name)
        .expect("method fixture entry")
        .key;
    let mut vm = Vm::new(validated(artifact), config);
    let mut natives = NativeServiceRegistry::for_artifact_with_seed(artifact, 123_456);
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    (vm, report)
}

fn assert_method_watch(
    vm: &Vm,
    artifact: &BytecodeArtifact,
    name: &str,
    index: u64,
    expected: VmValue,
) {
    assert_eq!(
        vm.read_variable(named_key(artifact, name), &[index], None),
        Ok(expected),
        "{name}:{index}"
    );
}

#[path = "strform/call_text.rs"]
mod call_text;
#[path = "strform/checked_forms.rs"]
mod checked_forms;
#[path = "strform/checkpoints.rs"]
mod checkpoints;
#[path = "strform/direct_host.rs"]
mod direct_host;
#[path = "strform/dynamic_method_snapshots.rs"]
mod dynamic_method_snapshots;
#[path = "strform/dynamic_methods.rs"]
mod dynamic_methods;
#[path = "strform/dynamic_natives.rs"]
mod dynamic_natives;
#[path = "strform/lease_snapshots.rs"]
mod lease_snapshots;
#[path = "strform/maps.rs"]
mod maps;
#[path = "strform/references.rs"]
mod references;
#[path = "strform/runtime_forms.rs"]
mod runtime_forms;
#[path = "strform/staged_data.rs"]
mod staged_data;
use checkpoints::restructuring::{
    corrupt_native_form_snapshot, reject_form_snapshot_before_native_restore,
};
use dynamic_method_snapshots::take_fault;
use lease_snapshots::lease_snapshot_natives;
