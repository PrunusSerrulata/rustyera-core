//! Bounded per-action position frequencies; source projection is outside execution.
use std::collections::BTreeMap;

use erabasic_bytecode::SymbolKey;
use serde::Serialize;

use crate::{GenerationId, Vm};

const MAXIMUM_POSITIONS: usize = 65536;
const MAXIMUM_LOCATIONS: usize = 1024;
type PositionKey = (GenerationId, SymbolKey, usize);

#[derive(Debug, Default)]
pub(super) struct PositionProfile {
    active: bool,
    started_at_dispatches: u64,
    ended_at_dispatches: u64,
    incomplete: bool,
    dropped_samples: u64,
    counts: BTreeMap<PositionKey, u64>,
}

impl PositionProfile {
    pub(super) fn boundary(&mut self, begin: bool, dispatches: u64) {
        if begin {
            *self = Self {
                active: true,
                started_at_dispatches: dispatches,
                ended_at_dispatches: dispatches,
                ..Self::default()
            };
        } else {
            self.active = false;
            self.ended_at_dispatches = dispatches;
        }
    }

    pub(super) fn mark_incomplete(&mut self) {
        if self.active {
            self.incomplete = true;
        }
    }

    pub(super) fn sample(
        &mut self,
        generation: GenerationId,
        function: SymbolKey,
        instruction: usize,
    ) {
        if !self.active {
            return;
        }
        let key = (generation, function, instruction);
        if let Some(count) = self.counts.get_mut(&key) {
            self.incomplete |= *count == u64::MAX;
            *count = count.saturating_add(1);
        } else if self.counts.len() < MAXIMUM_POSITIONS {
            self.counts.insert(key, 1);
        } else {
            self.incomplete = true;
            self.dropped_samples = self.dropped_samples.saturating_add(1);
        }
    }

    pub(super) fn snapshot(&self, vm: &Vm) -> PositionSnapshot {
        let mut ranked: Vec<_> = self.counts.iter().collect();
        ranked.sort_by_key(|(key, samples)| (std::cmp::Reverse(**samples), **key));
        let locations = ranked
            .into_iter()
            .take(MAXIMUM_LOCATIONS)
            .map(|(&key, _)| {
                let program = vm.generations.get(&key.0);
                let name = program
                    .and_then(|program| program.function(key.1))
                    .map_or("<reclaimed generation>", |function| function.name.as_str());
                let source = program.and_then(|program| program.source_location(key.1, key.2));
                PositionLocation {
                    position: PositionIdentity::from(key),
                    name: name.chars().take(64).collect(),
                    path: source
                        .as_ref()
                        .map(|source| source.relative_path.chars().take(160).collect()),
                    path_truncated: source
                        .as_ref()
                        .is_some_and(|source| source.relative_path.chars().count() > 160),
                    line: source.map(|source| source.line.to_string()),
                }
            })
            .collect();
        PositionSnapshot {
            active: self.active,
            started_at_dispatches: self.started_at_dispatches.to_string(),
            ended_at_dispatches: if self.active {
                vm.instruction_profile.dispatches
            } else {
                self.ended_at_dispatches
            }
            .to_string(),
            incomplete: self.incomplete,
            dropped_samples: self.dropped_samples.to_string(),
            counts: self
                .counts
                .iter()
                .map(|(&key, samples)| PositionCount {
                    position: PositionIdentity::from(key),
                    samples: samples.to_string(),
                })
                .collect(),
            unprojected_positions: self.counts.len().saturating_sub(MAXIMUM_LOCATIONS),
            locations,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PositionSnapshot {
    active: bool,
    started_at_dispatches: String,
    ended_at_dispatches: String,
    incomplete: bool,
    dropped_samples: String,
    counts: Vec<PositionCount>,
    locations: Vec<PositionLocation>,
    unprojected_positions: usize,
}

#[derive(Debug, Serialize)]
struct PositionIdentity {
    generation: String,
    function: SymbolKey,
    instruction: String,
}

impl From<PositionKey> for PositionIdentity {
    fn from((generation, function, instruction): PositionKey) -> Self {
        Self {
            generation: generation.0.to_string(),
            function,
            instruction: instruction.to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
struct PositionCount {
    #[serde(flatten)]
    position: PositionIdentity,
    samples: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PositionLocation {
    #[serde(flatten)]
    position: PositionIdentity,
    name: String,
    path: Option<String>,
    path_truncated: bool,
    line: Option<String>,
}

#[cfg(test)]
pub(super) mod tests;
