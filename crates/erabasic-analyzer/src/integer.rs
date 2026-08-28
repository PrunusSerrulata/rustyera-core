use erabasic_ast::BinaryOp;
use erabasic_compat::IntegerOperation;

pub(crate) fn binary_operation(operation: BinaryOp) -> Option<IntegerOperation> {
    Some(match operation {
        BinaryOp::Add => IntegerOperation::Add,
        BinaryOp::Subtract => IntegerOperation::Subtract,
        BinaryOp::Multiply => IntegerOperation::Multiply,
        BinaryOp::Divide => IntegerOperation::Divide,
        BinaryOp::Modulo => IntegerOperation::Modulo,
        _ => return None,
    })
}
