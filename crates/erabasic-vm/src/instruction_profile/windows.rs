//! Sample short, actually attempted straight-line dispatch sequences, not CPU time.
use std::collections::BTreeMap;

use erabasic_bytecode::{Opcode, SymbolKey};
use serde::Serialize;

use crate::{FiberId, FrameId, GenerationId};

const MAXIMUM_LENGTH: usize = 8;
const MAXIMUM_PATTERNS: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DispatchContext {
    pub fiber: FiberId,
    pub frame: FrameId,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Location {
    pub generation: GenerationId,
    pub function: SymbolKey,
    pub instruction: usize,
    pub context: DispatchContext,
}

impl Location {
    fn follows(self, previous: Self) -> bool {
        self.generation == previous.generation
            && self.function == previous.function
            && self.context == previous.context
            && previous.instruction.checked_add(1) == Some(self.instruction)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
enum Termination {
    LengthLimit,
    Discontinuous,
    SliceBoundary,
    ControlBoundary,
    Diagnostic,
    Fault,
    ActionEnd,
    Overflow,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Pattern {
    opcodes: [u16; MAXIMUM_LENGTH],
    length: usize,
    termination: Termination,
}

#[derive(Debug)]
struct Pending {
    previous: Location,
    opcodes: [u16; MAXIMUM_LENGTH],
    length: usize,
}

#[derive(Debug, Default)]
pub(super) struct WindowProfile {
    active: bool,
    started_at: u64,
    ended_at: u64,
    incomplete: bool,
    restarted_while_active: bool,
    opportunities: u64,
    started_windows: u64,
    excluded_continuations: u64,
    dropped_windows: u64,
    counts: BTreeMap<Pattern, u64>,
    pending: Option<Pending>,
}

fn increment(value: &mut u64, incomplete: &mut bool) {
    *incomplete |= *value == u64::MAX;
    *value = value.saturating_add(1);
}

impl WindowProfile {
    pub(super) fn boundary(&mut self, begin: bool, dispatches: u64) {
        if begin {
            let restarted = self.active;
            *self = Self {
                active: true,
                started_at: dispatches,
                ended_at: dispatches,
                incomplete: restarted,
                restarted_while_active: restarted,
                ..Self::default()
            };
        } else if self.active {
            self.flush(Termination::ActionEnd);
            self.active = false;
            self.ended_at = dispatches;
        }
    }

    pub(super) fn mark_incomplete(&mut self) {
        if self.active {
            self.incomplete = true;
            self.flush(Termination::Overflow);
        }
    }

    pub(super) fn observe(&mut self, dispatches: u64, location: Option<Location>, opcode: u16) {
        if !self.active {
            return;
        }
        if let Some(pending) = &mut self.pending {
            if let Some(location) = location.filter(|location| location.follows(pending.previous)) {
                pending.opcodes[pending.length] = opcode;
                pending.length += 1;
                pending.previous = location;
                // This limit counts attempts. The eighth opcode may subsequently
                // fault; length_limit never claims eight successful operations.
                if pending.length == MAXIMUM_LENGTH {
                    self.flush(Termination::LengthLimit);
                }
            } else {
                self.flush(Termination::Discontinuous);
            }
        }
        if !dispatches.is_multiple_of(super::INTERVAL) {
            return;
        }
        increment(&mut self.opportunities, &mut self.incomplete);
        let Some(location) = location else {
            increment(&mut self.excluded_continuations, &mut self.incomplete);
            return;
        };
        debug_assert!(
            self.pending.is_none(),
            "window shorter than sampling interval"
        );
        increment(&mut self.started_windows, &mut self.incomplete);
        let mut opcodes = [0; MAXIMUM_LENGTH];
        opcodes[0] = opcode;
        self.pending = Some(Pending {
            previous: location,
            opcodes,
            length: 1,
        });
    }

    fn flush(&mut self, termination: Termination) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        let pattern = Pattern {
            opcodes: pending.opcodes,
            length: pending.length,
            termination,
        };
        if let Some(count) = self.counts.get_mut(&pattern) {
            increment(count, &mut self.incomplete);
        } else if self.counts.len() < MAXIMUM_PATTERNS {
            self.counts.insert(pattern, 1);
        } else {
            self.incomplete = true;
            increment(&mut self.dropped_windows, &mut self.incomplete);
        }
    }

    pub(super) fn finish_dispatch(
        &mut self,
        opcode: u16,
        ordinary: bool,
        failed: bool,
        diagnostic: bool,
    ) {
        if self.pending.is_none() {
            return;
        }
        let reason = if failed {
            Some(Termination::Fault)
        } else if diagnostic {
            Some(Termination::Diagnostic)
        } else if !ordinary || !straight_line(opcode) {
            Some(Termination::ControlBoundary)
        } else {
            None
        };
        if let Some(reason) = reason {
            self.flush(reason);
        }
    }

    pub(super) fn finish_slice(&mut self) {
        self.flush(Termination::SliceBoundary);
    }

    pub(super) fn snapshot(&self, dispatches: u64) -> WindowSnapshot {
        WindowSnapshot {
            active: self.active,
            started_at_dispatches: self.started_at.to_string(),
            ended_at_dispatches: if self.active {
                dispatches
            } else {
                self.ended_at
            }
            .to_string(),
            maximum_length: MAXIMUM_LENGTH,
            maximum_patterns: MAXIMUM_PATTERNS,
            incomplete: self.incomplete,
            restarted_while_active: self.restarted_while_active,
            opportunities: self.opportunities.to_string(),
            started_windows: self.started_windows.to_string(),
            excluded_continuations: self.excluded_continuations.to_string(),
            dropped_windows: self.dropped_windows.to_string(),
            counts: self
                .counts
                .iter()
                .map(|(pattern, count)| WindowCount {
                    opcodes: pattern.opcodes[..pattern.length].to_vec(),
                    termination: pattern.termination,
                    samples: count.to_string(),
                })
                .collect(),
            pending: self.pending.as_ref().map(|pending| PendingSnapshot {
                opcodes: pending.opcodes[..pending.length].to_vec(),
            }),
        }
    }
}

fn straight_line(opcode: u16) -> bool {
    matches!(
        Opcode::try_from(opcode),
        Ok(Opcode::Nop
            | Opcode::PushInteger
            | Opcode::PushString
            | Opcode::LoadVariable
            | Opcode::StoreVariable
            | Opcode::MakePlace
            | Opcode::Unary
            | Opcode::Binary
            | Opcode::ToString
            | Opcode::Pop
            | Opcode::Dup
            | Opcode::StorePlace
            | Opcode::Concat)
    )
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WindowSnapshot {
    active: bool,
    started_at_dispatches: String,
    ended_at_dispatches: String,
    maximum_length: usize,
    maximum_patterns: usize,
    incomplete: bool,
    restarted_while_active: bool,
    opportunities: String,
    started_windows: String,
    excluded_continuations: String,
    dropped_windows: String,
    counts: Vec<WindowCount>,
    pending: Option<PendingSnapshot>,
}

#[derive(Debug, Serialize)]
struct WindowCount {
    opcodes: Vec<u16>,
    termination: Termination,
    samples: String,
}

#[derive(Debug, Serialize)]
struct PendingSnapshot {
    opcodes: Vec<u16>,
}

#[cfg(test)]
pub(super) mod tests;
