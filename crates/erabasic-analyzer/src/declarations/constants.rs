use super::{
    AnalyzerOptions, BTreeMap, BinaryOp, ConstantValue, DimError, Expr, ExprKind, FormPart,
    FormattedString, IndexResolver, ParserContext, UnaryOp, normalize, parse_expression,
};
use std::cell::RefCell;

use erabasic_compat::{IntegerArithmeticPolicy, IntegerArithmeticWarning, IntegerOperation};

pub(crate) type ConstantWarnings = Vec<(IntegerArithmeticWarning, String)>;

pub(super) struct ConstantEvaluation<'a> {
    pub(super) constants: &'a BTreeMap<String, ConstantValue>,
    pub(super) variable_dimensions: &'a BTreeMap<String, Vec<usize>>,
    pub(super) index_resolver: &'a IndexResolver,
    pub(super) options: &'a AnalyzerOptions,
    pub(super) warnings: RefCell<ConstantWarnings>,
}

impl ConstantEvaluation<'_> {
    fn integer(
        &self,
        operation: IntegerOperation,
        left: i64,
        right: Option<i64>,
    ) -> Result<i64, DimError> {
        let result = self
            .options
            .compatibility
            .integer_arithmetic_policy()
            .evaluate(operation, left, right)
            .map_err(|error| DimError::Invalid(error.to_string()))?;
        if let Some(warning) = result.warning {
            self.warnings.borrow_mut().push((warning, format!(
                "constant integer {operation:?} produced {warning:?}: {left}, {right:?}; result {}",
                result.value
            )));
        }
        Ok(result.value)
    }
}

pub(super) fn parse_constant(
    source: &str,
    context: &dyn ParserContext,
    evaluation: &ConstantEvaluation<'_>,
) -> Result<ConstantValue, DimError> {
    let output = parse_expression(source.trim(), context);
    if output.has_errors() {
        return Err(DimError::Invalid(format!(
            "invalid constant expression {source:?}"
        )));
    }
    let expression = output
        .value
        .ok_or_else(|| DimError::Invalid("constant expression is empty".into()))?;
    evaluate_constant(&expression, evaluation)
}

fn evaluate_constant(
    expression: &Expr,
    evaluation: &ConstantEvaluation<'_>,
) -> Result<ConstantValue, DimError> {
    match &expression.kind {
        ExprKind::Integer(value) => Ok(ConstantValue::Integer(*value)),
        ExprKind::String(value) => Ok(ConstantValue::String(value.clone())),
        ExprKind::Identifier(name) => evaluation
            .constants
            .get(&normalize(name, evaluation.options.ignore_case))
            .cloned()
            .ok_or_else(|| DimError::UnknownConstant(name.clone())),
        ExprKind::Group(inner) => evaluate_constant(inner, evaluation),
        ExprKind::Unary { op, operand } => {
            let ConstantValue::Integer(value) = evaluate_constant(operand, evaluation)? else {
                return Err(DimError::Invalid("integer unary operand required".into()));
            };
            let value = match op {
                UnaryOp::Plus => value,
                UnaryOp::Minus => evaluation.integer(IntegerOperation::Negate, value, None)?,
                UnaryOp::LogicalNot => i64::from(value == 0),
                UnaryOp::BitNot => !value,
                UnaryOp::PreIncrement | UnaryOp::PreDecrement => {
                    return Err(DimError::Invalid(
                        "increment is not a constant expression".into(),
                    ));
                }
            };
            Ok(ConstantValue::Integer(value))
        }
        ExprKind::Binary { op, left, right } => {
            evaluate_binary_expression(*op, left, right, evaluation)
        }
        ExprKind::Ternary {
            condition,
            then_expr,
            else_expr,
        } => {
            let ConstantValue::Integer(condition) = evaluate_constant(condition, evaluation)?
            else {
                return Err(DimError::Invalid("integer condition required".into()));
            };
            evaluate_constant(
                if condition != 0 { then_expr } else { else_expr },
                evaluation,
            )
        }
        ExprKind::Call { name, args } if name.eq_ignore_ascii_case("VARSIZE") => {
            evaluate_varsize(args, evaluation)
        }
        ExprKind::Call { name, args } if name.eq_ignore_ascii_case("GETNUM") => {
            evaluate_getnum(args, evaluation)
        }
        ExprKind::Call { name, args }
            if matches!(
                name.to_ascii_uppercase().as_str(),
                "UNCHECKED_ADD" | "UNCHECKED_SUB" | "UNCHECKED_MUL" | "UNCHECKED_NEG"
            ) =>
        {
            evaluate_unchecked(name, args, evaluation)
        }
        ExprKind::Call { name, args }
            if name.eq_ignore_ascii_case("GETDEFCOLOR") && args.is_empty() =>
        {
            Ok(ConstantValue::Integer(
                evaluation.options.default_foreground_color,
            ))
        }
        ExprKind::Call { name, args }
            if matches!(name.to_ascii_uppercase().as_str(), "STRLENS" | "STRLENSU")
                && args.len() == 1 =>
        {
            evaluate_string_length(name, args, evaluation)
        }
        ExprKind::Call { name, args }
            if name.eq_ignore_ascii_case("UNICODE") && args.len() == 1 =>
        {
            let argument = args[0]
                .as_ref()
                .ok_or_else(|| DimError::Invalid("UNICODE requires an argument".into()))?;
            let ConstantValue::Integer(value) = evaluate_constant(argument, evaluation)? else {
                return Err(DimError::Invalid(
                    "UNICODE requires an integer argument".into(),
                ));
            };
            let value = u32::try_from(value)
                .ok()
                .and_then(char::from_u32)
                .ok_or_else(|| DimError::Invalid("UNICODE argument is out of range".into()))?;
            Ok(ConstantValue::String(value.to_string()))
        }
        ExprKind::Formatted(formatted) => evaluate_formatted(formatted, evaluation),
        _ => Err(DimError::Invalid(
            "initializer must be a load-time constant".into(),
        )),
    }
}

fn evaluate_binary_expression(
    operation: BinaryOp,
    left: &Expr,
    right: &Expr,
    evaluation: &ConstantEvaluation<'_>,
) -> Result<ConstantValue, DimError> {
    let left = evaluate_constant(left, evaluation)?;
    if evaluation.options.compatibility.integer_arithmetic_policy()
        == IntegerArithmeticPolicy::SnakeSaturatingV1
        && let ConstantValue::Integer(value) = &left
    {
        // Do not emit warnings or faults from a branch that the VM skips.
        let short_circuit = match operation {
            BinaryOp::LogicalAnd if *value == 0 => Some(0),
            BinaryOp::LogicalOr if *value != 0 => Some(1),
            BinaryOp::Nand if *value == 0 => Some(1),
            BinaryOp::Nor if *value != 0 => Some(0),
            _ => None,
        };
        if let Some(value) = short_circuit {
            return Ok(ConstantValue::Integer(value));
        }
    }
    let right = evaluate_constant(right, evaluation)?;
    evaluate_binary(operation, left, right, evaluation)
}

fn evaluate_unchecked(
    name: &str,
    arguments: &[Option<Expr>],
    evaluation: &ConstantEvaluation<'_>,
) -> Result<ConstantValue, DimError> {
    let operation = match name.to_ascii_uppercase().as_str() {
        "UNCHECKED_ADD" => IntegerOperation::Add,
        "UNCHECKED_SUB" => IntegerOperation::Subtract,
        "UNCHECKED_MUL" => IntegerOperation::Multiply,
        _ => IntegerOperation::Negate,
    };
    let arity = if operation == IntegerOperation::Negate {
        1
    } else {
        2
    };
    if arguments.len() != arity {
        return Err(DimError::Invalid(format!(
            "{name} requires {arity} arguments"
        )));
    }
    let integer = |index: usize| -> Result<i64, DimError> {
        let expression = arguments[index]
            .as_ref()
            .ok_or_else(|| DimError::Invalid(format!("{name} arguments cannot be omitted")))?;
        let ConstantValue::Integer(value) = evaluate_constant(expression, evaluation)? else {
            return Err(DimError::Invalid(format!(
                "{name} requires integer arguments"
            )));
        };
        Ok(value)
    };
    let left = integer(0)?;
    let right = if arity == 2 { Some(integer(1)?) } else { None };
    let result = IntegerArithmeticPolicy::ReferenceWrappingV1
        .evaluate(operation, left, right)
        .map_err(|error| DimError::Invalid(error.to_string()))?;
    Ok(ConstantValue::Integer(result.value))
}

fn evaluate_string_length(
    name: &str,
    arguments: &[Option<Expr>],
    evaluation: &ConstantEvaluation<'_>,
) -> Result<ConstantValue, DimError> {
    let name = name.to_ascii_uppercase();
    let argument = arguments[0]
        .as_ref()
        .ok_or_else(|| DimError::Invalid(format!("{name} requires an argument")))?;
    let ConstantValue::String(value) = evaluate_constant(argument, evaluation)? else {
        return Err(DimError::Invalid(format!(
            "{name} requires a constant string argument"
        )));
    };
    let length = if name == "STRLENS" {
        evaluation.index_resolver.legacy_encoded_len(&value)
    } else {
        value.encode_utf16().count()
    };
    Ok(ConstantValue::Integer(
        i64::try_from(length).unwrap_or(i64::MAX),
    ))
}

fn evaluate_varsize(
    arguments: &[Option<Expr>],
    evaluation: &ConstantEvaluation<'_>,
) -> Result<ConstantValue, DimError> {
    if !(1..=2).contains(&arguments.len()) {
        return Err(DimError::Invalid(
            "VARSIZE requires one or two arguments".into(),
        ));
    }
    let variable_argument = arguments[0]
        .as_ref()
        .ok_or_else(|| DimError::Invalid("VARSIZE variable name cannot be omitted".into()))?;
    let ConstantValue::String(variable_name) = evaluate_constant(variable_argument, evaluation)?
    else {
        return Err(DimError::Invalid(
            "VARSIZE variable name must be a constant string".into(),
        ));
    };
    let dimensions = evaluation
        .variable_dimensions
        .get(&normalize(&variable_name, evaluation.options.ignore_case))
        .ok_or_else(|| DimError::UnknownConstant(variable_name.clone()))?;
    let mut dimension = if let Some(argument) = arguments.get(1) {
        let argument = argument
            .as_ref()
            .ok_or_else(|| DimError::Invalid("VARSIZE dimension cannot be omitted".into()))?;
        let ConstantValue::Integer(value) = evaluate_constant(argument, evaluation)? else {
            return Err(DimError::Invalid(
                "VARSIZE dimension must be a constant integer".into(),
            ));
        };
        value
    } else {
        0
    };
    if evaluation.options.varsize_dimension_is_one_based && dimension > 0 {
        dimension -= 1;
    }
    let dimension = usize::try_from(dimension)
        .map_err(|_| DimError::Invalid("VARSIZE dimension must be non-negative".into()))?;
    let length = dimensions
        .get(dimension)
        .copied()
        .ok_or_else(|| DimError::Invalid("VARSIZE dimension exceeds the variable rank".into()))?;
    Ok(ConstantValue::Integer(
        i64::try_from(length).unwrap_or(i64::MAX),
    ))
}

fn evaluate_getnum(
    arguments: &[Option<Expr>],
    evaluation: &ConstantEvaluation<'_>,
) -> Result<ConstantValue, DimError> {
    if !(2..=3).contains(&arguments.len()) {
        return Err(DimError::Invalid(
            "GETNUM requires two or three arguments".into(),
        ));
    }
    let variable = arguments[0]
        .as_ref()
        .and_then(constant_variable_name)
        .ok_or_else(|| DimError::Invalid("GETNUM argument 1 must be a variable name".into()))?;
    if !evaluation
        .variable_dimensions
        .contains_key(&normalize(variable, evaluation.options.ignore_case))
    {
        return Err(DimError::UnknownConstant(variable.into()));
    }
    let key_argument = arguments[1]
        .as_ref()
        .ok_or_else(|| DimError::Invalid("GETNUM key cannot be omitted".into()))?;
    let ConstantValue::String(key) = evaluate_constant(key_argument, evaluation)? else {
        return Err(DimError::Invalid(
            "GETNUM key must be a constant string".into(),
        ));
    };
    let dimension = if let Some(argument) = arguments.get(2) {
        let argument = argument
            .as_ref()
            .ok_or_else(|| DimError::Invalid("GETNUM dimension cannot be omitted".into()))?;
        let ConstantValue::Integer(value) = evaluate_constant(argument, evaluation)? else {
            return Err(DimError::Invalid(
                "GETNUM dimension must be a constant integer".into(),
            ));
        };
        let value = if value > 0 { value - 1 } else { value };
        usize::try_from(value)
            .map_err(|_| DimError::Invalid("GETNUM dimension must be non-negative".into()))?
    } else {
        0
    };
    Ok(ConstantValue::Integer(
        evaluation
            .index_resolver
            .resolve(variable, dimension, &key)
            .unwrap_or(-1),
    ))
}

fn constant_variable_name(expression: &Expr) -> Option<&str> {
    match &expression.kind {
        ExprKind::Identifier(name) => Some(name),
        ExprKind::Variable { name, indices } if indices.is_empty() => Some(name),
        ExprKind::Group(inner) => constant_variable_name(inner),
        _ => None,
    }
}

fn evaluate_formatted(
    formatted: &FormattedString,
    evaluation: &ConstantEvaluation<'_>,
) -> Result<ConstantValue, DimError> {
    let mut result = String::new();
    for part in &formatted.parts {
        match part {
            FormPart::Text(value) => result.push_str(value),
            FormPart::StringInterpolation { expression, .. } => {
                match evaluate_constant(expression, evaluation)? {
                    ConstantValue::String(value) => result.push_str(&value),
                    ConstantValue::Integer(value) => result.push_str(&value.to_string()),
                }
            }
            FormPart::IntegerInterpolation { expression, .. } => {
                let ConstantValue::Integer(value) = evaluate_constant(expression, evaluation)?
                else {
                    return Err(DimError::Invalid(
                        "integer interpolation requires an integer".into(),
                    ));
                };
                result.push_str(&value.to_string());
            }
            FormPart::Conditional {
                condition,
                then_value,
                else_value,
                ..
            } => {
                let ConstantValue::Integer(condition) = evaluate_constant(condition, evaluation)?
                else {
                    return Err(DimError::Invalid(
                        "formatted condition requires an integer".into(),
                    ));
                };
                let selected = if condition != 0 {
                    Some(then_value.as_ref())
                } else {
                    else_value.as_deref()
                };
                if let Some(selected) = selected {
                    let ConstantValue::String(value) = evaluate_formatted(selected, evaluation)?
                    else {
                        unreachable!("formatted evaluation always returns a string");
                    };
                    result.push_str(&value);
                }
            }
            FormPart::Triple { .. } => {
                return Err(DimError::Invalid(
                    "triple interpolation is not a load-time constant".into(),
                ));
            }
        }
    }
    Ok(ConstantValue::String(result))
}

#[allow(clippy::too_many_lines)]
fn evaluate_binary(
    op: BinaryOp,
    left: ConstantValue,
    right: ConstantValue,
    evaluation: &ConstantEvaluation<'_>,
) -> Result<ConstantValue, DimError> {
    if let (ConstantValue::String(left), ConstantValue::String(right)) = (&left, &right) {
        return match op {
            BinaryOp::Add => Ok(ConstantValue::String(format!("{left}{right}"))),
            BinaryOp::Equal => Ok(ConstantValue::Integer(i64::from(left == right))),
            BinaryOp::NotEqual => Ok(ConstantValue::Integer(i64::from(left != right))),
            BinaryOp::Less => Ok(ConstantValue::Integer(i64::from(left < right))),
            BinaryOp::LessEqual => Ok(ConstantValue::Integer(i64::from(left <= right))),
            BinaryOp::Greater => Ok(ConstantValue::Integer(i64::from(left > right))),
            BinaryOp::GreaterEqual => Ok(ConstantValue::Integer(i64::from(left >= right))),
            _ => Err(DimError::Invalid("invalid string constant operator".into())),
        };
    }
    if op == BinaryOp::Multiply {
        let repeated = match (&left, &right) {
            (ConstantValue::String(value), ConstantValue::Integer(count))
            | (ConstantValue::Integer(count), ConstantValue::String(value)) => {
                Some((value, *count))
            }
            _ => None,
        };
        if let Some((value, count)) = repeated {
            let count = usize::try_from(count)
                .ok()
                .filter(|count| *count < 10_000)
                .ok_or_else(|| {
                    DimError::Invalid("string repeat count must be between 0 and 9999".into())
                })?;
            return Ok(ConstantValue::String(value.repeat(count)));
        }
    }
    let (ConstantValue::Integer(left), ConstantValue::Integer(right)) = (left, right) else {
        return Err(DimError::Invalid("constant operand types differ".into()));
    };
    if evaluation.options.compatibility.integer_arithmetic_policy()
        == IntegerArithmeticPolicy::SnakeSaturatingV1
        && let Some(operation) = crate::integer::binary_operation(op)
    {
        return evaluation
            .integer(operation, left, Some(right))
            .map(ConstantValue::Integer);
    }
    // Preserve the established reference load-time overflow behavior separately
    // from the VM's checked division/remainder fault path.
    let value = match op {
        BinaryOp::Multiply => left.wrapping_mul(right),
        BinaryOp::Divide if right != 0 => left.wrapping_div(right),
        BinaryOp::Modulo if right != 0 => left.wrapping_rem(right),
        BinaryOp::Add => left.wrapping_add(right),
        BinaryOp::Subtract => left.wrapping_sub(right),
        BinaryOp::ShiftLeft => left.wrapping_shl(u32::try_from(right).unwrap_or_default()),
        BinaryOp::ShiftRight => left.wrapping_shr(u32::try_from(right).unwrap_or_default()),
        BinaryOp::Less => i64::from(left < right),
        BinaryOp::LessEqual => i64::from(left <= right),
        BinaryOp::Greater => i64::from(left > right),
        BinaryOp::GreaterEqual => i64::from(left >= right),
        BinaryOp::Equal => i64::from(left == right),
        BinaryOp::NotEqual => i64::from(left != right),
        BinaryOp::BitAnd => left & right,
        BinaryOp::BitXor => left ^ right,
        BinaryOp::BitOr => left | right,
        BinaryOp::LogicalAnd => i64::from(left != 0 && right != 0),
        BinaryOp::LogicalXor => i64::from((left != 0) ^ (right != 0)),
        BinaryOp::LogicalOr => i64::from(left != 0 || right != 0),
        BinaryOp::Nand => i64::from(!(left != 0 && right != 0)),
        BinaryOp::Nor => i64::from(!(left != 0 || right != 0)),
        BinaryOp::Divide | BinaryOp::Modulo => {
            return Err(DimError::Invalid("division by zero".into()));
        }
    };
    Ok(ConstantValue::Integer(value))
}
