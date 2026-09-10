//! Opt-in dispatch-frequency diagnosis, absent from ordinary builds and snapshots.
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use erabasic_bytecode::SymbolKey;
use serde::Serialize;

use crate::{GenerationId, Vm};

mod opcodes;
use opcodes::{OpcodeProfile, OpcodeSnapshot};

mod positions;
use positions::{PositionProfile, PositionSnapshot};
mod windows;
pub(crate) use windows::DispatchContext;
use windows::{WindowProfile, WindowSnapshot};

const INTERVAL: u64 = 1024;
const MAXIMUM_FUNCTIONS: usize = 4096;
static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub(crate) struct InstructionProfile {
    instance: u64,
    dispatches: u64,
    dropped_samples: u64,
    incomplete: bool,
    counts: BTreeMap<(GenerationId, SymbolKey), u64>,
    positions: PositionProfile,
    opcodes: OpcodeProfile,
    windows: WindowProfile,
}

impl Default for InstructionProfile {
    fn default() -> Self {
        Self {
            instance: allocate_instance(&NEXT_INSTANCE),
            dispatches: 0,
            dropped_samples: 0,
            incomplete: false,
            counts: BTreeMap::new(),
            positions: PositionProfile::default(),
            opcodes: OpcodeProfile::default(),
            windows: WindowProfile::default(),
        }
    }
}

fn allocate_instance(counter: &AtomicU64) -> u64 {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .unwrap_or(0)
}

// Candidate/fork execution must not inherit or mutate the live VM's counters.
impl Clone for InstructionProfile {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl InstructionProfile {
    #[cfg(test)]
    fn observe(
        &mut self,
        generation: GenerationId,
        function: SymbolKey,
        instruction: usize,
        opcode: u16,
    ) {
        self.observe_dispatch(generation, function, instruction, opcode, None);
    }

    #[inline]
    pub(crate) fn observe_dispatch(
        &mut self,
        generation: GenerationId,
        function: SymbolKey,
        instruction: usize,
        opcode: u16,
        context: Option<DispatchContext>,
    ) {
        let Some(next) = self.dispatches.checked_add(1) else {
            self.incomplete = true;
            self.positions.mark_incomplete();
            self.opcodes.mark_incomplete();
            self.windows.mark_incomplete();
            return;
        };
        self.dispatches = next;
        if self.dispatches.is_multiple_of(INTERVAL) {
            self.opcodes.sample(opcode);
            self.sample(generation, function);
            self.positions.sample(generation, function, instruction);
        }
        self.windows.observe(
            self.dispatches,
            context.map(|context| windows::Location {
                generation,
                function,
                instruction,
                context,
            }),
            opcode,
        );
    }

    pub(crate) fn finish_dispatch(
        &mut self,
        opcode: u16,
        ordinary: bool,
        failed: bool,
        diagnostic: bool,
    ) {
        self.windows
            .finish_dispatch(opcode, ordinary, failed, diagnostic);
    }

    pub(crate) fn finish_slice(&mut self) {
        self.windows.finish_slice();
    }

    #[cold]
    fn sample(&mut self, generation: GenerationId, function: SymbolKey) {
        let key = (generation, function);
        if let Some(count) = self.counts.get_mut(&key) {
            self.incomplete |= *count == u64::MAX;
            *count = count.saturating_add(1);
        } else if self.counts.len() < MAXIMUM_FUNCTIONS {
            self.counts.insert(key, 1);
        } else {
            self.incomplete = true;
            self.dropped_samples = self.dropped_samples.saturating_add(1);
        }
    }
}

/// Cumulative, read-only diagnostic counters. Frequency is not CPU time; memo/bulk
/// logical instructions are deliberately not expanded into physical dispatch samples.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstructionProfileSnapshot {
    schema_version: u32,
    instance: String,
    interval: u64,
    dispatches: String,
    dropped_samples: String,
    incomplete: bool,
    counts: Vec<FunctionCount>,
    symbols: Vec<FunctionSymbol>,
    positions: PositionSnapshot,
    opcodes: OpcodeSnapshot,
    dispatch_windows: WindowSnapshot,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FunctionCount {
    generation: String,
    function: SymbolKey,
    samples: String,
}

#[derive(Debug, Serialize)]
struct FunctionSymbol {
    generation: String,
    function: SymbolKey,
    name: String,
}

impl Vm {
    /// Start a fresh position window or stop it, without changing game state.
    /// Call only at diagnostic action boundaries, outside any latency clock.
    pub fn instruction_profile_boundary(&mut self, begin: bool) {
        self.instruction_profile
            .positions
            .boundary(begin, self.instruction_profile.dispatches);
        self.instruction_profile
            .windows
            .boundary(begin, self.instruction_profile.dispatches);
    }

    /// Inspect bounded counters outside the measured action. No game state is changed.
    #[must_use]
    pub fn instruction_profile_snapshot(&self) -> InstructionProfileSnapshot {
        let profile = &self.instruction_profile;
        let counts = profile
            .counts
            .iter()
            .map(|(&(generation, function), count)| FunctionCount {
                generation: generation.0.to_string(),
                function,
                samples: count.to_string(),
            })
            .collect();
        let mut ranked: Vec<_> = profile.counts.iter().collect();
        ranked.sort_by_key(|(key, count)| (std::cmp::Reverse(**count), **key));
        let symbols = ranked
            .into_iter()
            .take(128)
            .map(|(&(generation, function), _)| {
                let name = self
                    .generations
                    .get(&generation)
                    .and_then(|program| program.function(function))
                    .map_or("<reclaimed generation>", |function| function.name.as_str());
                FunctionSymbol {
                    generation: generation.0.to_string(),
                    function,
                    name: name.chars().take(64).collect(),
                }
            })
            .collect();
        InstructionProfileSnapshot {
            schema_version: 2,
            instance: profile.instance.to_string(),
            interval: INTERVAL,
            dispatches: profile.dispatches.to_string(),
            dropped_samples: profile.dropped_samples.to_string(),
            incomplete: profile.incomplete || profile.instance == 0,
            counts,
            symbols,
            positions: profile.positions.snapshot(self),
            opcodes: profile.opcodes.snapshot(),
            dispatch_windows: profile.windows.snapshot(profile.dispatches),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instruction_profile_overflow_is_explicit_without_reusing_identity() {
        let counter = AtomicU64::new(u64::MAX - 1);
        assert_eq!(allocate_instance(&counter), u64::MAX - 1);
        assert_eq!(allocate_instance(&counter), 0);
        assert_eq!(allocate_instance(&counter), 0);
        let key = SymbolKey::derive("profile.test", b"overflow");
        let mut profile = InstructionProfile {
            dispatches: u64::MAX,
            ..Default::default()
        };
        profile.observe(GenerationId(1), key, 0, 0);
        assert!(profile.incomplete);
        assert!(profile.counts.is_empty());
        profile.incomplete = false;
        profile.counts.insert((GenerationId(1), key), u64::MAX);
        profile.sample(GenerationId(1), key);
        assert!(profile.incomplete);
    }

    #[test]
    fn instruction_profile_worst_serialization_fits_record_budget() {
        let counts = (0..MAXIMUM_FUNCTIONS)
            .map(|index| FunctionCount {
                generation: u64::MAX.to_string(),
                function: SymbolKey::derive("profile.test", &index.to_le_bytes()),
                samples: u64::MAX.to_string(),
            })
            .collect();
        let symbols = (0..128usize)
            .map(|index| FunctionSymbol {
                generation: u64::MAX.to_string(),
                function: SymbolKey::derive("profile.test", &index.to_le_bytes()),
                name: "\u{0000}".repeat(64),
            })
            .collect();
        let snapshot = InstructionProfileSnapshot {
            schema_version: 2,
            instance: u64::MAX.to_string(),
            interval: INTERVAL,
            dispatches: u64::MAX.to_string(),
            dropped_samples: u64::MAX.to_string(),
            incomplete: true,
            counts,
            symbols,
            positions: positions::tests::maximum_snapshot(),
            opcodes: opcodes::tests::maximum_snapshot(),
            dispatch_windows: windows::tests::maximum_snapshot(),
        };
        // Leave room for the bounded JSONL boundary envelope.
        assert!(serde_json::to_vec(&snapshot).unwrap().len() < 32 * 1024 * 1024 - 1024);
    }

    #[test]
    fn instruction_profile_samples_physical_dispatches_across_intervals() {
        let mut profile = InstructionProfile::default();
        let key = SymbolKey::derive("profile.test", b"first");
        for _ in 0..1023 {
            profile.observe(GenerationId(1), key, 0, 0);
        }
        assert!(profile.counts.is_empty());
        profile.observe(GenerationId(1), key, 0, 0);
        assert_eq!(profile.counts[&(GenerationId(1), key)], 1);
        for _ in 0..1024 {
            profile.observe(GenerationId(2), key, 0, 0);
        }
        assert_eq!(profile.counts[&(GenerationId(2), key)], 1);
        assert_eq!(profile.dispatches, 2048);
        let clone = profile.clone();
        assert_ne!(clone.instance, profile.instance);
        assert_eq!(clone.dispatches, 0);
        assert!(clone.counts.is_empty());
    }

    #[test]
    fn instruction_profile_is_bounded_and_reports_lost_new_keys() {
        let mut profile = InstructionProfile::default();
        for index in 0..MAXIMUM_FUNCTIONS {
            profile.sample(
                GenerationId(1),
                SymbolKey::derive("profile.test", &index.to_le_bytes()),
            );
        }
        let extra = SymbolKey::derive("profile.test", b"extra");
        profile.sample(GenerationId(1), extra);
        assert_eq!(profile.counts.len(), MAXIMUM_FUNCTIONS);
        assert_eq!(profile.dropped_samples, 1);
        let existing = SymbolKey::derive("profile.test", &0usize.to_le_bytes());
        profile.sample(GenerationId(1), existing);
        assert_eq!(profile.counts[&(GenerationId(1), existing)], 2);
    }
    #[test]
    fn opcode_samples_share_dispatch_phase_and_ignore_position_window_boundaries() {
        let mut profile = InstructionProfile::default();
        let key = SymbolKey::derive("profile.test", b"opcode-phase");
        for _ in 0..INTERVAL - 1 {
            profile.observe(GenerationId(1), key, 0, 1);
        }
        assert!(
            serde_json::to_value(profile.opcodes.snapshot()).unwrap()["counts"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        profile.positions.boundary(true, profile.dispatches);
        profile.observe(GenerationId(1), key, 0, 3);
        profile.positions.boundary(false, profile.dispatches);
        for _ in 0..INTERVAL {
            profile.observe(GenerationId(1), key, 0, 4);
        }
        let snapshot = serde_json::to_value(profile.opcodes.snapshot()).unwrap();
        assert_eq!(
            snapshot["counts"],
            serde_json::json!([
                {"opcode": 3, "samples": "1"}, {"opcode": 4, "samples": "1"}
            ])
        );
        let clone = profile.clone();
        assert_ne!(clone.instance, profile.instance);
        assert!(
            serde_json::to_value(clone.opcodes.snapshot()).unwrap()["counts"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        profile.dispatches = u64::MAX;
        profile.observe(GenerationId(1), key, 0, 5);
        assert!(
            serde_json::to_value(profile.opcodes.snapshot()).unwrap()["incomplete"]
                .as_bool()
                .unwrap()
        );
    }

    #[test]
    fn function_capacity_loss_does_not_truncate_opcode_samples() {
        let mut profile = InstructionProfile::default();
        for index in 0..MAXIMUM_FUNCTIONS {
            profile.sample(
                GenerationId(1),
                SymbolKey::derive("profile", &index.to_le_bytes()),
            );
            profile.opcodes.sample(1);
        }
        profile.dispatches = (MAXIMUM_FUNCTIONS as u64 + 1) * INTERVAL - 1;
        profile.observe(
            GenerationId(1),
            SymbolKey::derive("profile", b"extra"),
            0,
            3,
        );
        assert!(profile.incomplete);
        assert_eq!(profile.dropped_samples, 1);
        let opcodes = serde_json::to_value(profile.opcodes.snapshot()).unwrap();
        assert_eq!(opcodes["incomplete"], false);
        assert_eq!(opcodes["droppedSamples"], "0");
        assert_eq!(
            opcodes["counts"][1],
            serde_json::json!({"opcode": 3, "samples": "1"})
        );
        let mut independent = InstructionProfile::default();
        independent.positions.boundary(true, 0);
        independent.opcodes.mark_incomplete();
        assert!(!independent.incomplete);
    }
}
