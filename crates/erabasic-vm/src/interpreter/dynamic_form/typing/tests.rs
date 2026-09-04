use super::*;
use erabasic_analyzer::{
    AnalysisInput, AnalyzerOptions, ExtensionRegistry, ProjectSource, SourcePayload,
    analyze_project,
};
use erabasic_compiler::{CompilerOptions, compile_project, default_host_registry};
use erabasic_csv::{CsvLoadOptions, ProjectFiles, load_project};
use erabasic_parser::DefaultParserContext;
use std::sync::Arc;
fn program_source(source: &str) -> crate::ProgramGeneration {
    let mut options = AnalyzerOptions::analysis_mode();
    options.compatibility = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let analysis = analyze_project(
        AnalysisInput {
            project_data: load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
                .data
                .unwrap(),
            sources: vec![ProjectSource {
                relative_path: "typing.erb".into(),
                payload: SourcePayload::Utf8(source.into()),
            }],
        },
        &options,
        &ExtensionRegistry::default(),
    );
    let compiled = compile_project(
        &analysis.project.expect("source analysis"),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    crate::ProgramGeneration::new(Arc::new(compiled.artifact.expect("compiled source")))
}

fn program() -> crate::ProgramGeneration {
    program_source(
        "@SYSTEM_TITLE\nRESULT = ABS(FLAG)\nRETURN\n@ECHO(ARG)\n#FUNCTION\nRETURNF ARG\n",
    )
}

#[test]
fn runtime_nested_native_and_user_calls_consume_one_root_type_analysis() {
    struct NoHost;
    impl crate::VmHost for NoHost {
        fn call(&mut self, _: crate::HostCallRequest) -> crate::HostCallResult {
            panic!("unexpected Host service")
        }
    }
    for pattern in [0, 1, 2] {
        let expression = (0..24).fold("1".to_owned(), |inner, index| {
            let name = if pattern == 0 || pattern == 2 && index % 2 == 0 {
                "ABS"
            } else {
                "ECHO"
            };
            format!("{name}({inner})")
        });
        let program = program_source(&format!(
            "@SYSTEM_TITLE\nRESULTS '= STRFORM(\"{{{expression}}}\")\nRETURN\n@ECHO(ARG)\n#FUNCTION\nRETURNF ARG\n"
        ));
        let entry = program.function_by_name("SYSTEM_TITLE").unwrap().key;
        let context = erabasic_compiler::runtime_native_validation_context(
            &program.artifact,
            &default_host_registry(),
        );
        let validated = erabasic_validator::validate_bytecode(
            (*program.artifact).clone().into_unvalidated(),
            &context,
        )
        .value
        .unwrap();
        let mut vm = crate::Vm::new(validated, crate::VmConfig::default());
        let mut natives = NativeServiceRegistry::for_artifact(&program.artifact);
        vm.spawn_entry(entry, Vec::new()).unwrap();
        TYPE_VISITS.with(|visits| visits.set(0));
        let report = vm.run_slice(&mut NoHost, &mut natives, crate::RunBudget::default());
        assert!(
            report
                .events
                .iter()
                .any(|event| matches!(event, crate::VmEvent::FiberCompleted { .. })),
            "{report:?}"
        );
        // Form + interpolation + 24 calls + one literal. No call or return retypes its actual tree.
        assert_eq!(
            TYPE_VISITS.with(std::cell::Cell::get),
            27,
            "pattern {pattern}"
        );
    }
}

#[test]
fn nested_types_visit_each_node_once_in_normal_and_probe_modes() {
    let program = program();
    let function = program.function_by_name("SYSTEM_TITLE").unwrap().key;
    let natives = NativeServiceRegistry::for_artifact(&program.artifact);
    for names in [
        vec!["ABS"; 24],
        vec!["ECHO"; 24],
        (0..24)
            .map(|i| if i % 2 == 0 { "ABS" } else { "ECHO" })
            .collect(),
    ] {
        let source = names
            .iter()
            .rev()
            .fold("1".to_owned(), |inner, name| format!("{name}({inner})"));
        let parsed = erabasic_parser::parse_expression(&source, &DefaultParserContext::default());
        let expression = parsed.value.expect("nested expression");
        for probe in [false, true] {
            let mut analysis = TypeAnalysis::new(
                &program,
                function,
                GenerationId::default(),
                probe,
                25,
                Some(&natives),
            );
            assert_eq!(
                analysis.expression(&expression, 0).unwrap(),
                BytecodeType::Integer
            );
            assert_eq!(analysis.nodes(), 25, "{probe}: {source}");
            let mut bounded = TypeAnalysis::new(
                &program,
                function,
                GenerationId::default(),
                probe,
                24,
                Some(&natives),
            );
            assert_eq!(
                bounded.expression(&expression, 0).unwrap_err().category,
                crate::FaultCategory::ResourceLimit
            );
            assert_eq!(bounded.nodes(), 24);
        }
    }
}
