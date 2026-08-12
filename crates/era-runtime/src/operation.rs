use std::collections::{BTreeMap, VecDeque};

use era_runtime_save::SaveFileKind;
use erabasic_vm::HostRequestId;
use serde::{Deserialize, Serialize};

use crate::host::{ExternalCompletion, PendingInput};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(crate) enum CandidateSaveContinuation {
    Autosave,
    SystemMenu { request: HostRequestId, slot: u32 },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum PendingService {
    StartEntropy,
    ProjectImageMetadata {
        relative_path: String,
    },
    PlatformEffect {
        operation: String,
    },
    CandidateSaveClock {
        slot: u32,
        precondition: era_runtime_protocol::StoragePrecondition,
        continuation: CandidateSaveContinuation,
    },
    Host(ExternalCompletion),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum PendingStorage {
    KeyMacroWrite {
        resume_phase: era_runtime_protocol::RuntimePhase,
    },
    SystemOutputLog {
        resume_phase: era_runtime_protocol::RuntimePhase,
    },
    ListLoadSlots,
    ListSaveSlots,
    ScanMenuSlot {
        save: bool,
        path: String,
        remaining: Vec<String>,
        data: Vec<u8>,
        change_token: Option<String>,
    },
    StatDeleteMenuSlot {
        save: bool,
        path: String,
    },
    DeleteMenuSlot {
        save: bool,
        path: String,
    },
    ReadLoadSlot {
        slot: u32,
    },
    HostWrite {
        request: HostRequestId,
    },
    HostDelete {
        request: HostRequestId,
    },
    HostLoadOrdinary {
        slot: u32,
    },
    HostLoadGlobal {
        request: HostRequestId,
        storage_path: String,
    },
    HostLoadCharacters {
        request: HostRequestId,
        storage_path: String,
    },
    HostCheck {
        request: HostRequestId,
        kind: SaveFileKind,
    },
    HostFunctionWrite {
        request: HostRequestId,
    },
    HostReadText {
        request: HostRequestId,
    },
    HostStat {
        request: HostRequestId,
    },
    HostListFiles {
        request: HostRequestId,
        target: Option<erabasic_vm::PlaceDescriptor>,
        strip_character_dat: bool,
    },
    GraphicsImageRead {
        request: HostRequestId,
        canvas_id: i64,
    },
    GraphicsImageWrite {
        request: HostRequestId,
    },
    CandidateSaveStat {
        slot: u32,
        continuation: CandidateSaveContinuation,
    },
    CandidateSaveWrite {
        continuation: CandidateSaveContinuation,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum PendingOperation {
    Input(PendingInput),
    Service {
        request_id: u64,
        value: PendingService,
    },
    Storage {
        request_id: u64,
        value: PendingStorage,
    },
    Delay {
        request: HostRequestId,
        deadline_ns: u64,
    },
}

/// The single asynchronous ownership table for a runtime session.
///
/// Protocol-visible IDs live in each typed entry and are never reused as table keys.
/// This prevents service, storage, VM host and wait ID domains from colliding while
/// preserving deterministic completion in the actor's inbound-message order.
#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct PendingOperations {
    next_id: u64,
    epoch: u64,
    entries: BTreeMap<u64, PendingOperation>,
    active_input: Option<u64>,
    queued_inputs: VecDeque<u64>,
}

impl PendingOperations {
    pub(crate) fn bind_epoch(&mut self, epoch: u64) {
        if self.epoch != epoch {
            self.clear();
            self.epoch = epoch;
        }
    }

    pub(crate) fn rebind_stable_inputs(
        &mut self,
        epoch: u64,
        next_wait: &mut u64,
        next_interaction: &mut u64,
    ) -> (
        BTreeMap<era_runtime_protocol::InteractionToken, era_runtime_protocol::InteractionToken>,
        BTreeMap<u64, u64>,
    ) {
        let mut tokens = BTreeMap::new();
        let mut waits = BTreeMap::new();
        for operation in self.entries.values_mut() {
            let PendingOperation::Input(input) = operation else {
                continue;
            };
            let new_wait = *next_wait;
            *next_wait = next_wait.saturating_add(1);
            waits.insert(input.wait.wait_id, new_wait);
            input.wait.wait_id = new_wait;

            let old_submission = input.wait.submission_token;
            let new_submission = era_runtime_protocol::InteractionToken {
                epoch,
                id: *next_interaction,
            };
            *next_interaction = next_interaction.saturating_add(1);
            tokens.insert(old_submission, new_submission);
            input.wait.submission_token = new_submission;
            input.choices = std::mem::take(&mut input.choices)
                .into_iter()
                .map(|(old, value)| {
                    let new = era_runtime_protocol::InteractionToken {
                        epoch,
                        id: *next_interaction,
                    };
                    *next_interaction = next_interaction.saturating_add(1);
                    tokens.insert(old, new);
                    (new, value)
                })
                .collect();
        }
        self.epoch = epoch;
        (tokens, waits)
    }

    fn insert(&mut self, operation: PendingOperation) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.entries.insert(id, operation);
        id
    }

    pub(crate) fn total_count(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn has_transient_external(&self) -> bool {
        self.entries.values().any(|operation| {
            matches!(
                operation,
                PendingOperation::Service { .. } | PendingOperation::Delay { .. }
            )
        })
    }

    pub(crate) fn is_snapshot_stable(&self) -> bool {
        !self.entries.is_empty()
            && self.entries.values().all(|operation| match operation {
                PendingOperation::Input(input) => {
                    input.wait.stability == era_runtime_protocol::WaitStability::StableInput
                        && input.wait.deadline_ns.is_none()
                        && input.timeout_duration_ns.is_none()
                }
                PendingOperation::Service { .. }
                | PendingOperation::Storage { .. }
                | PendingOperation::Delay { .. } => false,
            })
    }

    pub(crate) fn insert_service(&mut self, request_id: u64, value: PendingService) {
        self.insert(PendingOperation::Service { request_id, value });
    }

    pub(crate) fn take_service(&mut self, request_id: u64) -> Option<PendingService> {
        let id = self
            .entries
            .iter()
            .find_map(|(id, operation)| match operation {
                PendingOperation::Service {
                    request_id: candidate,
                    ..
                } if *candidate == request_id => Some(*id),
                _ => None,
            })?;
        match self.entries.remove(&id)? {
            PendingOperation::Service { value, .. } => Some(value),
            _ => unreachable!("operation kind changed"),
        }
    }

    pub(crate) fn insert_storage(&mut self, request_id: u64, value: PendingStorage) {
        self.insert(PendingOperation::Storage { request_id, value });
    }

    pub(crate) fn take_storage(&mut self, request_id: u64) -> Option<PendingStorage> {
        let id = self
            .entries
            .iter()
            .find_map(|(id, operation)| match operation {
                PendingOperation::Storage {
                    request_id: candidate,
                    ..
                } if *candidate == request_id => Some(*id),
                _ => None,
            })?;
        match self.entries.remove(&id)? {
            PendingOperation::Storage { value, .. } => Some(value),
            _ => unreachable!("operation kind changed"),
        }
    }

    pub(crate) fn insert_delay(&mut self, request: HostRequestId, deadline_ns: u64) {
        self.insert(PendingOperation::Delay {
            request,
            deadline_ns,
        });
    }

    pub(crate) fn take_ready_delays(&mut self, now_ns: u64) -> Vec<HostRequestId> {
        let ready = self
            .entries
            .iter()
            .filter_map(|(id, operation)| match operation {
                PendingOperation::Delay {
                    deadline_ns,
                    request,
                } if *deadline_ns <= now_ns => Some((*id, *request)),
                _ => None,
            })
            .collect::<Vec<_>>();
        for (id, _) in &ready {
            self.entries.remove(id);
        }
        ready.into_iter().map(|(_, request)| request).collect()
    }

    pub(crate) fn active_input(&self) -> Option<&PendingInput> {
        match self.entries.get(&self.active_input?) {
            Some(PendingOperation::Input(input)) => Some(input),
            _ => None,
        }
    }

    pub(crate) fn input_host_requests(&self) -> Vec<HostRequestId> {
        self.entries
            .values()
            .filter_map(|operation| match operation {
                PendingOperation::Input(input) => input.host_request,
                _ => None,
            })
            .collect()
    }

    pub(crate) fn active_input_mut(&mut self) -> Option<&mut PendingInput> {
        match self.entries.get_mut(&self.active_input?) {
            Some(PendingOperation::Input(input)) => Some(input),
            _ => None,
        }
    }

    pub(crate) fn activate_input(&mut self, input: PendingInput) {
        debug_assert!(self.active_input.is_none());
        let id = self.insert(PendingOperation::Input(input));
        self.active_input = Some(id);
    }

    pub(crate) fn queue_input(&mut self, input: PendingInput) {
        let id = self.insert(PendingOperation::Input(input));
        self.queued_inputs.push_back(id);
    }

    pub(crate) fn take_active_input(&mut self) -> Option<PendingInput> {
        let id = self.active_input.take()?;
        match self.entries.remove(&id)? {
            PendingOperation::Input(input) => Some(input),
            _ => unreachable!("active operation is not input"),
        }
    }

    pub(crate) fn pop_queued_input(&mut self) -> Option<PendingInput> {
        let id = self.queued_inputs.pop_front()?;
        match self.entries.remove(&id)? {
            PendingOperation::Input(input) => Some(input),
            _ => unreachable!("queued operation is not input"),
        }
    }

    pub(crate) fn restore_active_input(&mut self, input: PendingInput) {
        self.activate_input(input);
    }

    pub(crate) fn external_requests(&self) -> (Vec<u64>, Vec<u64>) {
        let mut services = Vec::new();
        let mut storage = Vec::new();
        for operation in self.entries.values() {
            match operation {
                PendingOperation::Service { request_id, .. } => services.push(*request_id),
                PendingOperation::Storage { request_id, .. } => storage.push(*request_id),
                PendingOperation::Input(_) | PendingOperation::Delay { .. } => {}
            }
        }
        (services, storage)
    }

    pub(crate) fn has_candidate_write(&self) -> bool {
        self.entries.values().any(|operation| {
            matches!(
                operation,
                PendingOperation::Storage {
                    value: PendingStorage::CandidateSaveWrite { .. },
                    ..
                }
            )
        })
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.active_input = None;
        self.queued_inputs.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_ids_do_not_collide_and_completion_is_consuming() {
        let mut operations = PendingOperations::default();
        operations.insert_service(7, PendingService::StartEntropy);
        operations.insert_storage(7, PendingStorage::ReadLoadSlot { slot: 2 });
        assert!(matches!(
            operations.take_service(7),
            Some(PendingService::StartEntropy)
        ));
        assert!(operations.take_service(7).is_none());
        assert!(matches!(
            operations.take_storage(7),
            Some(PendingStorage::ReadLoadSlot { slot: 2 })
        ));
    }

    #[test]
    fn delay_deadlines_are_removed_in_deterministic_registry_order() {
        let mut operations = PendingOperations::default();
        operations.insert_delay(HostRequestId(2), 20);
        operations.insert_delay(HostRequestId(1), 10);
        assert_eq!(operations.take_ready_delays(15), vec![HostRequestId(1)]);
        assert_eq!(operations.take_ready_delays(25), vec![HostRequestId(2)]);
        assert!(operations.take_ready_delays(30).is_empty());
    }

    #[test]
    fn advancing_epoch_cancels_every_old_timeline_operation() {
        let mut operations = PendingOperations::default();
        operations.insert_service(1, PendingService::StartEntropy);
        operations.insert_delay(HostRequestId(2), 10);
        operations.bind_epoch(2);
        assert_eq!(operations.total_count(), 0);
        assert!(operations.take_service(1).is_none());
    }

    #[test]
    fn candidate_write_is_identified_as_a_noncancellable_commit_window() {
        let mut operations = PendingOperations::default();
        operations.insert_storage(
            9,
            PendingStorage::CandidateSaveWrite {
                continuation: CandidateSaveContinuation::Autosave,
            },
        );
        assert!(operations.has_candidate_write());
    }
}
