use super::{
    can_restructure::ordinary_restructure_method,
    graph::{PreparedReferenceArguments, TermRef},
    invalid,
};
use crate::interpreter::StepError;
use crate::{ProgramGeneration, ScriptFaultKind, VmFaultCode};
use erabasic_bytecode::{
    BytecodeStorage, ReferenceTermCall, ReferenceTermId, ReferenceTermKind, ReferenceTermPart,
    ReferenceTermValue,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(in crate::interpreter::dynamic_form) enum RestructureTask {
    Visit {
        term: TermRef,
        reject_constant_index: bool,
    },
    Children(Children),
    CaptureChild(Children),
    CaptureFold,
    DiscardUniqueValue(ReferenceTermId),
    CheckFormPredicate(ReferenceTermId),
    CaptureRoot(usize),
    Publish,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(in crate::interpreter::dynamic_form) struct Children {
    pub node: ReferenceTermId,
    pub reject_constant_index: bool,
    pub visits: Vec<(usize, bool)>,
    pub next: usize,
    pub results: Vec<TermRef>,
}

impl PreparedReferenceArguments {
    pub(in crate::interpreter::dynamic_form) fn child_plan(
        &self,
        program: &ProgramGeneration,
        node: ReferenceTermId,
    ) -> Result<Vec<(usize, bool)>, StepError> {
        let value = &self.template.nodes[node as usize];
        let all = || {
            (0..self.edges[node as usize].len())
                .map(|index| (index, true))
                .collect()
        };
        let ReferenceTermKind::Call { target, arguments } = &value.kind else {
            return Ok(all());
        };
        let slots = reference_argument_edges(arguments);
        let selected = match target {
            ReferenceTermCall::User { key } => reference_user_children(program, *key, &slots)?,
            ReferenceTermCall::Native { name, .. }
            | ReferenceTermCall::DynamicNative { name, .. }
            | ReferenceTermCall::Host { name, .. }
            | ReferenceTermCall::Intrinsic { name }
                if name.eq_ignore_ascii_case("STRFORM") =>
            {
                slots
                    .first()
                    .copied()
                    .flatten()
                    .into_iter()
                    .map(|edge| (edge, false))
                    .collect()
            }
            ReferenceTermCall::Native { name, .. }
            | ReferenceTermCall::DynamicNative { name, .. }
            | ReferenceTermCall::Host { name, .. }
                if name.eq_ignore_ascii_case("VARSIZE") =>
            {
                slots.iter().flatten().map(|edge| (*edge, false)).collect()
            }
            ReferenceTermCall::Native { name, .. }
            | ReferenceTermCall::DynamicNative { name, .. }
            | ReferenceTermCall::Host { name, .. }
                if matches!(name.as_str(), "GETNUM" | "ERDNAME") =>
            {
                slots
                    .get(1)
                    .copied()
                    .flatten()
                    .into_iter()
                    .map(|edge| (edge, false))
                    .collect()
            }
            ReferenceTermCall::Native { name, .. }
            | ReferenceTermCall::DynamicNative { name, .. }
            | ReferenceTermCall::Host { name, .. }
                if matches!(name.as_str(), "FINDELEMENT" | "FINDLASTELEMENT" | "STRJOIN") =>
            {
                slots
                    .iter()
                    .enumerate()
                    .filter_map(|(slot, edge)| {
                        if slot == 0 && name == "STRJOIN" {
                            None
                        } else {
                            edge.map(|edge| (edge, false))
                        }
                    })
                    .collect()
            }
            ReferenceTermCall::Native { name, .. }
            | ReferenceTermCall::DynamicNative { name, .. }
            | ReferenceTermCall::Host { name, .. }
                if name == "REPLACE" =>
            {
                Vec::new()
            }
            ReferenceTermCall::Host { name, .. }
                if matches!(name.as_str(), "GDRAWG" | "GDRAWSPRITE") =>
            {
                let matrix_slot = if name == "GDRAWG" { 10 } else { 6 };
                slots
                    .iter()
                    .enumerate()
                    .filter_map(|(slot, edge)| edge.map(|edge| (edge, slot != matrix_slot)))
                    .collect()
            }
            ReferenceTermCall::Native { name, .. }
            | ReferenceTermCall::DynamicNative { name, .. }
            | ReferenceTermCall::Host { name, .. }
                if matches!(
                    name.as_str(),
                    "MATCH" | "CMATCH" | "ARRAYMSORT" | "ARRAYMSORTEX" | "GDRAWG" | "GDRAWSPRITE"
                ) =>
            {
                slots.iter().flatten().map(|edge| (*edge, false)).collect()
            }
            _ => all(),
        };
        Ok(selected)
    }

    /// Called only after every index has completed Restructure (VariableTerm.cs
    /// 302-335). An early constant bound failure must not skip later unique reads.
    pub(in crate::interpreter::dynamic_form) fn check_constant_indices(
        &self,
        program: &ProgramGeneration,
        node: ReferenceTermId,
    ) -> Result<(), StepError> {
        if self.constant_index_out_of_range(program, node)? {
            return Err(StepError::script(
                ScriptFaultKind::Bounds,
                VmFaultCode::Bounds,
                "reference argument constant variable index is out of range",
            ));
        }
        Ok(())
    }

    pub(in crate::interpreter::dynamic_form) fn constant_index_out_of_range(
        &self,
        program: &ProgramGeneration,
        node: ReferenceTermId,
    ) -> Result<bool, StepError> {
        Ok(self
            .first_constant_index_out_of_range(program, node)?
            .is_some())
    }

    fn first_constant_index_out_of_range(
        &self,
        program: &ProgramGeneration,
        node: ReferenceTermId,
    ) -> Result<Option<usize>, StepError> {
        for (edge, value) in self.edges[node as usize].iter().enumerate() {
            if self.constant_index_out_of_range_at(program, node, edge, value)? {
                return Ok(Some(edge));
            }
        }
        Ok(None)
    }

    pub(in crate::interpreter::dynamic_form) fn defer_constant_index_failure(
        &mut self,
        program: &ProgramGeneration,
        node: ReferenceTermId,
    ) -> Result<(), StepError> {
        let edge = self
            .first_constant_index_out_of_range(program, node)?
            .ok_or_else(|| invalid("deferred constant bound failure disappeared"))?;
        // ConvertArg evaluates index expressions before reading the variable. A
        // constant bound is rejected before later indices in the reference engine,
        // so replace only those later expressions with inert Integer values. The
        // original failing edge remains and follows the ordinary, catchable Bounds
        // path; earlier index effects retain source order.
        for value in self.edges[node as usize].iter_mut().skip(edge + 1) {
            *value = TermRef::Single(ReferenceTermValue::Integer(0));
        }
        Ok(())
    }

    fn constant_index_out_of_range_at(
        &self,
        program: &ProgramGeneration,
        node: ReferenceTermId,
        edge: usize,
        value: &TermRef,
    ) -> Result<bool, StepError> {
        let ReferenceTermKind::Variable { key, indices } = &self.template.nodes[node as usize].kind
        else {
            return Ok(false);
        };
        let Some(ReferenceTermValue::Integer(index)) = self.single(value) else {
            return Ok(false);
        };
        let definition = program
            .global(*key)
            .ok_or_else(|| invalid("reference argument variable schema disappeared"))?;
        let metadata = program
            .artifact
            .runtime_variables
            .iter()
            .find(|item| item.key == *key)
            .ok_or_else(|| invalid("reference argument variable metadata is missing"))?;
        if metadata.reference || matches!(definition.name.as_str(), "ARG" | "ARGS") {
            return Ok(false);
        }
        let character = definition.storage == BytecodeStorage::Character
            && indices.len() > definition.dimensions.len();
        if character && edge == 0 {
            return Ok(false);
        }
        let dimension = edge.saturating_sub(usize::from(character));
        if definition
            .dimensions
            .get(dimension)
            .is_none_or(|size| u64::try_from(*index).map_or(true, |index| index >= *size))
        {
            return Ok(true);
        }
        Ok(false)
    }

    pub(in crate::interpreter::dynamic_form) fn variable_const(
        &self,
        program: &ProgramGeneration,
        value: &TermRef,
    ) -> Result<bool, StepError> {
        let TermRef::Original(id) = value else {
            return Ok(false);
        };
        let ReferenceTermKind::Variable { key, .. } = self.template.nodes[*id as usize].kind else {
            return Ok(false);
        };
        Ok(program
            .artifact
            .runtime_variables
            .iter()
            .find(|item| item.key == key)
            .ok_or_else(|| invalid("reference argument variable metadata is missing"))?
            .reference_semantics
            .is_const)
    }

    pub(in crate::interpreter::dynamic_form) fn may_fold(
        &self,
        program: &ProgramGeneration,
        function: erabasic_bytecode::SymbolKey,
        node: ReferenceTermId,
        scratch: &[TermRef],
    ) -> Result<bool, StepError> {
        let value = &self.template.nodes[node as usize];
        let edges = &self.edges[node as usize];
        let all_single = || edges.iter().all(|value| self.single(value).is_some());
        Ok(match &value.kind {
            ReferenceTermKind::Variable { key, .. } => {
                let metadata = program
                    .artifact
                    .runtime_variables
                    .iter()
                    .find(|item| item.key == *key)
                    .ok_or_else(|| invalid("reference argument variable metadata is missing"))?;
                metadata.reference_semantics.can_restructure && all_single()
            }
            ReferenceTermKind::Unary { op, .. } => {
                !matches!(
                    op,
                    erabasic_ast::UnaryOp::PreIncrement | erabasic_ast::UnaryOp::PreDecrement
                ) && all_single()
            }
            ReferenceTermKind::Binary { .. } | ReferenceTermKind::Ternary { .. } => all_single(),
            ReferenceTermKind::Form { parts } => {
                !parts
                    .iter()
                    .any(|part| matches!(part, ReferenceTermPart::Triple(_)))
                    && all_single()
            }
            ReferenceTermKind::Value(_)
            | ReferenceTermKind::Postfix { .. }
            | ReferenceTermKind::Call {
                target: ReferenceTermCall::User { .. } | ReferenceTermCall::Intrinsic { .. },
                ..
            } => false,
            ReferenceTermKind::Call {
                target:
                    ReferenceTermCall::Native { name, .. }
                    | ReferenceTermCall::DynamicNative { name, .. }
                    | ReferenceTermCall::Host { name, .. },
                arguments,
            } => match name.as_str() {
                "STRFORM" => false, // Its first read and parse-only predicate are scheduled separately.
                "GETNUM" | "ERDNAME" => scratch
                    .first()
                    .is_some_and(|value| self.single(value).is_some()),
                "FINDELEMENT" | "FINDLASTELEMENT" => {
                    edges
                        .first()
                        .map(|value| self.variable_const(program, value))
                        .transpose()?
                        .unwrap_or(false)
                        && scratch
                            .iter()
                            .skip(1)
                            .all(|value| self.single(value).is_some())
                }
                "STRJOIN" => {
                    edges
                        .first()
                        .map(|value| self.variable_const(program, value))
                        .transpose()?
                        .unwrap_or(false)
                        && scratch.iter().all(|value| self.single(value).is_some())
                }
                "VARSIZE" => {
                    // Root replacements returned by Restructure were discarded.
                    let Some(TermRef::Original(first)) = edges.first() else {
                        return Ok(false);
                    };
                    let ReferenceTermKind::Value(ReferenceTermValue::String(name)) =
                        &self.template.nodes[*first as usize].kind
                    else {
                        return Ok(false);
                    };
                    let original_single = arguments.len() == 1
                        || (arguments.len() == 2
                            && arguments[1].node.is_some_and(|id| {
                                matches!(
                                    self.template.nodes[id as usize].kind,
                                    ReferenceTermKind::Value(_)
                                )
                            }));
                    let definition = program.scoped_variable(function, name);
                    original_single
                        && definition.is_some_and(|definition| {
                            program
                                .artifact
                                .runtime_variables
                                .iter()
                                .any(|item| item.key == definition.key && !item.reference)
                        })
                }
                _ => ordinary_restructure_method(name) && all_single(),
            },
        })
    }
}

fn reference_user_children(
    program: &ProgramGeneration,
    key: erabasic_bytecode::SymbolKey,
    slots: &[Option<usize>],
) -> Result<Vec<(usize, bool)>, StepError> {
    let function = program
        .function(key)
        .ok_or_else(|| invalid("reference argument method metadata disappeared"))?;
    Ok(slots
        .iter()
        .take(function.parameters.len())
        .enumerate()
        .filter_map(|(slot, edge)| {
            edge.map(|edge| {
                (
                    edge,
                    !function
                        .parameters
                        .get(slot)
                        .is_some_and(|arg| arg.by_reference),
                )
            })
        })
        .collect())
}

fn reference_argument_edges(
    arguments: &[erabasic_bytecode::ReferenceTermArgument],
) -> Vec<Option<usize>> {
    // Omitted slots have no edge and are not visited. Keep slot->edge mapping.
    let mut edge = 0;
    arguments
        .iter()
        .map(|arg| {
            arg.node.map(|_| {
                let value = edge;
                edge += 1;
                value
            })
        })
        .collect()
}
