//! Immutable static edges avoid re-scanning every structured scope on loop branches.
use super::{
    BytecodeFunction, Opcode, StaticStructuredJump, StructuredJumpTransition, StructuredScopeKind,
    StructuredScopeRange,
};

// Optional acceleration only: once full, ordinary transition calculation remains authoritative.
pub(super) const MAXIMUM_STATIC_JUMP_BYTES: usize = 16 * 1024 * 1024;
pub(super) const MAXIMUM_STATIC_JUMP_WORK: usize = 32 * 1024 * 1024;

pub(super) fn allocate_directory(
    functions: usize,
    remaining: &mut usize,
) -> Vec<Vec<(u32, StaticStructuredJump)>> {
    let bytes = functions.saturating_mul(std::mem::size_of::<Vec<(u32, StaticStructuredJump)>>());
    if bytes > *remaining {
        return Vec::new();
    }
    *remaining -= bytes;
    Vec::with_capacity(functions)
}

pub(super) fn transition(
    ranges: &[StructuredScopeRange],
    source: usize,
    target: usize,
) -> StructuredJumpTransition {
    let (loops, selects, first) = transition_parts(ranges, source, target);
    collect_transition(ranges, target, loops, selects, first)
}

fn transition_parts(
    ranges: &[StructuredScopeRange],
    source: usize,
    target: usize,
) -> (usize, usize, Option<usize>) {
    let mut source_ranges = ranges
        .iter()
        .filter(|range| range.start <= source && source <= range.end);
    let mut target_ranges = ranges
        .iter()
        .enumerate()
        .filter(|(_, range)| range.start <= target && target <= range.end);
    let mut retain_loops = 0;
    let mut retain_selects = 0;
    let entered = loop {
        match (source_ranges.next(), target_ranges.next()) {
            (Some(left), Some((_, right)))
                if left.kind == right.kind && left.opener == right.opener =>
            {
                match right.kind {
                    StructuredScopeKind::Loop => retain_loops += 1,
                    StructuredScopeKind::Select => retain_selects += 1,
                }
            }
            (_, Some((first, _))) => break Some(first),
            (_, None) => break None,
        }
    };
    (retain_loops, retain_selects, entered)
}

fn entered_ranges(
    ranges: &[StructuredScopeRange],
    target: usize,
    first: Option<usize>,
) -> impl Iterator<Item = &StructuredScopeRange> {
    ranges[first.unwrap_or(ranges.len())..]
        .iter()
        .filter(move |range| range.start <= target && target <= range.end)
}

fn collect_transition(
    ranges: &[StructuredScopeRange],
    target: usize,
    retain_loops: usize,
    retain_selects: usize,
    first: Option<usize>,
) -> StructuredJumpTransition {
    let count = entered_ranges(ranges, target, first).count();
    let mut entered = Vec::with_capacity(count);
    entered.extend(entered_ranges(ranges, target, first).map(|range| range.kind));
    StructuredJumpTransition {
        retain_loops,
        retain_selects,
        entered,
    }
}

pub(super) fn plan_static_jumps(
    function: &BytecodeFunction,
    ranges: &[StructuredScopeRange],
    remaining_bytes: &mut usize,
    remaining_work: &mut usize,
) -> Vec<(u32, StaticStructuredJump)> {
    let work = ranges.len().saturating_mul(5);
    if ranges.is_empty() || work > *remaining_work {
        return Vec::new();
    }
    let entries = function
        .code
        .iter()
        .filter(|encoded| {
            matches!(
                Opcode::try_from(encoded.opcode),
                Ok(Opcode::Jump | Opcode::JumpIfFalse)
            )
        })
        .count()
        .min(*remaining_bytes / std::mem::size_of::<(u32, StaticStructuredJump)>());
    // Charge capacity, including unused entries; never grow this allocation.
    let mut plans = Vec::with_capacity(entries);
    *remaining_bytes -= plans.capacity() * std::mem::size_of::<(u32, StaticStructuredJump)>();
    for (source, encoded) in function.code.iter().enumerate() {
        if plans.len() == entries || work > *remaining_work {
            break;
        }
        if !matches!(
            Opcode::try_from(encoded.opcode),
            Ok(Opcode::Jump | Opcode::JumpIfFalse)
        ) {
            continue;
        }
        let Ok(bytes) = <[u8; 4]>::try_from(encoded.payload.as_ref()) else {
            continue;
        };
        let Ok(source_index) = u32::try_from(source) else {
            break;
        };
        let target = u32::from_le_bytes(bytes) as usize;
        *remaining_work -= work;
        let (loops, selects, first) = transition_parts(ranges, source, target);
        let bytes = entered_ranges(ranges, target, first)
            .count()
            .saturating_mul(std::mem::size_of::<StructuredScopeKind>());
        if bytes > *remaining_bytes {
            continue;
        }
        *remaining_bytes -= bytes;
        let transition = collect_transition(ranges, target, loops, selects, first);
        plans.push((source_index, StaticStructuredJump { target, transition }));
    }
    plans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_transition_preserves_nested_siblings_and_bypassed_entries() {
        let ranges = [
            StructuredScopeRange {
                kind: StructuredScopeKind::Loop,
                opener: 0,
                start: 1,
                end: 20,
            },
            StructuredScopeRange {
                kind: StructuredScopeKind::Select,
                opener: 2,
                start: 3,
                end: 8,
            },
            StructuredScopeRange {
                kind: StructuredScopeKind::Loop,
                opener: 10,
                start: 11,
                end: 15,
            },
        ];
        assert_eq!(
            transition(&ranges, 4, 5),
            StructuredJumpTransition {
                retain_loops: 1,
                retain_selects: 1,
                entered: Vec::new(),
            }
        );
        assert_eq!(
            transition(&ranges, 4, 12),
            StructuredJumpTransition {
                retain_loops: 1,
                retain_selects: 0,
                entered: vec![StructuredScopeKind::Loop],
            }
        );
        assert_eq!(
            transition(&ranges, 21, 4),
            StructuredJumpTransition {
                retain_loops: 0,
                retain_selects: 0,
                entered: vec![StructuredScopeKind::Loop, StructuredScopeKind::Select],
            }
        );
        assert_eq!(
            transition(&ranges, 12, 21),
            StructuredJumpTransition {
                retain_loops: 0,
                retain_selects: 0,
                entered: Vec::new(),
            }
        );
    }
}
