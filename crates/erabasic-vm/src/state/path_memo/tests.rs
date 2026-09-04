use super::*;
use crate::{
    HostCallRequest, HostCallResult, HostReady, ImmediateHostCall, ImmediateHostCallResult,
    NativeServiceRegistry, RunBudget, VmEvent, VmHost,
};
use erabasic_analyzer::{
    AnalysisInput, AnalyzerOptions, ExtensionRegistry, ProjectSource, SourcePayload,
    analyze_project,
};
use erabasic_compiler::{CompilerOptions, compile_project, default_host_registry};
use erabasic_csv::{CsvLoadOptions, ProjectFiles, load_project};
use erabasic_validator::{ValidationContext, validate_bytecode};
struct RejectHost;

impl VmHost for RejectHost {
    fn call(&mut self, _request: HostCallRequest) -> HostCallResult {
        HostCallResult::Error("unexpected host call".into())
    }
}

#[derive(Default)]
struct PureTextHost {
    calls: usize,
    safe: bool,
}

impl VmHost for PureTextHost {
    fn path_memo_safe(&self, import: &erabasic_bytecode::RuntimeImport) -> bool {
        self.safe && import.name.eq_ignore_ascii_case("HTML_TOPLAINTEXT")
    }

    fn call_immediate(&mut self, request: ImmediateHostCall<'_>) -> ImmediateHostCallResult {
        if !request
            .normalized_name
            .eq_ignore_ascii_case("HTML_TOPLAINTEXT")
        {
            return ImmediateHostCallResult::Unsupported;
        }
        self.calls += 1;
        ImmediateHostCallResult::Ready(HostReady {
            value: Some(VmValue::String("plain".into())),
            writes: Vec::new(),
        })
    }

    fn call(&mut self, _request: HostCallRequest) -> HostCallResult {
        HostCallResult::Error("unexpected deferred host call".into())
    }
}

fn compile_vm(source: &str) -> (Vm, Arc<BytecodeArtifact>) {
    compile_vm_with_profile(source, erabasic_compat::CompatibilityProfileId::EmueraEm)
}

fn compile_vm_with_profile(
    source: &str,
    profile: erabasic_compat::CompatibilityProfileId,
) -> (Vm, Arc<BytecodeArtifact>) {
    let project_data = load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
        .data
        .expect("default project data");
    let analysis = analyze_project(
        AnalysisInput {
            project_data,
            sources: vec![ProjectSource {
                relative_path: "main.erb".into(),
                payload: SourcePayload::Utf8(source.into()),
            }],
        },
        &AnalyzerOptions {
            compatibility: erabasic_compat::CompatibilityIdentity::for_profile(profile),
            ..AnalyzerOptions::analysis_mode()
        },
        &ExtensionRegistry::default(),
    );
    let compilation = compile_project(
        analysis.project.as_ref().expect("analyzed project"),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    let artifact = Arc::new(
        compilation
            .artifact
            .unwrap_or_else(|| panic!("{:#?}", compilation.diagnostics)),
    );
    let validation = validate_bytecode(
        artifact.as_ref().clone().into_unvalidated(),
        &ValidationContext::for_artifact(&artifact),
    );
    (
        Vm::new(
            validation
                .value
                .unwrap_or_else(|| panic!("validated artifact: {:#?}", validation.diagnostics)),
            VmConfig::default(),
        ),
        artifact,
    )
}

fn cached_entry(retained_bytes: usize) -> Arc<PathMemoEntry> {
    Arc::new(PathMemoEntry {
        dependencies: Vec::new(),
        safe_natives: Vec::new(),
        safe_hosts: Vec::new(),
        mutation_groups: Vec::new(),
        result: VmValue::Integer(0),
        result_dependency: None,
        body_instructions: 1,
        backward_branches: 0,
        retained_bytes,
    })
}

#[test]
fn path_memo_cache_borrows_value_arguments_without_weakening_equality() {
    let head = PathMemoHead {
        generation: GenerationId(1),
        function: SymbolKey::derive("test", b"function"),
    };
    let stored = vec![VmValue::String("same".into()), VmValue::Integer(7)];
    let mut cache = PathMemoCache::new();
    cache
        .entry(head)
        .or_default()
        .insert(stored, vec![cached_entry(1)]);

    let equal = [VmValue::String("same".into()), VmValue::Integer(7)];
    assert!(path_memo_entries(&cache, &head, &equal).is_some());
    assert!(
        path_memo_entries(
            &cache,
            &head,
            &[VmValue::Integer(7), VmValue::String("same".into())]
        )
        .is_none(),
        "argument order remains significant"
    );
    assert!(
        path_memo_entries(&cache, &head, &equal[..1]).is_none(),
        "argument length remains significant"
    );
    assert!(
        path_memo_entries(
            &cache,
            &head,
            &[VmValue::String("same".into()), VmValue::String("7".into())]
        )
        .is_none(),
        "integer and string values remain distinct"
    );
    assert!(Vm::path_memo_head(GenerationId(1), head.function, &equal).is_some());
    assert!(
        Vm::path_memo_head(
            GenerationId(1),
            head.function,
            &[VmValue::IntegerPlace(Box::default())]
        )
        .is_none(),
        "place arguments must be rejected before probing the cache"
    );
}

#[test]
fn path_memo_clear_and_generation_reclaim_keep_usage_exact() {
    let (mut vm, _) = compile_vm("@SYSTEM_TITLE\nRETURN\n");
    let retained_generation = vm.current_generation;
    let obsolete_generation = GenerationId(retained_generation.0 + 1);
    let program = vm
        .generations
        .get(&retained_generation)
        .expect("current generation")
        .clone();
    vm.generations.insert(obsolete_generation, program);
    let function = SymbolKey::derive("test", b"function");
    vm.path_memo_cache
        .entry(PathMemoHead {
            generation: retained_generation,
            function,
        })
        .or_default()
        .insert(vec![VmValue::Integer(1)], vec![cached_entry(11)]);
    vm.path_memo_cache
        .entry(PathMemoHead {
            generation: obsolete_generation,
            function,
        })
        .or_default()
        .insert(
            vec![VmValue::String("obsolete".into())],
            vec![cached_entry(13), cached_entry(17)],
        );
    (vm.path_memo_key_count, vm.path_memo_retained_bytes) =
        path_memo_cache_usage(&vm.path_memo_cache);
    assert_eq!(
        (vm.path_memo_key_count, vm.path_memo_retained_bytes),
        (2, 41)
    );

    vm.reclaim_generations();
    assert_eq!(
        (vm.path_memo_key_count, vm.path_memo_retained_bytes),
        (1, 11)
    );
    assert!(
        vm.path_memo_cache
            .keys()
            .all(|head| head.generation == retained_generation)
    );

    vm.clear_path_memo_cache();
    assert!(vm.path_memo_cache.is_empty());
    assert_eq!(
        (vm.path_memo_key_count, vm.path_memo_retained_bytes),
        (0, 0)
    );
}

#[test]
fn dynamic_path_memo_replays_an_explicit_character_parameter() {
    let (mut vm, artifact) = compile_vm(
        "@SYSTEM_TITLE\n\
             ADDVOIDCHARA\nADDVOIDCHARA\n\
             RESULT:10 = DYNAMIC_SET(0, 7)\n\
             CFLAG:1:5 = 0\n\
             RESULT:11 = DYNAMIC_SET(0, 7)\nRETURN RESULT\n\
             @DYNAMIC_SET, ARG, ARG:1\n#FUNCTION\n\
             CALLFORMF TARGET_{ARG}, ARG:1\nRETURNF RESULT\n\
             @TARGET_0(CFLAG:1:5)\n#FUNCTION\nRETURNF 1\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let cflag = artifact
        .globals
        .iter()
        .find(|global| global.name == "CFLAG")
        .expect("CFLAG")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    vm.spawn_entry(entry, Vec::new()).expect("spawn entry");
    let report = vm.run_slice(&mut RejectHost, &mut natives, RunBudget::default());

    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{:#?}",
        report.events
    );
    assert!(
        vm.path_memo_replays > 0,
        "the second call must physically replay"
    );
    assert_eq!(
        vm.read_variable(cflag, &[5], Some(1)),
        Ok(VmValue::Integer(7))
    );
    assert_eq!(
        vm.read_variable(cflag, &[5], Some(0)),
        Ok(VmValue::Integer(0))
    );
}

#[test]
fn path_memo_refreshes_a_unique_tail_result_read() {
    let (mut vm, artifact) = compile_vm(
        "@SYSTEM_TITLE\n\
             FLAG:0 = 10\n\
             RESULT:10 = REFRESH_RESULT(0)\n\
             FLAG:0 = 20\n\
             RESULT:11 = REFRESH_RESULT(0)\nRETURN RESULT\n\
             @REFRESH_RESULT, ARG\n#FUNCTION\n#DIM DYNAMIC OFFSET\n\
             SELECTCASE ARG\nCASE 0\n\
                 OFFSET = ARG + STRCOUNT(\"aaa\", \"a\") - 3\n\
                 RETURNF FLAG:OFFSET\n\
             CASEELSE\nRETURNF 0\nENDSELECT\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .expect("RESULT")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    vm.spawn_entry(entry, Vec::new()).expect("spawn entry");
    let report = vm.run_slice(&mut RejectHost, &mut natives, RunBudget::default());

    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{:#?}",
        report.events
    );
    assert_eq!(vm.path_memo_replays, 1);
    assert_eq!(
        (10..=11)
            .map(|index| vm.read_variable(result, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        [10, 20].map(VmValue::Integer)
    );
}

#[test]
fn path_memo_does_not_refresh_a_tail_place_read_twice() {
    let (mut vm, artifact) = compile_vm(
        "@SYSTEM_TITLE\n\
             FLAG:0 = 10\n\
             RESULT:10 = READ_TWICE(0)\n\
             FLAG:0 = 20\n\
             RESULT:11 = READ_TWICE(0)\nRETURN RESULT\n\
             @READ_TWICE, ARG\n#FUNCTION\n#DIM DYNAMIC DISCARD\n\
             DISCARD = FLAG:ARG\n\
             RETURNF FLAG:ARG\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .expect("RESULT")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    vm.spawn_entry(entry, Vec::new()).expect("spawn entry");
    let report = vm.run_slice(&mut RejectHost, &mut natives, RunBudget::default());

    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{:#?}",
        report.events
    );
    assert_eq!(vm.path_memo_replays, 0);
    assert_eq!(
        (10..=11)
            .map(|index| vm.read_variable(result, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        [10, 20].map(VmValue::Integer)
    );
}

#[test]
fn path_memo_result_refresh_still_validates_index_dependencies() {
    let (mut vm, artifact) = compile_vm(
        "@SYSTEM_TITLE\n\
             FLAG:0 = 10\nFLAG:1 = 20\nCOUNT = 0\n\
             RESULT:10 = READ_SELECTED()\n\
             COUNT = 1\n\
             RESULT:11 = READ_SELECTED()\nRETURN RESULT\n\
             @READ_SELECTED\n#FUNCTION\n#DIM DYNAMIC INDEX\n\
             INDEX = COUNT\n\
             RETURNF FLAG:INDEX\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .expect("RESULT")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    vm.spawn_entry(entry, Vec::new()).expect("spawn entry");
    let report = vm.run_slice(&mut RejectHost, &mut natives, RunBudget::default());

    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{:#?}",
        report.events
    );
    assert_eq!(vm.path_memo_replays, 0);
    assert_eq!(
        (10..=11)
            .map(|index| vm.read_variable(result, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        [10, 20].map(VmValue::Integer)
    );
}

#[test]
fn path_memo_only_crosses_hosts_with_a_current_purity_guarantee() {
    fn run(safe: bool) -> (usize, u64, Vec<VmValue>) {
        let (mut vm, artifact) = compile_vm(
            "@SYSTEM_TITLE\n\
                 RESULTS:10 '= PURE_TEXT(\"<b>x</b>\")\n\
                 RESULTS:11 '= PURE_TEXT(\"<b>x</b>\")\nRETURN\n\
                 @PURE_TEXT, ARGS\n#FUNCTIONS\n\
                 RETURNF HTML_TOPLAINTEXT(ARGS)\n",
        );
        let entry = artifact
            .functions
            .iter()
            .find(|function| function.name == "SYSTEM_TITLE")
            .expect("SYSTEM_TITLE")
            .key;
        let results = artifact
            .globals
            .iter()
            .find(|global| global.name == "RESULTS")
            .expect("RESULTS")
            .key;
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        let mut host = PureTextHost { calls: 0, safe };
        vm.spawn_entry(entry, Vec::new()).expect("spawn entry");
        let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{:#?}",
            report.events
        );
        (
            host.calls,
            vm.path_memo_replays,
            (10..=11)
                .map(|index| vm.read_variable(results, &[index], None).unwrap())
                .collect(),
        )
    }

    let safe = run(true);
    assert_eq!(safe.0, 1, "the second pure Host call should be replayed");
    assert_eq!(safe.1, 1);
    assert_eq!(
        safe.2,
        ["plain", "plain"].map(|value| VmValue::String(value.into()))
    );

    let unsafe_host = run(false);
    assert_eq!(
        unsafe_host.0, 2,
        "unclassified Host calls remain boundaries"
    );
    assert_eq!(unsafe_host.1, 0);
    assert_eq!(unsafe_host.2, safe.2);
}

#[test]
fn full_cell_replay_keeps_only_the_canonical_final_snapshot() {
    let (mut vm, artifact) = compile_vm(
        "@SYSTEM_TITLE\n\
             RESULT:10 = DYNAMIC_RESET(0)\n\
             RESULT:11 = DYNAMIC_RESET(0)\nRETURN\n\
             @DYNAMIC_RESET, ARG\n#FUNCTION\n\
             CALLFORMF RESET_{ARG}\nRETURNF RESULT\n\
             @RESET_0\n#FUNCTION\n#LOCALSSIZE 32\n\
             VARSET LOCALS\nLOCALS:5 = \"done\"\nRETURNF 7\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let locals = artifact
        .globals
        .iter()
        .find(|global| global.name == "LOCALS" && global.dimensions == [32])
        .expect("resized LOCALS")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    vm.spawn_entry(entry, Vec::new()).expect("spawn entry");
    let report = vm.run_slice(&mut RejectHost, &mut natives, RunBudget::default());

    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{:#?}",
        report.events
    );
    assert_eq!(vm.path_memo_replays, 1);
    let group = vm
        .path_memo_cache
        .values()
        .flat_map(|paths| paths.values())
        .flatten()
        .flat_map(|entry| &entry.mutation_groups)
        .find(|group| group.variable == locals)
        .expect("LOCALS mutation group");
    assert!(group.final_cell.is_some());
    assert!(
        group.mutations.is_empty(),
        "the final snapshot replaces the redundant mutation log"
    );
}

#[test]
fn dynamic_call_warnings_remain_path_memo_boundaries_after_site_deduplication() {
    let (mut vm, artifact) = compile_vm_with_profile(
        "@SYSTEM_TITLE\nRESULT:10 = WRAPPER()\nRESULT:11 = WRAPPER()\nRESULT:12 = WRAPPER()\nRETURN\n\
             @WRAPPER\n#FUNCTION\nCALLFORMF TARGET_0, 1, EXTRA()\nRETURNF RESULT\n\
             @TARGET_0(ARG)\n#FUNCTION\nFLAG:1 = 7\nRETURNF ARG\n\
             @EXTRA\n#FUNCTION\nFLAG:0 += 1\nRETURNF 99\n",
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let flag = artifact
        .globals
        .iter()
        .find(|variable| variable.name == "FLAG")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(&mut RejectHost, &mut natives, RunBudget::default());
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_eq!(report.events.iter().filter(|event| matches!(event,
            VmEvent::Diagnostic { code, origin, .. } if code == "compat.call.excess_arguments" && origin.function_name == "WRAPPER"
        )).count(), 1, "{report:?}");
    assert_eq!(vm.path_memo_replays, 0);
    assert!(vm.path_memo_cache.is_empty());
    assert_eq!(vm.read_variable(flag, &[0], None), Ok(VmValue::Integer(0)));
    assert_eq!(vm.read_variable(flag, &[1], None), Ok(VmValue::Integer(7)));
}

#[test]
fn over_budget_full_cell_groups_do_not_create_a_path_memo_entry() {
    let value = "x".repeat(512);
    let source = format!(
        "@SYSTEM_TITLE\n\
             RESULT:10 = DYNAMIC_GET(0)\n\
             RESULT:11 = DYNAMIC_GET(0)\nRETURN RESULT\n\
             @DYNAMIC_GET, ARG\n#FUNCTION\n\
             CALLFORMF TARGET_{{ARG}}\nRETURNF RESULT\n\
             @TARGET_0\n#FUNCTION\n\
             VARSET RESULTS, \"{value}\"\n\
             VARSET LOCALS, \"{value}\"\nRETURNF 1\n"
    );
    let (mut vm, artifact) = compile_vm(&source);
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    vm.spawn_entry(entry, Vec::new()).expect("spawn entry");
    let report = vm.run_slice(&mut RejectHost, &mut natives, RunBudget::default());

    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{:#?}",
        report.events
    );
    assert_eq!(vm.path_memo_replays, 0);
    assert!(
        vm.path_memo_cache.is_empty(),
        "the combined final-cell snapshots exceed the per-entry retained-memory budget"
    );
}
