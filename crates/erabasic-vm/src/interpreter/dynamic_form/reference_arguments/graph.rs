//! Object nodes retain child mutations. Replacing a root is a separate operation;
//! unique methods' shallow scratch arrays can therefore discard just that root.
use super::invalid;
use crate::interpreter::StepError;
use crate::{ProgramGeneration, VmValue};
use erabasic_ast::{Expr, ExprKind, FormPart, FormattedString};
use erabasic_bytecode::{
    ReferenceTermCall, ReferenceTermGraph, ReferenceTermId, ReferenceTermKind, ReferenceTermPart,
    ReferenceTermValue,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(in crate::interpreter::dynamic_form) enum TermRef {
    Original(ReferenceTermId),
    Single(ReferenceTermValue),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct PreparedReferenceArguments {
    pub(in crate::interpreter::dynamic_form) template: ReferenceTermGraph,
    pub(in crate::interpreter::dynamic_form) edges: Vec<Vec<TermRef>>,
    pub(in crate::interpreter::dynamic_form) roots: Vec<Option<TermRef>>,
}

impl PreparedReferenceArguments {
    pub(in crate::interpreter::dynamic_form) fn new(template: ReferenceTermGraph) -> Self {
        let edges = template
            .nodes
            .iter()
            .map(|node| node.children().into_iter().map(TermRef::Original).collect())
            .collect();
        let roots = template
            .roots
            .iter()
            .map(|root| root.map(TermRef::Original))
            .collect();
        Self {
            template,
            edges,
            roots,
        }
    }

    pub(in crate::interpreter::dynamic_form) fn single<'a>(
        &'a self,
        term: &'a TermRef,
    ) -> Option<&'a ReferenceTermValue> {
        match term {
            TermRef::Single(value) => Some(value),
            TermRef::Original(id) => match &self.template.nodes[*id as usize].kind {
                ReferenceTermKind::Value(value) => Some(value),
                _ => None,
            },
        }
    }

    pub(in crate::interpreter::dynamic_form) fn from_value(
        value: VmValue,
    ) -> Result<TermRef, StepError> {
        Ok(TermRef::Single(match value {
            VmValue::Integer(value) => ReferenceTermValue::Integer(value),
            VmValue::String(value) => ReferenceTermValue::String(value),
            _ => return Err(invalid("reference argument folded value is a place")),
        }))
    }

    /// Every replacement is scalar and must preserve the original edge type.
    /// This runs before any Native restoration when decoding a VM snapshot.
    pub(crate) fn valid_for_template(&self, template: &ReferenceTermGraph) -> bool {
        if &self.template != template || self.edges.len() != template.nodes.len() {
            return false;
        }
        for (node, replacements) in template.nodes.iter().zip(&self.edges) {
            let children = node.children();
            if children.len() != replacements.len() {
                return false;
            }
            for (original, replacement) in children.iter().zip(replacements) {
                if !self.valid_replacement(*original, replacement) {
                    return false;
                }
            }
        }
        template.roots.len() == self.roots.len()
            && template
                .roots
                .iter()
                .zip(&self.roots)
                .all(|(id, replacement)| match (id, replacement) {
                    (Some(id), Some(replacement)) => self.valid_replacement(*id, replacement),
                    (None, None) => true,
                    _ => false,
                })
    }

    fn valid_replacement(&self, original: ReferenceTermId, replacement: &TermRef) -> bool {
        match replacement {
            TermRef::Original(id) => *id == original,
            TermRef::Single(value) => self
                .template
                .nodes
                .get(original as usize)
                .is_some_and(|node| node.value_type == value.value_type()),
        }
    }

    pub(in crate::interpreter::dynamic_form) fn string_bytes(&self) -> Option<usize> {
        self.edges
            .iter()
            .flatten()
            .chain(self.roots.iter().flatten())
            .try_fold(0usize, |count, term| {
                count.checked_add(match term {
                    TermRef::Single(ReferenceTermValue::String(value)) => value.len(),
                    _ => 0,
                })
            })
    }

    /// This is a syntax adapter only: evaluation uses `RuntimeForm`'s existing
    /// arithmetic, user-call continuation, mutation and Native transaction paths.
    pub(in crate::interpreter::dynamic_form) fn expression(
        &self,
        program: &ProgramGeneration,
        term: &TermRef,
    ) -> Result<Expr, StepError> {
        match term {
            TermRef::Single(value) => Ok(Expr {
                kind: literal(value),
                span: erabasic_ast::Span::default(),
            }),
            TermRef::Original(id) => {
                let node = self
                    .template
                    .nodes
                    .get(*id as usize)
                    .ok_or_else(|| invalid("reference argument node is missing"))?;
                let edges = self
                    .edges
                    .get(*id as usize)
                    .ok_or_else(|| invalid("reference argument edges are missing"))?;
                let mut children = edges.iter();
                let mut child = || {
                    self.expression(
                        program,
                        children
                            .next()
                            .ok_or_else(|| invalid("reference argument child is missing"))?,
                    )
                };
                let kind = match &node.kind {
                    ReferenceTermKind::Value(value) => literal(value),
                    ReferenceTermKind::Variable { key, .. } => {
                        let definition = program
                            .global(*key)
                            .ok_or_else(|| invalid("reference argument variable disappeared"))?;
                        ExprKind::Variable {
                            name: definition.name.clone(),
                            indices: edges
                                .iter()
                                .map(|edge| self.expression(program, edge))
                                .collect::<Result<_, _>>()?,
                        }
                    }
                    ReferenceTermKind::Unary { op, .. } => ExprKind::Unary {
                        op: *op,
                        operand: Box::new(child()?),
                    },
                    ReferenceTermKind::Postfix { op, .. } => ExprKind::Postfix {
                        op: *op,
                        operand: Box::new(child()?),
                    },
                    ReferenceTermKind::Binary { op, .. } => ExprKind::Binary {
                        op: *op,
                        left: Box::new(child()?),
                        right: Box::new(child()?),
                    },
                    ReferenceTermKind::Ternary { .. } => ExprKind::Ternary {
                        condition: Box::new(child()?),
                        then_expr: Box::new(child()?),
                        else_expr: Box::new(child()?),
                    },
                    ReferenceTermKind::Call { target, arguments } => {
                        let name = match target {
                            ReferenceTermCall::Native { name, .. }
                            | ReferenceTermCall::DynamicNative { name, .. }
                            | ReferenceTermCall::Host { name, .. }
                            | ReferenceTermCall::Intrinsic { name } => name.clone(),
                            ReferenceTermCall::User { key } => program
                                .function(*key)
                                .ok_or_else(|| invalid("reference argument method disappeared"))?
                                .name
                                .clone(),
                        };
                        let args = arguments
                            .iter()
                            .map(|arg| {
                                if arg.node.is_some() {
                                    child().map(Some)
                                } else {
                                    Ok(None)
                                }
                            })
                            .collect::<Result<_, _>>()?;
                        ExprKind::Call { name, args }
                    }
                    ReferenceTermKind::Form { .. } => {
                        ExprKind::Formatted(self.form(program, term)?)
                    }
                };
                Ok(Expr {
                    kind,
                    span: node.span,
                })
            }
        }
    }

    fn form(
        &self,
        program: &ProgramGeneration,
        term: &TermRef,
    ) -> Result<FormattedString, StepError> {
        if let Some(ReferenceTermValue::String(value)) = self.single(term) {
            return Ok(FormattedString {
                parts: vec![FormPart::Text(value.clone())],
                span: erabasic_ast::Span::default(),
            });
        }
        let TermRef::Original(id) = term else {
            return Err(invalid("reference argument form replacement is not String"));
        };
        let node = &self.template.nodes[*id as usize];
        let ReferenceTermKind::Form { parts } = &node.kind else {
            return Err(invalid(
                "reference argument conditional branch is not a form",
            ));
        };
        let mut edges = self.edges[*id as usize].iter();
        let mut next = || {
            edges
                .next()
                .ok_or_else(|| invalid("reference argument form child is missing"))
        };
        let mut result = Vec::with_capacity(parts.len());
        for part in parts {
            result.push(match part {
                ReferenceTermPart::Text(value) => FormPart::Text(value.clone()),
                ReferenceTermPart::Triple(symbol) => FormPart::Triple {
                    symbol: *symbol,
                    span: node.span,
                },
                ReferenceTermPart::Interpolation {
                    width,
                    integer,
                    alignment,
                    ..
                } => {
                    let expression = Box::new(self.expression(program, next()?)?);
                    let width = width
                        .map(|_| {
                            next()
                                .and_then(|term| self.expression(program, term))
                                .map(Box::new)
                        })
                        .transpose()?;
                    if *integer {
                        FormPart::IntegerInterpolation {
                            expression,
                            width,
                            alignment: *alignment,
                            span: node.span,
                        }
                    } else {
                        FormPart::StringInterpolation {
                            expression,
                            width,
                            alignment: *alignment,
                            span: node.span,
                        }
                    }
                }
                ReferenceTermPart::Conditional { else_value, .. } => {
                    let condition = Box::new(self.expression(program, next()?)?);
                    let then_value = Box::new(self.form(program, next()?)?);
                    let else_value = else_value
                        .map(|_| {
                            next()
                                .and_then(|term| self.form(program, term))
                                .map(Box::new)
                        })
                        .transpose()?;
                    FormPart::Conditional {
                        condition,
                        then_value,
                        else_value,
                        span: node.span,
                    }
                }
            });
        }
        Ok(FormattedString {
            parts: result,
            span: node.span,
        })
    }
}

fn literal(value: &ReferenceTermValue) -> ExprKind {
    match value {
        ReferenceTermValue::Integer(value) => ExprKind::Integer(*value),
        ReferenceTermValue::String(value) => ExprKind::String(value.clone()),
    }
}
