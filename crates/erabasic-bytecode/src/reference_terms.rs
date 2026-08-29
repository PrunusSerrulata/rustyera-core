//! Bounded pre-Restructure source terms shared by runtime calls and static templates.
//! Variable properties are obtained from the artifact's runtime-variable table.
//! A payload never grants itself const, REF, callable, or provider permissions.
use crate::{BytecodeType, SymbolKey};
use erabasic_ast::{Alignment, BinaryOp, PostfixOp, Span, UnaryOp};
use serde::{Deserialize, Serialize};

pub const MAX_REFERENCE_TERM_PAYLOAD: usize = 1024 * 1024;
pub const MAX_REFERENCE_TERM_NODES: usize = 16_384;
pub const MAX_REFERENCE_TERM_DEPTH: usize = 256;
pub type ReferenceTermId = u32;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ReferenceTermValue {
    Integer(i64),
    String(String),
}
impl ReferenceTermValue {
    #[must_use]
    pub const fn value_type(&self) -> BytecodeType {
        match self {
            Self::Integer(_) => BytecodeType::Integer,
            Self::String(_) => BytecodeType::String,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ReferenceTermCall {
    Native { key: SymbolKey, name: String },
    DynamicNative { key: SymbolKey, name: String },
    Host { key: SymbolKey, name: String },
    User { key: SymbolKey },
    Intrinsic { name: String },
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReferenceTermArgument {
    pub node: Option<ReferenceTermId>,
    pub place: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ReferenceTermPart {
    Text(String),
    Interpolation {
        expression: ReferenceTermId,
        width: Option<ReferenceTermId>,
        integer: bool,
        alignment: Option<Alignment>,
    },
    Conditional {
        condition: ReferenceTermId,
        then_value: ReferenceTermId,
        else_value: Option<ReferenceTermId>,
    },
    Triple(char),
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ReferenceTermKind {
    Value(ReferenceTermValue),
    Variable {
        key: SymbolKey,
        indices: Vec<ReferenceTermId>,
    },
    Unary {
        op: UnaryOp,
        operand: ReferenceTermId,
    },
    Postfix {
        op: PostfixOp,
        operand: ReferenceTermId,
    },
    Binary {
        op: BinaryOp,
        left: ReferenceTermId,
        right: ReferenceTermId,
    },
    Ternary {
        condition: ReferenceTermId,
        then_value: ReferenceTermId,
        else_value: ReferenceTermId,
    },
    Call {
        target: ReferenceTermCall,
        arguments: Vec<ReferenceTermArgument>,
    },
    Form {
        parts: Vec<ReferenceTermPart>,
    },
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReferenceTermNode {
    pub kind: ReferenceTermKind,
    pub value_type: BytecodeType,
    pub span: Span,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReferenceTermGraph {
    pub nodes: Vec<ReferenceTermNode>,
    pub roots: Vec<Option<ReferenceTermId>>,
}

impl ReferenceTermNode {
    /// Edge order is the source's Restructure order, including unselected branches.
    #[must_use]
    pub fn children(&self) -> Vec<ReferenceTermId> {
        match &self.kind {
            ReferenceTermKind::Value(_) => Vec::new(),
            ReferenceTermKind::Variable { indices, .. } => indices.clone(),
            ReferenceTermKind::Unary { operand, .. }
            | ReferenceTermKind::Postfix { operand, .. } => vec![*operand],
            ReferenceTermKind::Binary { left, right, .. } => vec![*left, *right],
            ReferenceTermKind::Ternary {
                condition,
                then_value,
                else_value,
            } => vec![*condition, *then_value, *else_value],
            ReferenceTermKind::Call { arguments, .. } => {
                arguments.iter().filter_map(|arg| arg.node).collect()
            }
            ReferenceTermKind::Form { parts } => parts
                .iter()
                .flat_map(|part| match part {
                    ReferenceTermPart::Text(_) | ReferenceTermPart::Triple(_) => Vec::new(),
                    ReferenceTermPart::Interpolation {
                        expression, width, ..
                    } => std::iter::once(*expression).chain(*width).collect(),
                    ReferenceTermPart::Conditional {
                        condition,
                        then_value,
                        else_value,
                    } => [*condition, *then_value]
                        .into_iter()
                        .chain(*else_value)
                        .collect(),
                })
                .collect(),
        }
    }
}

impl ReferenceTermGraph {
    /// Validate bounded postorder edges and scalar term types.
    ///
    /// # Errors
    /// Returns an explanation when a graph exceeds its limits or contains an
    /// invalid edge, root, source span, or operand type.
    pub fn validate_structure(&self) -> Result<(), &'static str> {
        if self.nodes.len() > MAX_REFERENCE_TERM_NODES
            || self.roots.len() > MAX_REFERENCE_TERM_NODES
        {
            return Err("reference argument template node limit");
        }
        let mut depths = Vec::<usize>::with_capacity(self.nodes.len());
        for (position, node) in self.nodes.iter().enumerate() {
            if node.span.start > node.span.end
                || !matches!(
                    node.value_type,
                    BytecodeType::Integer | BytecodeType::String
                )
            {
                return Err("reference argument node source or type is invalid");
            }
            let children = node.children();
            if children.iter().any(|child| *child as usize >= position) {
                return Err("reference argument graph is not in postorder");
            }
            let depth = children
                .iter()
                .map(|child| depths[*child as usize])
                .max()
                .unwrap_or(0)
                + 1;
            if depth > MAX_REFERENCE_TERM_DEPTH {
                return Err("reference argument template depth limit");
            }
            depths.push(depth);
            let valid = self.node_types_valid(node, &children);
            if !valid {
                return Err("reference argument typed term is inconsistent");
            }
        }
        if self
            .roots
            .iter()
            .flatten()
            .any(|root| *root as usize >= self.nodes.len())
        {
            return Err("reference argument root is outside its graph");
        }
        Ok(())
    }
    fn node_types_valid(&self, node: &ReferenceTermNode, children: &[ReferenceTermId]) -> bool {
        let ty = |child: ReferenceTermId| self.nodes[child as usize].value_type;
        let integer = |child| ty(child) == BytecodeType::Integer;
        match &node.kind {
            ReferenceTermKind::Value(value) => value.value_type() == node.value_type,
            ReferenceTermKind::Variable { indices, .. } => indices.iter().copied().all(integer),
            ReferenceTermKind::Unary { .. } | ReferenceTermKind::Postfix { .. } => {
                node.value_type == BytecodeType::Integer && children.iter().copied().all(integer)
            }
            ReferenceTermKind::Binary { op, left, right } => {
                let same = ty(*left) == ty(*right);
                same && match op {
                    BinaryOp::Add => node.value_type == ty(*left),
                    BinaryOp::Equal
                    | BinaryOp::NotEqual
                    | BinaryOp::Less
                    | BinaryOp::LessEqual
                    | BinaryOp::Greater
                    | BinaryOp::GreaterEqual => node.value_type == BytecodeType::Integer,
                    _ => integer(*left) && node.value_type == BytecodeType::Integer,
                }
            }
            ReferenceTermKind::Ternary {
                condition,
                then_value,
                else_value,
            } => {
                integer(*condition)
                    && ty(*then_value) == node.value_type
                    && ty(*else_value) == node.value_type
            }
            ReferenceTermKind::Call { arguments, .. } => arguments.iter().all(|arg| {
                !arg.place
                    || arg.node.is_some_and(|id| {
                        matches!(
                            self.nodes[id as usize].kind,
                            ReferenceTermKind::Variable { .. }
                        )
                    })
            }),
            ReferenceTermKind::Form { parts } => {
                node.value_type == BytecodeType::String
                    && parts.iter().all(|part| match part {
                        ReferenceTermPart::Text(_) => true,
                        ReferenceTermPart::Triple(symbol) => {
                            matches!(symbol, '*' | '+' | '=' | '/' | '$')
                        }
                        ReferenceTermPart::Interpolation {
                            expression,
                            width,
                            integer: numeric,
                            ..
                        } => {
                            ty(*expression)
                                == if *numeric {
                                    BytecodeType::Integer
                                } else {
                                    BytecodeType::String
                                }
                                && width.is_none_or(integer)
                        }
                        ReferenceTermPart::Conditional {
                            condition,
                            then_value,
                            else_value,
                        } => {
                            integer(*condition)
                                && matches!(
                                    self.nodes[*then_value as usize].kind,
                                    ReferenceTermKind::Form { .. }
                                )
                                && else_value.is_none_or(|id| {
                                    matches!(
                                        self.nodes[id as usize].kind,
                                        ReferenceTermKind::Form { .. }
                                    )
                                })
                        }
                    })
            }
        }
    }
}
