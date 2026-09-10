use super::*;

#[test]
fn borrowed_signature_arguments_preserve_defaults_and_rejections() {
    use erabasic_compat::CompatibilityProfileId;

    for profile in [
        CompatibilityProfileId::EmueraEm,
        CompatibilityProfileId::EmueraSkiaSnake,
    ] {
        let vm = signature_fixture(profile);
        let program = &vm.generations[&vm.current_generation];
        let arguments = [UserArgumentSpec::Value(BytecodeType::Integer)];
        let call = resolve_user_call_arguments(
            program,
            vm.current_generation,
            "TARGET",
            UserCallMode::Procedure,
            false,
            &arguments,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            call.bindings,
            vec![
                UserArgumentBinding::Value {
                    convert_integer_to_string: false
                },
                UserArgumentBinding::Default(BytecodeConstant::String("fallback".into()))
            ]
        );
        for (name, mode, allow_missing, arguments, expected) in [
            (
                "TARGET",
                UserCallMode::Procedure,
                false,
                arguments.to_vec(),
                Ok(Some(call)),
            ),
            (
                "TARGET",
                UserCallMode::Procedure,
                false,
                vec![UserArgumentSpec::Value(BytecodeType::String)],
                Err(expected_argument_failure(
                    "method TARGET argument 1 has an incompatible value type",
                )),
            ),
            (
                "TARGET",
                UserCallMode::MethodDiscard,
                false,
                vec![],
                Err(expected_argument_failure(
                    "dynamic target TARGET has an incompatible kind",
                )),
            ),
            (
                "TARGET",
                UserCallMode::MethodDiscard,
                true,
                vec![],
                Ok(None),
            ),
            ("ABSENT", UserCallMode::Procedure, false, vec![], Ok(None)),
        ] {
            let spec = UserCallSpec {
                mode,
                allow_missing,
                missing_target: 0,
                arguments,
            };
            assert_eq!(
                resolve_user_call(program, vm.current_generation, name, &spec),
                expected
            );
            assert_eq!(
                resolve_user_call_arguments(
                    program,
                    vm.current_generation,
                    name,
                    mode,
                    allow_missing,
                    &spec.arguments
                ),
                expected
            );
        }
    }
}

fn expected_argument_failure(message: &str) -> VmError {
    VmError::ScriptFailure(crate::ExecutionFailure::script(
        crate::ScriptFaultKind::Argument,
        crate::VmFaultCode::TypeMismatch,
        message,
    ))
}

fn signature_fixture(profile: erabasic_compat::CompatibilityProfileId) -> Vm {
    use crate::interpreter::literal_groupmatch_tests::compile_cursor_fixture_with_options;
    use erabasic_compiler::{default_host_registry, runtime_native_validation_context};
    let mut options = erabasic_analyzer::AnalyzerOptions::analysis_mode();
    options.compatibility = erabasic_compat::CompatibilityIdentity::for_profile(profile);
    let source = "@SYSTEM_TITLE\nRETURN\n@TARGET(ARG, ARGS = \"fallback\")\nRETURN\n";
    let artifact = compile_cursor_fixture_with_options(source.into(), &options);
    let context = runtime_native_validation_context(&artifact, &default_host_registry());
    let validation = erabasic_validator::validate_bytecode(artifact.into_unvalidated(), &context);
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    Vm::new(validation.value.unwrap(), crate::VmConfig::default())
}
