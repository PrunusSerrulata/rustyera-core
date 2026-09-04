use std::borrow::Cow;

use erabasic_ast::{BinaryOp, Expr, ExprKind};
use erabasic_compat::IntegerArithmeticPolicy;
use erabasic_hir::ConstantValue;

pub(super) fn normalize_colon_indices(indices: &[Expr], maximum: usize) -> Cow<'_, [Expr]> {
    if maximum == 0 || indices.len() <= maximum {
        return Cow::Borrowed(indices);
    }
    // Syntax parsing cannot know whether `TMP:ARG:0` describes a two-dimensional
    // TMP or a one-dimensional TMP indexed by ARG:0. Once the declaration is
    // resolved, fold the excess colon tail into the final permitted index. This
    // mirrors the reference's variable-token-aware expression parser without
    // leaking semantic variable shapes into the public parser context.
    let nested_start = maximum - 1;
    let mut normalized = indices[..nested_start].to_vec();
    let mut nested = indices[nested_start].clone();
    let extra = indices[nested_start + 1..].to_vec();
    let end = extra.last().map_or(nested.span, |value| value.span);
    nested.kind = match nested.kind {
        ExprKind::Identifier(name) => ExprKind::Variable {
            name,
            indices: extra,
        },
        ExprKind::Variable {
            name,
            indices: mut existing,
        } => {
            existing.extend(extra);
            ExprKind::Variable {
                name,
                indices: existing,
            }
        }
        _ => return Cow::Borrowed(indices),
    };
    nested.span = nested.span.join(end);
    normalized.push(nested);
    Cow::Owned(normalized)
}

#[allow(clippy::too_many_lines)]
pub(super) fn fold_binary(
    op: BinaryOp,
    left: Option<&ConstantValue>,
    right: Option<&ConstantValue>,
    policy: IntegerArithmeticPolicy,
) -> Option<ConstantValue> {
    match (left?, right?) {
        (ConstantValue::String(left), ConstantValue::String(right)) => match op {
            BinaryOp::Add => Some(ConstantValue::String(format!("{left}{right}"))),
            BinaryOp::Equal => Some(ConstantValue::Integer(i64::from(left == right))),
            BinaryOp::NotEqual => Some(ConstantValue::Integer(i64::from(left != right))),
            BinaryOp::Less => Some(ConstantValue::Integer(i64::from(left < right))),
            BinaryOp::LessEqual => Some(ConstantValue::Integer(i64::from(left <= right))),
            BinaryOp::Greater => Some(ConstantValue::Integer(i64::from(left > right))),
            BinaryOp::GreaterEqual => Some(ConstantValue::Integer(i64::from(left >= right))),
            _ => None,
        },
        (ConstantValue::String(value), ConstantValue::Integer(count))
        | (ConstantValue::Integer(count), ConstantValue::String(value))
            if op == BinaryOp::Multiply && (0..10_000).contains(count) =>
        {
            Some(ConstantValue::String(
                value.repeat(usize::try_from(*count).ok()?),
            ))
        }
        (ConstantValue::Integer(left), ConstantValue::Integer(right)) => {
            if policy == IntegerArithmeticPolicy::SnakeSaturatingV1
                && let Some(operation) = crate::integer::binary_operation(op)
            {
                let result = policy.evaluate(operation, *left, Some(*right)).ok()?;
                // A folded constant would bypass the VM diagnostic and its source identity.
                return result
                    .warning
                    .is_none()
                    .then_some(ConstantValue::Integer(result.value));
            }
            // Keep the established reference load-time behavior, including wrapping
            // MIN / -1 and MIN % -1; changing that legacy difference is out of scope.
            Some(ConstantValue::Integer(match op {
                BinaryOp::Multiply => left.wrapping_mul(*right),
                BinaryOp::Divide if *right != 0 => left.wrapping_div(*right),
                BinaryOp::Modulo if *right != 0 => left.wrapping_rem(*right),
                BinaryOp::Add => left.wrapping_add(*right),
                BinaryOp::Subtract => left.wrapping_sub(*right),
                BinaryOp::ShiftLeft => left.wrapping_shl(u32::try_from(*right).unwrap_or_default()),
                BinaryOp::ShiftRight => {
                    left.wrapping_shr(u32::try_from(*right).unwrap_or_default())
                }
                BinaryOp::Less => i64::from(left < right),
                BinaryOp::LessEqual => i64::from(left <= right),
                BinaryOp::Greater => i64::from(left > right),
                BinaryOp::GreaterEqual => i64::from(left >= right),
                BinaryOp::Equal => i64::from(left == right),
                BinaryOp::NotEqual => i64::from(left != right),
                BinaryOp::BitAnd => left & right,
                BinaryOp::BitXor => left ^ right,
                BinaryOp::BitOr => left | right,
                BinaryOp::LogicalAnd => i64::from(*left != 0 && *right != 0),
                BinaryOp::LogicalXor => i64::from((*left != 0) ^ (*right != 0)),
                BinaryOp::LogicalOr => i64::from(*left != 0 || *right != 0),
                BinaryOp::Nand => i64::from(!(*left != 0 && *right != 0)),
                BinaryOp::Nor => i64::from(!(*left != 0 || *right != 0)),
                BinaryOp::Divide | BinaryOp::Modulo => return None,
            }))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_folding_retains_operations_that_warn_or_fault() {
        let left = ConstantValue::Integer(i64::MAX);
        let one = ConstantValue::Integer(1);
        assert_eq!(
            fold_binary(
                BinaryOp::Add,
                Some(&left),
                Some(&one),
                IntegerArithmeticPolicy::SnakeSaturatingV1
            ),
            None,
        );
        assert_eq!(
            fold_binary(
                BinaryOp::Add,
                Some(&left),
                Some(&one),
                IntegerArithmeticPolicy::ReferenceWrappingV1
            ),
            Some(ConstantValue::Integer(i64::MIN)),
        );
        for operation in [BinaryOp::Divide, BinaryOp::Modulo] {
            let minimum = ConstantValue::Integer(i64::MIN);
            let negative_one = ConstantValue::Integer(-1);
            assert_eq!(
                fold_binary(
                    operation,
                    Some(&minimum),
                    Some(&negative_one),
                    IntegerArithmeticPolicy::SnakeSaturatingV1
                ),
                None
            );
            assert_eq!(
                fold_binary(
                    operation,
                    Some(&minimum),
                    Some(&negative_one),
                    IntegerArithmeticPolicy::ReferenceWrappingV1
                ),
                Some(ConstantValue::Integer(if operation == BinaryOp::Divide {
                    i64::MIN
                } else {
                    0
                }))
            );
            assert_eq!(
                fold_binary(
                    operation,
                    Some(&one),
                    Some(&ConstantValue::Integer(0)),
                    IntegerArithmeticPolicy::SnakeSaturatingV1
                ),
                None
            );
        }
        assert_eq!(
            fold_binary(
                BinaryOp::Add,
                Some(&one),
                Some(&one),
                IntegerArithmeticPolicy::SnakeSaturatingV1
            ),
            Some(ConstantValue::Integer(2))
        );
    }
}
