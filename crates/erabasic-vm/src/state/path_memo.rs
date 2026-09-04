#[allow(clippy::wildcard_imports)]
use super::*;
use crate::VmHost;

const MAX_PATH_MEMO_KEYS: usize = 8_192;
const MAX_PATHS_PER_KEY: usize = 4;
const MAX_DEPENDENCIES: usize = 128;
const MAX_MUTATIONS: usize = 512;
const MAX_RETAINED_BYTES: usize = 64 * 1024;
const MAX_CACHE_RETAINED_BYTES: usize = 16 * 1024 * 1024;
const MAX_IDEMPOTENT_REPLAY_CELL_ELEMENTS: usize = 4_096;

fn path_memo_mutation_retained_bytes(mutation: &PathMemoMutation) -> usize {
    match mutation {
        PathMemoMutation::Write { place, value } => retained_value_bytes(value).saturating_add(
            place
                .indices
                .len()
                .saturating_mul(std::mem::size_of::<u64>()),
        ),
        PathMemoMutation::Fill { value, .. } => retained_value_bytes(value),
        PathMemoMutation::Replace { values, .. } => {
            values.iter().map(retained_value_bytes).sum::<usize>()
        }
    }
}

fn path_memo_entries<'a>(
    cache: &'a PathMemoCache,
    head: &PathMemoHead,
    arguments: &[VmValue],
) -> Option<&'a [Arc<PathMemoEntry>]> {
    cache
        .get(head)
        .and_then(|paths| paths.get(arguments))
        .map(Vec::as_slice)
}

fn path_memo_dependencies_match(
    generations: &BTreeMap<GenerationId, Arc<ProgramGeneration>>,
    memory: &Memory,
    entry: &PathMemoEntry,
) -> bool {
    entry
        .dependencies
        .iter()
        .enumerate()
        .all(|(index, dependency)| {
            if entry.result_dependency == Some(index) {
                return true;
            }
            match dependency {
                PathMemoDependency::Value { place, value } => {
                    let Some(program) = generations.get(&place.generation) else {
                        return false;
                    };
                    let Some(definition) = program.global(place.variable) else {
                        return false;
                    };
                    memory
                        .cell(place.generation, definition, place.character)
                        .and_then(|cell| cell.read(&place.indices).ok())
                        .is_some_and(|observed| observed.eq(value))
                }
                PathMemoDependency::CellRevision {
                    generation,
                    variable,
                    revision,
                } => generations
                    .get(generation)
                    .and_then(|program| program.global(*variable))
                    .and_then(|definition| memory.cell(*generation, definition, 0))
                    .is_some_and(|cell| cell.revision() == *revision),
                PathMemoDependency::TargetIdentity {
                    generation,
                    character,
                } => {
                    generations.get(generation).map_or(0, |program| {
                        memory
                            .target_character_from_definition(program.target_global(), *generation)
                    }) == *character
                }
            }
        })
}

fn read_path_memo_place(
    generations: &BTreeMap<GenerationId, Arc<ProgramGeneration>>,
    memory: &Memory,
    place: &PathMemoPlace,
) -> Option<VmValue> {
    let definition = generations.get(&place.generation)?.global(place.variable)?;
    memory
        .cell(place.generation, definition, place.character)?
        .read(&place.indices)
        .ok()
}

fn replay_path_memo_mutation(
    generations: &BTreeMap<GenerationId, Arc<ProgramGeneration>>,
    memory: &mut Memory,
    mutation: &PathMemoMutation,
) -> Result<(), VmError> {
    let (generation, variable, character) = mutation.cell_key();
    let definition = generations
        .get(&generation)
        .and_then(|program| program.global(variable))
        .ok_or_else(|| VmError::InvalidState("path memo variable is missing".into()))?;
    let storage = definition.storage;
    let cell = memory
        .cell_mut(generation, definition.key, storage, character)
        .ok_or_else(|| VmError::InvalidState("path memo storage is missing".into()))?;
    apply_path_memo_mutation(cell, mutation)
}

pub(crate) fn path_memo_cache_usage(cache: &PathMemoCache) -> (usize, usize) {
    cache.values().flat_map(|paths| paths.values()).fold(
        (0_usize, 0_usize),
        |(key_count, retained_bytes), entries| {
            (
                key_count.saturating_add(1),
                entries.iter().fold(retained_bytes, |bytes, entry| {
                    bytes.saturating_add(entry.retained_bytes)
                }),
            )
        },
    )
}

fn apply_path_memo_mutation(
    cell: &mut VariableCell,
    mutation: &PathMemoMutation,
) -> Result<(), VmError> {
    match mutation {
        PathMemoMutation::Write { place, value } => cell
            .write(&place.indices, value.clone())
            .map_err(VmError::InvalidState),
        PathMemoMutation::Fill {
            start, end, value, ..
        } => cell
            .fill_range(*start, *end, value.clone())
            .map_err(VmError::InvalidState),
        PathMemoMutation::Replace { values, .. } => cell
            .replace_values(values.clone())
            .map_err(VmError::InvalidState),
    }
}

fn retained_value_bytes(value: &VmValue) -> usize {
    std::mem::size_of::<VmValue>().saturating_add(match value {
        VmValue::String(value) => value.len(),
        VmValue::Integer(_) => 0,
        VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => MAX_RETAINED_BYTES,
    })
}

fn enforce_path_memo_limits(active: &mut ActivePathMemo) {
    if active.dependencies.len() > MAX_DEPENDENCIES
        || active.mutations.len() > MAX_MUTATIONS
        || active.retained_bytes > MAX_RETAINED_BYTES
    {
        active.valid = false;
    }
}

mod observation;
mod replay;
#[cfg(test)]
mod tests;
