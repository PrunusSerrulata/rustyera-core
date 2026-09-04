#[allow(clippy::wildcard_imports)]
use super::*;

impl Vm {
    pub(crate) fn allocate_frame_id(&mut self) -> FrameId {
        let id = FrameId(self.next_frame);
        self.next_frame = self.next_frame.saturating_add(1);
        id
    }

    pub(crate) fn allocate_request_id(&mut self) -> HostRequestId {
        let id = HostRequestId(self.next_request);
        self.next_request = self.next_request.saturating_add(1);
        id
    }

    pub(in super::super) fn next_available_fiber_id(&self) -> FiberId {
        let mut candidate = 1_u64;
        for id in self.fibers.keys() {
            if id.0 < candidate {
                continue;
            }
            if id.0 != candidate {
                break;
            }
            candidate = candidate
                .checked_add(1)
                .expect("the fiber map cannot contain every positive u64 id");
        }
        FiberId(candidate)
    }

    pub(crate) fn live_fiber_count(&self) -> usize {
        self.fibers
            .values()
            .filter(|fiber| {
                !matches!(
                    fiber.state,
                    FiberState::Completed(_) | FiberState::Cancelled | FiberState::Faulted(_)
                )
            })
            .count()
    }

    pub(crate) fn active_generations(&self) -> BTreeSet<GenerationId> {
        self.fibers
            .values()
            .flat_map(|fiber| fiber.frames.iter().map(|frame| frame.generation))
            .collect()
    }

    pub(crate) fn reclaim_generations(&mut self) {
        self.prune_bit_leases();
        if self.generations.len() <= 1 {
            return;
        }
        let mut active = self.active_generations();
        active.extend(self.memory.array_leases.retained_generations());
        let obsolete: Vec<_> = self
            .generations
            .keys()
            .copied()
            .filter(|generation| {
                *generation != self.current_generation && !active.contains(generation)
            })
            .collect();
        let reclaimed = !obsolete.is_empty();
        for generation in obsolete {
            self.generations.remove(&generation);
            self.memory.reclaim_generation(generation);
            self.compatibility_warning_sites
                .retain(|site| site.0 != generation);
            self.path_memo_cache
                .retain(|head, _| head.generation != generation);
        }
        if reclaimed {
            (self.path_memo_key_count, self.path_memo_retained_bytes) =
                path_memo_cache_usage(&self.path_memo_cache);
            self.clear_derived_caches();
        }
    }
}
