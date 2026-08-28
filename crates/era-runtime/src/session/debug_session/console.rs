#[allow(clippy::wildcard_imports)]
use super::*;
use erabasic_compat::{
    CompatibilityIdentity, IntegerArithmeticPolicy, IntegerArithmeticWarning, IntegerOperation,
};

pub(super) fn console_diagnostic(code: &str, message: &str) -> DebugDiagnostic {
    DebugDiagnostic {
        code: code.into(),
        message: message.into(),
        source: None,
    }
}

#[cfg(test)]
pub(super) fn parse_console_expression(
    source: &str,
    variables: &[VmDebugVariable],
) -> Result<VmValue, (&'static str, String)> {
    parse_console_expression_with_compatibility(
        source,
        variables,
        &CompatibilityIdentity::reference(),
        &mut Vec::new(),
    )
}

pub(super) fn parse_console_expression_with_compatibility(
    source: &str,
    variables: &[VmDebugVariable],
    compatibility: &CompatibilityIdentity,
    diagnostics: &mut Vec<DebugDiagnostic>,
) -> Result<VmValue, (&'static str, String)> {
    let mut context = DefaultParserContext::default();
    for variable in variables {
        context.register_variable(&variable.name);
    }
    for function in PURE_CONSOLE_METHODS {
        context.register_function(function);
    }
    let parsed = parse_expression(source, &context);
    if parsed.has_errors() {
        let message = parsed
            .diagnostics
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        return Err(("debug.console.parse_error", message));
    }
    let expression = parsed.value.ok_or_else(|| {
        (
            "debug.console.parse_error",
            "expression parser produced no value".into(),
        )
    })?;
    evaluate_console_expression(&expression, variables, compatibility, diagnostics)
}

fn evaluate_console_expression(
    expression: &Expr,
    variables: &[VmDebugVariable],
    compatibility: &CompatibilityIdentity,
    diagnostics: &mut Vec<DebugDiagnostic>,
) -> Result<VmValue, (&'static str, String)> {
    match &expression.kind {
        ExprKind::Integer(value) => Ok(VmValue::Integer(*value)),
        ExprKind::String(value) => Ok(VmValue::String(value.clone())),
        ExprKind::Identifier(name) => console_variable(name, variables),
        ExprKind::Variable { name, indices } if indices.is_empty() => {
            console_variable(name, variables)
        }
        ExprKind::Variable { .. } => Err((
            "debug.console.unsupported_expression",
            "indexed variable reads are not in the safe console subset".into(),
        )),
        ExprKind::Group(inner) => {
            evaluate_console_expression(inner, variables, compatibility, diagnostics)
        }
        ExprKind::Unary { op, operand } => {
            let evaluated =
                evaluate_console_expression(operand, variables, compatibility, diagnostics)?;
            let value = console_integer(&evaluated)?;
            match op {
                UnaryOp::Plus => Ok(VmValue::Integer(value)),
                UnaryOp::Minus => evaluate_console_arithmetic(
                    IntegerOperation::Negate,
                    value,
                    None,
                    compatibility.integer_arithmetic_policy(),
                    diagnostics,
                ),
                UnaryOp::LogicalNot => Ok(VmValue::Integer(i64::from(value == 0))),
                UnaryOp::BitNot => Ok(VmValue::Integer(!value)),
                UnaryOp::PreIncrement | UnaryOp::PreDecrement => Err((
                    "debug.console.unsafe_expression",
                    "increment and decrement are not allowed in the transactional console".into(),
                )),
            }
        }
        ExprKind::Postfix { .. } => Err((
            "debug.console.unsafe_expression",
            "increment and decrement are not allowed in the transactional console".into(),
        )),
        ExprKind::Binary { op, left, right } => {
            let left = evaluate_console_expression(left, variables, compatibility, diagnostics)?;
            if let Some(value) =
                console_short_circuit(*op, &left, compatibility.integer_arithmetic_policy())?
            {
                return Ok(value);
            }
            let right = evaluate_console_expression(right, variables, compatibility, diagnostics)?;
            evaluate_console_binary(*op, &left, &right, compatibility, diagnostics)
        }
        ExprKind::Ternary {
            condition,
            then_expr,
            else_expr,
        } => {
            let evaluated =
                evaluate_console_expression(condition, variables, compatibility, diagnostics)?;
            let condition = console_integer(&evaluated)?;
            evaluate_console_expression(
                if condition != 0 { then_expr } else { else_expr },
                variables,
                compatibility,
                diagnostics,
            )
        }
        ExprKind::Call { name, args } => {
            evaluate_console_method(name, args, variables, compatibility, diagnostics)
        }
        ExprKind::Formatted(_) => Err((
            "debug.console.unsupported_expression",
            "formatted strings are not part of the safe console subset".into(),
        )),
        ExprKind::Error => Err(("debug.console.parse_error", "invalid expression".into())),
    }
}

fn console_short_circuit(
    op: BinaryOp,
    left: &VmValue,
    policy: IntegerArithmeticPolicy,
) -> Result<Option<VmValue>, (&'static str, String)> {
    // Retain the reference console's eager evaluation, but snake queries must
    // not execute or diagnose a branch skipped by the compiled expression.
    if policy == IntegerArithmeticPolicy::ReferenceWrappingV1
        || !matches!(
            op,
            BinaryOp::LogicalAnd | BinaryOp::LogicalOr | BinaryOp::Nand | BinaryOp::Nor
        )
    {
        return Ok(None);
    }
    let truthy = console_integer(left)? != 0;
    let value = match (op, truthy) {
        (BinaryOp::LogicalAnd, false) | (BinaryOp::Nor, true) => Some(0),
        (BinaryOp::LogicalOr, true) | (BinaryOp::Nand, false) => Some(1),
        _ => None,
    };
    Ok(value.map(VmValue::Integer))
}

fn console_variable(
    name: &str,
    variables: &[VmDebugVariable],
) -> Result<VmValue, (&'static str, String)> {
    variables
        .iter()
        .find(|item| item.name.eq_ignore_ascii_case(name))
        .map(|item| item.value.clone())
        .ok_or_else(|| {
            (
                "debug.console.unknown_variable",
                format!("{name} is not a visible scalar variable"),
            )
        })
}

fn evaluate_console_binary(
    op: BinaryOp,
    left: &VmValue,
    right: &VmValue,
    compatibility: &CompatibilityIdentity,
    diagnostics: &mut Vec<DebugDiagnostic>,
) -> Result<VmValue, (&'static str, String)> {
    if matches!(op, BinaryOp::Equal | BinaryOp::NotEqual) {
        let equal = left == right;
        return Ok(VmValue::Integer(i64::from(if op == BinaryOp::Equal {
            equal
        } else {
            !equal
        })));
    }
    let left = console_integer(left)?;
    let right = console_integer(right)?;
    let arithmetic = match op {
        BinaryOp::Multiply => Some(IntegerOperation::Multiply),
        BinaryOp::Divide => Some(IntegerOperation::Divide),
        BinaryOp::Modulo => Some(IntegerOperation::Modulo),
        BinaryOp::Add => Some(IntegerOperation::Add),
        BinaryOp::Subtract => Some(IntegerOperation::Subtract),
        _ => None,
    };
    if let Some(operation) = arithmetic {
        return evaluate_console_arithmetic(
            operation,
            left,
            Some(right),
            compatibility.integer_arithmetic_policy(),
            diagnostics,
        );
    }
    // EraBasic follows the CLR's masked 64-bit shift-count behavior.
    let shift = u32::try_from(right & 63).expect("masked shift count fits u32");
    let value = match op {
        BinaryOp::ShiftLeft => left.wrapping_shl(shift),
        BinaryOp::ShiftRight => left.wrapping_shr(shift),
        BinaryOp::Less => i64::from(left < right),
        BinaryOp::LessEqual => i64::from(left <= right),
        BinaryOp::Greater => i64::from(left > right),
        BinaryOp::GreaterEqual => i64::from(left >= right),
        BinaryOp::BitAnd => left & right,
        BinaryOp::BitXor => left ^ right,
        BinaryOp::BitOr => left | right,
        BinaryOp::LogicalAnd => i64::from(left != 0 && right != 0),
        BinaryOp::LogicalXor => i64::from((left != 0) ^ (right != 0)),
        BinaryOp::LogicalOr => i64::from(left != 0 || right != 0),
        BinaryOp::Nand => i64::from(!(left != 0 && right != 0)),
        BinaryOp::Nor => i64::from(!(left != 0 || right != 0)),
        BinaryOp::Equal
        | BinaryOp::NotEqual
        | BinaryOp::Multiply
        | BinaryOp::Divide
        | BinaryOp::Modulo
        | BinaryOp::Add
        | BinaryOp::Subtract => unreachable!("handled above"),
    };
    Ok(VmValue::Integer(value))
}

fn evaluate_console_arithmetic(
    operation: IntegerOperation,
    left: i64,
    right: Option<i64>,
    policy: IntegerArithmeticPolicy,
    diagnostics: &mut Vec<DebugDiagnostic>,
) -> Result<VmValue, (&'static str, String)> {
    // The reference console historically wraps even MIN / -1 and MIN % -1.
    // Preserve that debugger behavior while snake uses the VM's exact policy.
    if policy == IntegerArithmeticPolicy::ReferenceWrappingV1
        && let Some(right) = right
    {
        match operation {
            IntegerOperation::Divide if right != 0 => {
                return Ok(VmValue::Integer(left.wrapping_div(right)));
            }
            IntegerOperation::Modulo if right != 0 => {
                return Ok(VmValue::Integer(left.wrapping_rem(right)));
            }
            IntegerOperation::Divide | IntegerOperation::Modulo => {
                return Err(("debug.console.execution_error", "division by zero".into()));
            }
            _ => {}
        }
    }
    let outcome = policy
        .evaluate(operation, left, right)
        .map_err(|error| ("debug.console.execution_error", error.to_string()))?;
    if let Some(warning) = outcome.warning {
        let (code, message) = match warning {
            IntegerArithmeticWarning::Overflow => (
                "compat.arithmetic.overflow",
                "integer arithmetic overflowed; snake saturation policy applied",
            ),
            IntegerArithmeticWarning::DivideByZero => (
                "compat.arithmetic.divide_by_zero",
                "integer division or remainder by zero returned zero under snake policy",
            ),
        };
        // Console diagnostics belong to this request, not the VM's warning history.
        diagnostics.push(console_diagnostic(code, message));
    }
    Ok(VmValue::Integer(outcome.value))
}

fn evaluate_console_method(
    name: &str,
    arguments: &[Option<Expr>],
    variables: &[VmDebugVariable],
    compatibility: &CompatibilityIdentity,
    diagnostics: &mut Vec<DebugDiagnostic>,
) -> Result<VmValue, (&'static str, String)> {
    let values = arguments
        .iter()
        .map(|argument| {
            argument
                .as_ref()
                .ok_or_else(|| {
                    (
                        "debug.console.unsupported_expression",
                        "omitted method arguments are not supported".into(),
                    )
                })
                .and_then(|argument| {
                    evaluate_console_expression(argument, variables, compatibility, diagnostics)
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let upper = name.to_ascii_uppercase();
    if !PURE_CONSOLE_METHODS.contains(&upper.as_str()) {
        return Err((
            "debug.console.unsafe_method",
            format!("{name} is not in the debugger's pure method whitelist"),
        ));
    }
    evaluate_pure_native_with_compatibility(&upper, values, compatibility)
        .map_err(|message| ("debug.console.execution_error", message))
}

const PURE_CONSOLE_METHODS: [&str; 35] = [
    "ABS",
    "SIGN",
    "SQRT",
    "CBRT",
    "LOG",
    "LOG10",
    "EXPONENT",
    "POWER",
    "GETBIT",
    "BITCOUNT",
    "STRLEN",
    "STRLENU",
    "TOINT",
    "ISNUMERIC",
    "UNICODE",
    "CONVERT",
    "COLOR_FROMRGB",
    "MAX",
    "MIN",
    "LIMIT",
    "INRANGE",
    "TOSTR",
    "SUBSTRING",
    "SUBSTRINGU",
    "STRFIND",
    "STRFINDU",
    "STRCOUNT",
    "STRLENS",
    "STRLENSU",
    "REPLACE",
    "ESCAPE",
    "UNICODETOSTR",
    "ENCODETOUNI",
    "UNICODEBYTE",
    "CHARATU",
];

pub(super) fn console_integer(value: &VmValue) -> Result<i64, (&'static str, String)> {
    match value {
        VmValue::Integer(value) => Ok(*value),
        _ => console_type_error("integer"),
    }
}

pub(super) fn console_type_error<T>(expected: &str) -> Result<T, (&'static str, String)> {
    Err((
        "debug.console.type_mismatch",
        format!("safe expression expected an {expected} value"),
    ))
}

pub(super) fn all_debug_scopes() -> [DebugScope; 10] {
    [
        DebugScope::VariablesRead,
        DebugScope::VariablesWrite,
        DebugScope::GameFieldsRead,
        DebugScope::GameFieldsWrite,
        DebugScope::ExecutionRead,
        DebugScope::ExecutionControl,
        DebugScope::ConsoleEvaluate,
        DebugScope::ConsoleExecute,
        DebugScope::BreakpointsManage,
        DebugScope::ScriptOutput,
    ]
}

pub(super) fn scope_bit(scope: DebugScope) -> u64 {
    1_u64
        << match scope {
            DebugScope::VariablesRead => 0,
            DebugScope::VariablesWrite => 1,
            DebugScope::GameFieldsRead => 2,
            DebugScope::GameFieldsWrite => 3,
            DebugScope::ExecutionRead => 4,
            DebugScope::ExecutionControl => 5,
            DebugScope::ConsoleEvaluate => 6,
            DebugScope::ConsoleExecute => 7,
            DebugScope::BreakpointsManage => 8,
            DebugScope::ScriptOutput => 9,
        }
}

pub(super) fn command_scope(command: &DebugCommand) -> DebugScope {
    match command {
        DebugCommand::Pause | DebugCommand::Continue { .. } | DebugCommand::Step { .. } => {
            DebugScope::ExecutionControl
        }
        DebugCommand::ListVariables { .. } | DebugCommand::ReadVariable { .. } => {
            DebugScope::VariablesRead
        }
        DebugCommand::WriteVariables { .. } => DebugScope::VariablesWrite,
        DebugCommand::ListGameFields { .. } | DebugCommand::ReadGameField { .. } => {
            DebugScope::GameFieldsRead
        }
        DebugCommand::WriteGameFields { .. } => DebugScope::GameFieldsWrite,
        DebugCommand::ListFibers { .. }
        | DebugCommand::ReadCallStack { .. }
        | DebugCommand::ReadOperandStack { .. } => DebugScope::ExecutionRead,
        DebugCommand::Console {
            command: ConsoleCommand::Evaluate { .. },
            ..
        } => DebugScope::ConsoleEvaluate,
        DebugCommand::Console {
            command: ConsoleCommand::ExecuteSafe { .. },
            ..
        } => DebugScope::ConsoleExecute,
        DebugCommand::UpdateBreakpoints { .. } => DebugScope::BreakpointsManage,
        DebugCommand::ReadScriptOutput { .. } | DebugCommand::SubscribeScriptOutput { .. } => {
            DebugScope::ScriptOutput
        }
    }
}

pub(super) fn next_char_boundary(value: &str, mut index: usize) -> usize {
    while index < value.len() && !value.is_char_boundary(index) {
        index += 1;
    }
    index
}

pub(super) fn previous_char_boundary(value: &str, mut index: usize) -> usize {
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}
