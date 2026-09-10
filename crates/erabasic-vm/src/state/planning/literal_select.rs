use super::{BytecodeFunction, Opcode, StructuredScopeKind, StructuredScopeRange, VmValue};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub(crate) struct SelectTarget {
    pub instruction: usize,
    pub additional_instructions: u64,
    pub peak_extra: usize,
}

#[derive(Clone, Debug)]
enum Labels {
    Integers(HashMap<i64, SelectTarget>),
    Strings(HashMap<String, SelectTarget>),
}

#[derive(Clone, Debug)]
pub(crate) struct LiteralSelectPlan {
    labels: Labels,
    miss: SelectTarget,
    pub loops: usize,
    pub outer_selects: usize,
}

impl LiteralSelectPlan {
    pub(crate) fn target(&self, selector: &VmValue) -> Option<SelectTarget> {
        Some(match (&self.labels, selector) {
            (Labels::Integers(labels), VmValue::Integer(value)) => {
                labels.get(value).copied().unwrap_or(self.miss)
            }
            (Labels::Strings(labels), VmValue::String(value)) => labels
                .get::<str>(value.as_ref())
                .copied()
                .unwrap_or(self.miss),
            _ => return None,
        })
    }
}

// Optional generation-local metadata. Charge temporary labels, hash capacity and
// strings conservatively, including rejected plans. Exhaustion only disables plans.
pub(in crate::state) struct SelectPlanBudget {
    bytes: usize,
    work: usize,
}

impl Default for SelectPlanBudget {
    fn default() -> Self {
        Self {
            bytes: 64 * 1024 * 1024,
            work: 32 * 1024 * 1024,
        }
    }
}

impl SelectPlanBudget {
    pub(in crate::state) fn reserve_directory(&mut self, functions: usize) -> bool {
        functions
            .checked_mul(48)
            .and_then(|bytes| self.charge(bytes, 0))
            .is_some()
    }

    fn charge(&mut self, bytes: usize, work: usize) -> Option<()> {
        self.bytes = self.bytes.checked_sub(bytes)?;
        self.work = self.work.checked_sub(work)?;
        Some(())
    }
}

enum Literal {
    Integer(i64),
    String(String),
}

fn opcode(function: &BytecodeFunction, pc: usize, budget: &mut SelectPlanBudget) -> Option<Opcode> {
    budget.charge(0, 1)?;
    Opcode::try_from(function.code.get(pc)?.opcode).ok()
}

fn literal(
    function: &BytecodeFunction,
    pc: usize,
    budget: &mut SelectPlanBudget,
) -> Option<Literal> {
    let encoded = function.code.get(pc)?;
    budget.charge(256, encoded.payload.len())?;
    match opcode(function, pc, budget)? {
        Opcode::PushInteger => Some(Literal::Integer(i64::from_le_bytes(
            encoded.payload.as_ref().try_into().ok()?,
        ))),
        Opcode::PushString => {
            let length = u32::from_le_bytes(encoded.payload.get(..4)?.try_into().ok()?) as usize;
            let bytes = encoded.payload.get(4..)?;
            if bytes.len() != length {
                return None;
            }
            budget.charge(length.checked_mul(2)?, 0)?;
            Some(Literal::String(std::str::from_utf8(bytes).ok()?.to_owned()))
        }
        _ => None,
    }
}

fn same_scopes(
    ranges: &[StructuredScopeRange],
    origin: usize,
    pc: usize,
    budget: &mut SelectPlanBudget,
) -> Option<()> {
    budget.charge(0, ranges.len())?;
    ranges
        .iter()
        .all(|range| {
            (range.start <= origin && origin <= range.end) == (range.start <= pc && pc <= range.end)
        })
        .then_some(())
}

pub(in crate::state) fn plans(
    function: &BytecodeFunction,
    ranges: &[StructuredScopeRange],
    budget: &mut SelectPlanBudget,
) -> Vec<(u32, LiteralSelectPlan)> {
    let mut plans = Vec::new();
    for range in ranges {
        if budget.charge(0, 1).is_none() {
            break;
        }
        if range.kind == StructuredScopeKind::Select
            && let Some(plan) = plan(function, ranges, *range, budget)
            && let Ok(pc) = u32::try_from(range.opener)
        {
            plans.push((pc, plan));
        }
    }
    plans.sort_by_key(|(pc, _)| *pc);
    plans
}

fn insert(labels: &mut Option<Labels>, values: Vec<Literal>, target: SelectTarget) -> Option<()> {
    for value in values {
        match value {
            Literal::Integer(value) => {
                let labels = labels.get_or_insert_with(|| Labels::Integers(HashMap::new()));
                let Labels::Integers(labels) = labels else {
                    return None;
                };
                labels.entry(value).or_insert(target);
            }
            Literal::String(value) => {
                let labels = labels.get_or_insert_with(|| Labels::Strings(HashMap::new()));
                let Labels::Strings(labels) = labels else {
                    return None;
                };
                labels.entry(value).or_insert(target);
            }
        }
    }
    Some(())
}

fn case_group(
    function: &BytecodeFunction,
    pc: &mut usize,
    budget: &mut SelectPlanBudget,
) -> Option<Vec<Literal>> {
    let mut values = Vec::new();
    loop {
        values.push(literal(function, *pc, budget)?);
        *pc += 1;
        if opcode(function, *pc, budget)? != Opcode::SelectCompare
            || function.code[*pc].payload.as_ref() != [0]
        {
            return None;
        }
        *pc += 1;
        if values.len() > 1 {
            if opcode(function, *pc, budget)? != Opcode::Binary
                || function.code[*pc].payload.as_ref() != [18]
            {
                return None;
            }
            *pc += 1;
        }
        if opcode(function, *pc, budget)? == Opcode::JumpIfFalse {
            return Some(values);
        }
    }
}

fn plan(
    function: &BytecodeFunction,
    ranges: &[StructuredScopeRange],
    range: StructuredScopeRange,
    budget: &mut SelectPlanBudget,
) -> Option<LiteralSelectPlan> {
    budget.charge(512, ranges.len())?;
    if !function.code.get(range.opener)?.payload.is_empty() {
        return None;
    }
    let mut loops = 0;
    let mut outer_selects = 0;
    for scope in ranges {
        if scope.start <= range.opener && range.opener <= scope.end {
            match scope.kind {
                StructuredScopeKind::Loop => loops += 1,
                StructuredScopeKind::Select => outer_selects += 1,
            }
        }
    }
    let mut pc = range.start;
    let mut cost = 0_u64;
    let mut peak_extra = 0;
    let mut label_count = 0;
    let mut labels = None;
    let miss = loop {
        same_scopes(ranges, range.start, pc, budget)?;
        if pc == range.end {
            break SelectTarget {
                instruction: pc,
                additional_instructions: cost,
                peak_extra,
            };
        }
        let first = pc;
        let is_else = opcode(function, pc, budget)? == Opcode::SelectCompare
            && function.code[pc].payload.as_ref() == [8];
        let values = if is_else {
            pc += 1;
            Vec::new()
        } else {
            case_group(function, &mut pc, budget)?
        };
        if pc >= range.end || opcode(function, pc, budget)? != Opcode::JumpIfFalse {
            return None;
        }
        let next = u32::from_le_bytes(function.code[pc].payload.as_ref().try_into().ok()?) as usize;
        if next <= pc || next > range.end {
            return None;
        }
        // Skipped jumps must neither enter nor leave a structured scope. Runtime
        // depth guards additionally make their truncation of scope stacks a no-op.
        same_scopes(ranges, range.start, pc, budget)?;
        same_scopes(ranges, range.start, pc + 1, budget)?;
        same_scopes(ranges, range.start, next, budget)?;
        peak_extra = peak_extra.max(usize::from(values.len() > 1));
        cost = cost.checked_add(u64::try_from(pc - first + 1).ok()?)?;
        let target = SelectTarget {
            instruction: pc + 1,
            additional_instructions: cost,
            peak_extra,
        };
        if is_else {
            break target;
        }
        label_count += values.len();
        insert(&mut labels, values, target)?;
        pc = next;
    };
    if label_count < 4 {
        return None;
    }
    Some(LiteralSelectPlan {
        labels: labels?,
        miss,
        loops,
        outer_selects,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_select_budget_exhaustion_disables_optional_plans() {
        let artifact = crate::interpreter::literal_groupmatch_tests::compile_cursor_fixture(
            "@SYSTEM_TITLE\nSELECTCASE 4\nCASE 1\nRETURN 1\nCASE 2\nRETURN 2\nCASE 3\nRETURN 3\nCASE 4\nRETURN 4\nENDSELECT\nRETURN\n".into(),
        );
        let function = &artifact.functions[0];
        let ranges = super::super::structured_scope_ranges(function);
        for (bytes, work) in [(0, usize::MAX), (usize::MAX, 0), (1024, 1024)] {
            assert!(plans(function, &ranges, &mut SelectPlanBudget { bytes, work }).is_empty());
        }
        assert_eq!(
            plans(function, &ranges, &mut SelectPlanBudget::default()).len(),
            1
        );
        let mut budget = SelectPlanBudget::default();
        assert!(!budget.reserve_directory(usize::MAX));
    }
}
