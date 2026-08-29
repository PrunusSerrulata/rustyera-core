//! Syntax-only graph construction. Types come from the one existing `TypeAnalysis`
//! visitor; no storage reads, speculative Native calls, or fabricated imports.
use super::{
    BytecodeType, Expr, ExprKind, FormPart, FormattedString, MAX_RUNTIME_FORM_NESTING,
    ReferenceTermCall, ReferenceTermKind, StepError, SymbolKey, invalid, resource_limit,
};
use erabasic_bytecode::{
    ReferenceTermArgument, ReferenceTermNode, ReferenceTermPart, ReferenceTermValue,
};
use std::collections::BTreeMap;

pub(in crate::interpreter::dynamic_form) struct GraphBuilder<'a> {
    program: &'a crate::ProgramGeneration,
    function: SymbolKey,
    types: &'a BTreeMap<usize, BytecodeType>,
    graph: erabasic_bytecode::ReferenceTermGraph,
}
impl<'a> GraphBuilder<'a> {
    pub(in crate::interpreter::dynamic_form) fn new(
        program: &'a crate::ProgramGeneration,
        function: SymbolKey,
        types: &'a BTreeMap<usize, BytecodeType>,
    ) -> Self {
        Self {
            program,
            function,
            types,
            graph: erabasic_bytecode::ReferenceTermGraph {
                nodes: Vec::new(),
                roots: Vec::new(),
            },
        }
    }
    pub(in crate::interpreter::dynamic_form) fn build(
        mut self,
        arguments: &[Option<Expr>],
    ) -> Result<erabasic_bytecode::ReferenceTermGraph, StepError> {
        for argument in arguments {
            let root = argument
                .as_ref()
                .map(|expression| self.expression(expression, 0))
                .transpose()?;
            self.graph.roots.push(root);
        }
        self.graph.validate_structure().map_err(invalid)?;
        Ok(self.graph)
    }
    fn push(
        &mut self,
        kind: ReferenceTermKind,
        value_type: BytecodeType,
        span: erabasic_ast::Span,
    ) -> Result<u32, StepError> {
        if self.graph.nodes.len() >= erabasic_bytecode::MAX_REFERENCE_TERM_NODES {
            return Err(resource_limit(
                "reference argument graph exceeds node limit",
            ));
        }
        let id = u32::try_from(self.graph.nodes.len())
            .map_err(|_| resource_limit("reference argument node identity exhausted"))?;
        self.graph.nodes.push(ReferenceTermNode {
            kind,
            value_type,
            span,
        });
        Ok(id)
    }
    fn expression(&mut self, expression: &Expr, depth: usize) -> Result<u32, StepError> {
        if depth > MAX_RUNTIME_FORM_NESTING {
            return Err(resource_limit(
                "reference argument graph exceeds nesting limit",
            ));
        }
        let value_type = *self
            .types
            .get(&(std::ptr::from_ref(expression) as usize))
            .ok_or_else(|| invalid("reference term lacks its analyzed scalar type"))?;
        let kind = match &expression.kind {
            ExprKind::Integer(value) => {
                ReferenceTermKind::Value(ReferenceTermValue::Integer(*value))
            }
            ExprKind::String(value) => {
                ReferenceTermKind::Value(ReferenceTermValue::String(value.clone()))
            }
            ExprKind::Group(inner) => return self.expression(inner, depth + 1),
            ExprKind::Identifier(name) | ExprKind::Variable { name, .. } => {
                let definition = self
                    .program
                    .scoped_variable(self.function, name)
                    .ok_or_else(|| invalid("analyzed reference variable disappeared"))?;
                let key = definition.key;
                let indices = if let ExprKind::Variable { indices, .. } = &expression.kind {
                    indices
                        .iter()
                        .map(|index| self.expression(index, depth + 1))
                        .collect::<Result<_, _>>()?
                } else {
                    Vec::new()
                };
                ReferenceTermKind::Variable { key, indices }
            }
            ExprKind::Unary { op, operand } => ReferenceTermKind::Unary {
                op: *op,
                operand: self.expression(operand, depth + 1)?,
            },
            ExprKind::Postfix { op, operand } => ReferenceTermKind::Postfix {
                op: *op,
                operand: self.expression(operand, depth + 1)?,
            },
            ExprKind::Binary { op, left, right } => ReferenceTermKind::Binary {
                op: *op,
                left: self.expression(left, depth + 1)?,
                right: self.expression(right, depth + 1)?,
            },
            ExprKind::Ternary {
                condition,
                then_expr,
                else_expr,
            } => ReferenceTermKind::Ternary {
                condition: self.expression(condition, depth + 1)?,
                then_value: self.expression(then_expr, depth + 1)?,
                else_value: self.expression(else_expr, depth + 1)?,
            },
            ExprKind::Formatted(form) => return self.form(form, depth + 1),
            ExprKind::Call { name, args } => self.call(name, args, depth)?,
            ExprKind::Error => {
                return Err(invalid("parser error reached reference graph construction"));
            }
        };
        self.push(kind, value_type, expression.span)
    }
    fn call(
        &mut self,
        name: &str,
        args: &[Option<Expr>],
        depth: usize,
    ) -> Result<ReferenceTermKind, StepError> {
        let user = self.program.function_by_name(name).map(|function| {
            (
                function.key,
                function
                    .parameters
                    .iter()
                    .map(|parameter| parameter.by_reference)
                    .collect::<Vec<_>>(),
            )
        });
        let parameters = user.as_ref().map(|(_, parameters)| parameters.len());
        let mut arguments = Vec::with_capacity(args.len());
        for (slot, argument) in args.iter().enumerate() {
            // Create() already called ConvertArg. Keep only arity bookkeeping
            // for the existing warning path, never an excess term/child edge.
            let node = if parameters.is_some_and(|count| slot >= count) {
                None
            } else {
                argument
                    .as_ref()
                    .map(|argument| self.expression(argument, depth + 1))
                    .transpose()?
            };
            arguments.push(ReferenceTermArgument {
                node,
                place: user
                    .as_ref()
                    .is_some_and(|(_, parameters)| parameters.get(slot).copied().unwrap_or(false)),
            });
        }
        let target = if let Some((key, _)) = user {
            ReferenceTermCall::User { key }
        } else if matches!(
            name.to_ascii_uppercase().as_str(),
            "STRFORM" | "STRFORMCHECK" | "GETMETH" | "GETMETHS" | "EXISTMETH" | "EXISTVAR"
        ) {
            ReferenceTermCall::Intrinsic {
                name: name.to_ascii_uppercase(),
            }
        } else {
            let shapes = args
                .iter()
                .map(|argument| {
                    argument
                        .as_ref()
                        .map(|expression| {
                            let ty = *self
                                .types
                                .get(&(std::ptr::from_ref(expression) as usize))
                                .ok_or_else(|| invalid("Native term lacks analyzed type"))?;
                            Ok(super::super::typing::source_shape(
                                self.program,
                                self.function,
                                expression,
                                ty,
                            ))
                        })
                        .transpose()
                })
                .collect::<Result<Vec<_>, StepError>>()?;
            let host = self
                .program
                .artifact
                .runtime_host_authorizations
                .iter()
                .any(|family| family.name.eq_ignore_ascii_case(name));
            let (target, parameters) = if host {
                let bound = super::super::host_calls::bind(self.program, name, &shapes)?;
                (
                    ReferenceTermCall::Host {
                        key: bound.family_key,
                        name: name.to_ascii_uppercase(),
                    },
                    bound.import.parameters,
                )
            } else {
                let bound = super::super::native_binding::bind(self.program, name, &shapes, None)?;
                (
                    ReferenceTermCall::DynamicNative {
                        key: bound.service_key,
                        name: name.to_ascii_uppercase(),
                    },
                    bound.import.parameters,
                )
            };
            for (argument, parameter) in arguments.iter_mut().zip(&parameters) {
                argument.place = matches!(
                    parameter,
                    BytecodeType::IntegerPlace | BytecodeType::StringPlace
                );
            }
            target
        };
        Ok(ReferenceTermKind::Call { target, arguments })
    }
    fn form(&mut self, form: &FormattedString, depth: usize) -> Result<u32, StepError> {
        if depth > MAX_RUNTIME_FORM_NESTING {
            return Err(resource_limit("reference form graph exceeds nesting limit"));
        }
        let mut parts = Vec::with_capacity(form.parts.len());
        for part in &form.parts {
            parts.push(match part {
                FormPart::Text(value) => ReferenceTermPart::Text(value.clone()),
                FormPart::Triple { symbol, .. } => ReferenceTermPart::Triple(*symbol),
                FormPart::IntegerInterpolation {
                    expression,
                    width,
                    alignment,
                    ..
                }
                | FormPart::StringInterpolation {
                    expression,
                    width,
                    alignment,
                    ..
                } => ReferenceTermPart::Interpolation {
                    expression: self.expression(expression, depth + 1)?,
                    width: width
                        .as_ref()
                        .map(|width| self.expression(width, depth + 1))
                        .transpose()?,
                    integer: matches!(part, FormPart::IntegerInterpolation { .. }),
                    alignment: *alignment,
                },
                FormPart::Conditional {
                    condition,
                    then_value,
                    else_value,
                    ..
                } => ReferenceTermPart::Conditional {
                    condition: self.expression(condition, depth + 1)?,
                    then_value: self.form(then_value, depth + 1)?,
                    else_value: else_value
                        .as_ref()
                        .map(|value| self.form(value, depth + 1))
                        .transpose()?,
                },
            });
        }
        self.push(
            ReferenceTermKind::Form { parts },
            BytecodeType::String,
            form.span,
        )
    }
}
