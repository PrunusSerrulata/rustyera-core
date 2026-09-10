use super::*;

#[test]
fn non_arithmetic_dispatch_preserves_profile_independent_values_and_errors() {
    use erabasic_compat::{CompatibilityIdentity, CompatibilityProfileId};
    let source = "@SYSTEM_TITLE\nRETURN\n";
    for profile in [
        CompatibilityProfileId::EmueraEm,
        CompatibilityProfileId::EmueraSkiaSnake,
    ] {
        let mut options = erabasic_analyzer::AnalyzerOptions::analysis_mode();
        options.compatibility = CompatibilityIdentity::for_profile(profile);
        let artifact = super::super::literal_groupmatch_tests::compile_cursor_fixture_with_options(
            source.into(),
            &options,
        );
        let validated = erabasic_validator::validate_bytecode(
            artifact.clone().into_unvalidated(),
            &erabasic_compiler::runtime_native_validation_context(
                &artifact,
                &erabasic_compiler::default_host_registry(),
            ),
        );
        assert!(
            validated.diagnostics.is_empty(),
            "{:?}",
            validated.diagnostics
        );
        let mut vm = Vm::new(validated.value.unwrap(), crate::VmConfig::default());
        let generation = vm.current_generation;
        let values = [
            VmValue::Integer(i64::MIN),
            VmValue::Integer(0),
            VmValue::Integer(63),
            VmValue::String(String::new()),
            VmValue::String("玄関🙂".into()),
            VmValue::IntegerPlace(Box::default()),
            VmValue::StringPlace(Box::default()),
        ];
        for operation in [0, 2, 3, 8, 255] {
            for value in &values {
                assert_eq!(
                    vm.unary_value(generation, operation, value.clone()),
                    operand::unary_value(operation, value.clone()),
                );
            }
        }
        for operation in (5..=21).chain([255]) {
            for left in &values {
                for right in &values {
                    assert_eq!(
                        vm.binary_value(generation, operation, left.clone(), right.clone()),
                        operand::binary_value(operation, left.clone(), right.clone()),
                    );
                }
            }
        }
        assert!(vm.pending_compatibility_warnings.is_empty());
    }
}
