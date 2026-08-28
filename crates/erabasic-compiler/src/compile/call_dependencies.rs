//! Function-local call-signature dependencies; no callee bodies enter this index.

use std::collections::{BTreeMap, BTreeSet};

use erabasic_bytecode::{Digest, SymbolKey};
use erabasic_hir::{
    CallTarget, ConstantValue, ControlFlowKind, Function, FunctionId, HirArgument, HirCallArgument,
    HirExpr, HirExprKind, HirFormPart, HirFormattedString, HirPlace, HirStatementKind, Variable,
};

use super::{DenseIdIndex, FunctionSignature, canonical_digest};

pub(super) struct CallDependencies<'a> {
    signatures: DenseIdIndex<&'a FunctionSignature>,
    digests: DenseIdIndex<(SymbolKey, Digest)>,
    names: BTreeMap<String, Vec<FunctionId>>,
    dynamic: Digest,
}

impl<'a> CallDependencies<'a> {
    pub(super) fn new(
        signatures: &'a [FunctionSignature],
        keys: &DenseIdIndex<SymbolKey>,
        variables: &[Variable],
    ) -> Self {
        let mut result = Self {
            signatures: DenseIdIndex::new(signatures.len()),
            digests: DenseIdIndex::new(signatures.len()),
            names: BTreeMap::new(),
            dynamic: Digest::default(),
        };
        let mut all = Vec::with_capacity(signatures.len());
        for signature in signatures {
            let key = *keys.get(signature.id.0).expect("validated function key");
            let reference_contracts = signature
                .parameters
                .iter()
                .map(|parameter| {
                    variables
                        .get(parameter.target.variable.0 as usize)
                        .map(|variable| {
                            (
                                variable.reference,
                                &variable.dimensions,
                                variable.storage,
                                variable.scope,
                                variable.static_lifetime,
                            )
                        })
                })
                .collect::<Vec<_>>();
            // Source spans are presentation identities, not a called signature. In particular,
            // moving another function must not invalidate a caller through a default's span.
            let mut signature_value = serde_json::to_value((
                &signature.name,
                signature.kind,
                signature.return_type,
                &signature.parameters,
                reference_contracts,
            ))
            .expect("function signatures are serializable");
            erase_locations(&mut signature_value);
            let digest = canonical_digest("rustyera.compiler.call-signature.v1", &signature_value);
            result.signatures.insert(signature.id.0, signature);
            result.digests.insert(signature.id.0, (key, digest));
            result
                .names
                .entry(signature.name.to_ascii_uppercase())
                .or_default()
                .push(signature.id);
            all.push((key, digest));
        }
        all.sort_by_key(|entry| entry.0);
        result.dynamic = canonical_digest("rustyera.compiler.dynamic-signatures.v1", &all);
        result
    }

    pub(super) fn for_function(&self, function: &Function) -> Digest {
        let mut visitor = Visitor {
            index: self,
            targets: BTreeSet::new(),
            dynamic: false,
        };
        for parameter in &function.parameters {
            visitor.place(&parameter.target);
            if let Some(default) = &parameter.default {
                visitor.expression(default);
            }
        }
        let static_targets = function
            .control_flow
            .iter()
            .filter_map(|edge| {
                matches!(edge.kind, ControlFlowKind::Call | ControlFlowKind::Jump)
                    .then_some(edge.function)
                    .flatten()
                    .map(|target| (edge.from, target))
            })
            .collect::<BTreeMap<_, _>>();
        for statement in &function.lines {
            match &statement.kind {
                HirStatementKind::Instruction { target, arguments } => {
                    let static_target = static_targets.get(&statement.id).copied();
                    if let Some(target) = static_target {
                        visitor.target(target);
                        let count = self
                            .signatures
                            .get(target.0)
                            .map_or(0, |value| value.parameters.len());
                        // Static lowering retains only this formal prefix. Analyzer still checks
                        // the complete actual list and regenerates its load diagnostics.
                        for argument in arguments.iter().skip(1).take(count) {
                            visitor.argument(argument);
                        }
                    } else {
                        let name = target.name().to_ascii_uppercase();
                        visitor.runtime_target(&name, arguments.first().and_then(argument_name));
                        for argument in arguments {
                            visitor.argument(argument);
                        }
                    }
                }
                HirStatementKind::Assignment { target, value, .. } => {
                    visitor.place(target);
                    visitor.expression(value);
                }
                HirStatementKind::Label { .. } | HirStatementKind::Error => {}
            }
        }
        if visitor.dynamic {
            // Only a caller that can parse/resolve an unknown target depends on this universe.
            // Body-only edits never change it; unrelated static callers do not include it.
            return canonical_digest("rustyera.compiler.call-dependencies.v1", &self.dynamic);
        }
        let mut dependencies = visitor
            .targets
            .iter()
            .filter_map(|id| self.digests.get(id.0))
            .copied()
            .collect::<Vec<_>>();
        dependencies.sort_by_key(|entry| entry.0);
        canonical_digest("rustyera.compiler.call-dependencies.v1", &dependencies)
    }
}

fn erase_locations(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            fields.remove("location");
            for value in fields.values_mut() {
                erase_locations(value);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                erase_locations(value);
            }
        }
        _ => {}
    }
}

struct Visitor<'index, 'signature> {
    index: &'index CallDependencies<'signature>,
    targets: BTreeSet<FunctionId>,
    dynamic: bool,
}

impl Visitor<'_, '_> {
    fn target(&mut self, id: FunctionId) {
        if !self.targets.insert(id) {
            return;
        }
        if let Some(signature) = self.index.signatures.get(id.0).copied() {
            // A default is lowered in the caller. Its own calls therefore also carry signature
            // dependencies. The visited set bounds recursive default/signature graphs.
            for parameter in &signature.parameters {
                if let Some(default) = &parameter.default {
                    self.expression(default);
                }
            }
        }
    }

    fn runtime_target(&mut self, name: &str, literal: Option<String>) {
        if matches!(
            name,
            "STRFORM"
                | "STRFORMCHECK"
                | "EXISTVAR"
                | "CALLSTR"
                | "JUMPSTR"
                | "TRYCALLSTR"
                | "TRYJUMPSTR"
                | "TRYCCALLSTR"
                | "TRYCJUMPSTR"
                | "TRYCALLLIST"
                | "TRYJUMPLIST"
        ) {
            self.dynamic = true;
        } else if matches!(
            name,
            "GETMETH"
                | "GETMETHS"
                | "EXISTMETH"
                | "CALLEVENT"
                | "CALLFORM"
                | "CALLFORMF"
                | "JUMPFORM"
                | "TRYCALLFORM"
                | "TRYCALLFORMF"
                | "TRYJUMPFORM"
                | "TRYCCALL"
                | "TRYCCALLFORM"
                | "TRYCJUMP"
                | "TRYCJUMPFORM"
        ) {
            if let Some(name) = literal {
                let targets = self
                    .index
                    .names
                    .get(&name.to_ascii_uppercase())
                    .cloned()
                    .unwrap_or_default();
                for id in targets {
                    self.target(id);
                }
            } else {
                self.dynamic = true;
            }
        }
    }

    fn argument(&mut self, argument: &HirArgument) {
        match argument {
            HirArgument::Expression(value)
            | HirArgument::MixedExpression {
                expression: value, ..
            } => self.expression(value),
            HirArgument::Place(place) => self.place(place),
            HirArgument::Formatted(value) => self.formatted(value),
            HirArgument::Raw(_) | HirArgument::Omitted => {}
        }
    }

    fn expression(&mut self, expression: &HirExpr) {
        match &expression.kind {
            HirExprKind::Call { target, arguments } => {
                let count = if let CallTarget::User { function } = target {
                    self.target(*function);
                    self.index
                        .signatures
                        .get(function.0)
                        .map_or(0, |value| value.parameters.len())
                } else {
                    if let CallTarget::Builtin { name } = target {
                        self.runtime_target(
                            name,
                            arguments.first().and_then(|argument| {
                                if let HirCallArgument::Value(value) = argument {
                                    expression_name(value)
                                } else {
                                    None
                                }
                            }),
                        );
                    }
                    arguments.len()
                };
                for argument in arguments.iter().take(count) {
                    match argument {
                        HirCallArgument::Value(value) => self.expression(value),
                        HirCallArgument::Place(place) => self.place(place),
                        HirCallArgument::Omitted => {}
                    }
                }
            }
            HirExprKind::Variable { place } => self.place(place),
            HirExprKind::Unary { operand, .. } | HirExprKind::Postfix { operand, .. } => {
                self.expression(operand);
            }
            HirExprKind::Binary { left, right, .. } => {
                self.expression(left);
                self.expression(right);
            }
            HirExprKind::Ternary {
                condition,
                then_expr,
                else_expr,
            } => {
                self.expression(condition);
                self.expression(then_expr);
                self.expression(else_expr);
            }
            HirExprKind::Formatted { value } => self.formatted(value),
            HirExprKind::Integer { .. } | HirExprKind::String { .. } | HirExprKind::Error => {}
        }
    }

    fn place(&mut self, place: &HirPlace) {
        for index in &place.indices {
            self.expression(index);
        }
    }

    fn formatted(&mut self, formatted: &HirFormattedString) {
        for part in &formatted.parts {
            match part {
                HirFormPart::Interpolation {
                    expression, width, ..
                } => {
                    self.expression(expression);
                    if let Some(width) = width {
                        self.expression(width);
                    }
                }
                HirFormPart::Conditional {
                    condition,
                    then_value,
                    else_value,
                    ..
                } => {
                    self.expression(condition);
                    self.formatted(then_value);
                    if let Some(value) = else_value {
                        self.formatted(value);
                    }
                }
                HirFormPart::Text { .. } | HirFormPart::Triple { .. } => {}
            }
        }
    }
}

fn argument_name(argument: &HirArgument) -> Option<String> {
    match argument {
        HirArgument::Raw(value) => Some(value.clone()),
        HirArgument::Expression(value)
        | HirArgument::MixedExpression {
            expression: value, ..
        } => expression_name(value),
        HirArgument::Formatted(value) => formatted_name(value),
        HirArgument::Place(_) | HirArgument::Omitted => None,
    }
}

fn expression_name(expression: &HirExpr) -> Option<String> {
    match &expression.constant {
        Some(ConstantValue::String(value)) => Some(value.clone()),
        _ => match &expression.kind {
            HirExprKind::String { value } => Some(value.clone()),
            HirExprKind::Formatted { value } => formatted_name(value),
            _ => None,
        },
    }
}

fn formatted_name(value: &HirFormattedString) -> Option<String> {
    value
        .parts
        .iter()
        .map(|part| {
            if let HirFormPart::Text { value } = part {
                Some(value.as_str())
            } else {
                None
            }
        })
        .collect::<Option<Vec<_>>>()
        .map(|parts| parts.concat())
}
