#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod console_tests {
    use super::*;
    use erabasic_compat::{CompatibilityIdentity, CompatibilityProfileId};

    #[test]
    fn safe_console_uses_erabasic_precedence_and_pure_methods() {
        assert_eq!(
            parse_console_expression("1 + 2 * 3", &[]),
            Ok(VmValue::Integer(7))
        );
        assert_eq!(
            parse_console_expression("ABS(-4) + MAX(2, 5)", &[]),
            Ok(VmValue::Integer(9))
        );
        assert_eq!(
            parse_console_expression("STRLENS(\"界\")", &[]),
            Ok(VmValue::Integer(2))
        );
        assert_eq!(
            parse_console_expression("STRLENSU(\"😀\")", &[]),
            Ok(VmValue::Integer(2))
        );
    }

    #[test]
    fn safe_console_rejects_failed_or_non_whitelisted_work_before_commit() {
        assert!(matches!(
            parse_console_expression("1 / 0", &[]),
            Err(("debug.console.execution_error", _))
        ));
        assert!(parse_console_expression("GETKEY(1)", &[]).is_err());
    }

    #[test]
    fn safe_console_arithmetic_uses_the_profile_and_request_local_warnings() {
        let snake = CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake);
        for (source, reference, expected, code) in [
            (
                "9223372036854775807 + 1",
                Some(i64::MIN),
                i64::MAX,
                "overflow",
            ),
            (
                "0 - TOINT(\"-9223372036854775808\")",
                Some(i64::MIN),
                i64::MIN,
                "overflow",
            ),
            ("9223372036854775807 * 2", Some(-2), i64::MAX, "overflow"),
            (
                "-TOINT(\"-9223372036854775808\")",
                Some(i64::MIN),
                i64::MAX,
                "overflow",
            ),
            ("8 / 0", None, 0, "divide_by_zero"),
            ("8 % 0", None, 0, "divide_by_zero"),
        ] {
            let result = parse_console_expression(source, &[]);
            if let Some(expected) = reference {
                assert_eq!(result, Ok(VmValue::Integer(expected)), "{source}");
            } else {
                assert!(result.is_err(), "{source}");
            }
            // Repeated queries must not consume the live VM's warning allowance.
            for _ in 0..2 {
                let mut diagnostics = Vec::new();
                assert_eq!(
                    parse_console_expression_with_compatibility(
                        source,
                        &[],
                        &snake,
                        &mut diagnostics
                    ),
                    Ok(VmValue::Integer(expected)),
                    "{source}",
                );
                assert_eq!(diagnostics.len(), 1, "{source}");
                assert_eq!(diagnostics[0].code, format!("compat.arithmetic.{code}"));
            }
        }
        for (operator, reference) in [("/", i64::MIN), ("%", 0)] {
            let source = format!("TOINT(\"-9223372036854775808\") {operator} -1");
            assert_eq!(
                parse_console_expression(&source, &[]),
                Ok(VmValue::Integer(reference))
            );
            let mut diagnostics = Vec::new();
            assert!(
                parse_console_expression_with_compatibility(&source, &[], &snake, &mut diagnostics)
                    .is_err()
            );
            assert!(diagnostics.is_empty());
        }
    }

    #[test]
    fn safe_console_native_policy_keeps_unchecked_wrapping_and_the_pure_boundary() {
        let snake = CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake);
        for (source, expected) in [
            ("TOINT(\"9223372036854775808\")", 0),
            ("UNCHECKED_ADD(9223372036854775807, 1)", i64::MIN),
            (
                "UNCHECKED_SUB(TOINT(\"-9223372036854775808\"), 1)",
                i64::MAX,
            ),
            ("UNCHECKED_MUL(9223372036854775807, 2)", -2),
            ("UNCHECKED_NEG(TOINT(\"-9223372036854775808\"))", i64::MIN),
        ] {
            let mut diagnostics = Vec::new();
            assert_eq!(
                parse_console_expression_with_compatibility(source, &[], &snake, &mut diagnostics),
                Ok(VmValue::Integer(expected)),
                "{source}",
            );
            assert!(diagnostics.is_empty(), "{source}");
        }
        assert!(parse_console_expression("TOINT(\"9223372036854775808\")", &[]).is_err());
        for source in [
            "RAND(2)",
            "GETKEY(1)",
            "TOINT(1)",
            "UNCHECKED_ADD(1, 2, 3)",
            "UNCHECKED_NEG(1, 2)",
        ] {
            assert!(
                parse_console_expression_with_compatibility(source, &[], &snake, &mut Vec::new())
                    .is_err()
            );
            assert!(parse_console_expression(source, &[]).is_err());
        }
        assert_eq!(
            parse_console_expression("UNCHECKED_ADD(9223372036854775807, 1)", &[]),
            Ok(VmValue::Integer(i64::MIN)),
        );
        assert_eq!(
            parse_console_expression_with_compatibility(
                "ISNUMERIC(\"9223372036854775808\")",
                &[],
                &snake,
                &mut Vec::new()
            ),
            parse_console_expression("ISNUMERIC(\"9223372036854775808\")", &[]),
        );
    }

    #[test]
    fn safe_console_keeps_ternary_evaluation_lazy() {
        for compatibility in [
            CompatibilityIdentity::reference(),
            CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake),
        ] {
            for source in ["1 ? 7 # (1 / 0)", "0 ? (1 / 0) # 7"] {
                let mut diagnostics = Vec::new();
                assert_eq!(
                    parse_console_expression_with_compatibility(
                        source,
                        &[],
                        &compatibility,
                        &mut diagnostics
                    ),
                    Ok(VmValue::Integer(7)),
                );
                assert!(diagnostics.is_empty());
            }
        }
    }

    #[test]
    fn safe_console_snake_logic_skips_unexecuted_errors_and_warnings() {
        let snake = CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake);
        for (source, expected) in [
            ("0 && (1 / 0)", 0),
            ("1 || (1 / 0)", 1),
            ("0 !& (1 / 0)", 1),
            ("1 !| (1 / 0)", 0),
        ] {
            let mut diagnostics = Vec::new();
            assert_eq!(
                parse_console_expression_with_compatibility(source, &[], &snake, &mut diagnostics),
                Ok(VmValue::Integer(expected)),
            );
            assert!(diagnostics.is_empty());
            assert!(parse_console_expression(source, &[]).is_err());
        }
        for source in [
            "1 && (1 / 0)",
            "0 || (1 / 0)",
            "1 !& (1 / 0)",
            "0 !| (1 / 0)",
        ] {
            let mut diagnostics = Vec::new();
            assert!(
                parse_console_expression_with_compatibility(source, &[], &snake, &mut diagnostics)
                    .is_ok()
            );
            assert_eq!(diagnostics.len(), 1);
            assert_eq!(diagnostics[0].code, "compat.arithmetic.divide_by_zero");
        }
    }
}
