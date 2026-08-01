// This is part of the split RuntimeSession storage implementation.
#[allow(clippy::wildcard_imports)]
use super::super::*;

impl RuntimeSession {
    pub(in super::super) fn resume_storage_host_value(
        &mut self,
        request: erabasic_vm::HostRequestId,
        value: VmValue,
        writes: Vec<HostWrite>,
    ) -> Result<(), RuntimeError> {
        let vm = self
            .vm
            .as_mut()
            .ok_or_else(|| RuntimeError::Internal("storage completion has no VM".into()))?;
        commit_completion(
            vm,
            request,
            VmHostCompletion::Ready(HostReady {
                value: Some(value),
                writes,
            }),
        )?;
        self.set_phase(RuntimePhase::Running)
    }

    pub(in super::super) fn file_list_writes(
        &self,
        target: Option<PlaceDescriptor>,
        values: &[String],
    ) -> Result<Vec<HostWrite>, RuntimeError> {
        let vm = self
            .vm
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("file list completion has no VM".into()))?;
        let base = target.or_else(|| global_place_at(vm, "RESULTS", 0));
        let Some(base) = base else {
            return Ok(Vec::new());
        };
        let maximum = vm
            .vm()
            .artifact()
            .globals
            .iter()
            .find(|definition| definition.key == base.variable)
            .and_then(|definition| definition.dimensions.first())
            .and_then(|value| usize::try_from(*value).ok())
            .unwrap_or(0);
        Ok(values
            .iter()
            .take(maximum)
            .enumerate()
            .map(|(index, value)| {
                let mut target = base.clone();
                if let Some(last) = target.indices.last_mut() {
                    *last = u64::try_from(index).unwrap_or(u64::MAX);
                } else {
                    target
                        .indices
                        .push(u64::try_from(index).unwrap_or(u64::MAX));
                }
                HostWrite {
                    target,
                    value: VmValue::String(value.clone()),
                }
            })
            .collect())
    }

    pub(in super::super) fn result_write(
        &self,
        value: i64,
    ) -> Result<Vec<HostWrite>, RuntimeError> {
        let vm = self
            .vm
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("storage completion has no VM".into()))?;
        Ok(global_place(vm, "RESULT")
            .map(|target| {
                vec![HostWrite {
                    target,
                    value: VmValue::Integer(value),
                }]
            })
            .unwrap_or_default())
    }

    pub(in super::super) fn check_data_writes(
        &self,
        description: &str,
    ) -> Result<Vec<HostWrite>, RuntimeError> {
        let vm = self
            .vm
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("save check completion has no VM".into()))?;
        let mut writes = Vec::new();
        if let Some(target) = global_place(vm, "RESULTS") {
            writes.push(HostWrite {
                target,
                value: VmValue::String(description.to_owned()),
            });
        }
        Ok(writes)
    }

    pub(in super::super) fn complete_global_load(
        &mut self,
        request: erabasic_vm::HostRequestId,
        bytes: &[u8],
    ) -> Result<(), RuntimeError> {
        let mut vm = self
            .vm
            .take()
            .ok_or_else(|| RuntimeError::Internal("global load has no VM".into()))?;
        let decoded = decode_scoped_save(
            bytes,
            vm.vm().artifact(),
            era_runtime_save::SaveFileKind::Global,
        )
        .map_err(|error| RuntimeError::Internal(format!("invalid global save: {error}")))?;
        let (prepared, _) = vm
            .prepare_runtime_state_with_extensions(
                VmRuntimeStateTransaction::OverlayGlobal(Box::new(decoded.state)),
                StructuredScope::Global,
                &decoded.structured_extensions,
            )
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        vm.commit_runtime_state(prepared)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        self.save_extensions =
            merge_opaque_extensions(&self.save_extensions, decoded.opaque_extensions);
        let writes = global_place(&vm, "RESULT")
            .map(|target| {
                vec![HostWrite {
                    target,
                    value: VmValue::Integer(1),
                }]
            })
            .unwrap_or_default();
        commit_completion(
            &mut vm,
            request,
            VmHostCompletion::Ready(HostReady {
                value: None,
                writes,
            }),
        )?;
        self.vm = Some(vm);
        self.set_phase(RuntimePhase::Running)
    }

    pub(in super::super) fn complete_ordinary_load(
        &mut self,
        slot: u32,
        bytes: &[u8],
    ) -> Result<(), RuntimeError> {
        let establish_undo = self.undo_replay.is_none();
        let random_before_load = establish_undo
            .then(|| {
                self.vm
                    .as_ref()
                    .ok_or_else(|| RuntimeError::Internal("ordinary load has no VM".into()))?
                    .export_random_state()
                    .map_err(|error| RuntimeError::Internal(error.to_string()))
            })
            .transpose()?;
        let mut vm = self
            .vm
            .take()
            .ok_or_else(|| RuntimeError::Internal("ordinary load has no VM".into()))?;
        let decoded = decode_scoped_save(
            bytes,
            vm.vm().artifact(),
            era_runtime_save::SaveFileKind::Normal,
        )
        .map_err(|error| RuntimeError::Internal(format!("invalid ordinary save: {error}")))?;
        let version = decoded.state.version;
        let description = decoded.description.clone();
        let (prepared, _) = vm
            .prepare_runtime_state_with_extensions(
                VmRuntimeStateTransaction::RestoreOrdinary(Box::new(decoded.state)),
                StructuredScope::Ordinary,
                &decoded.structured_extensions,
            )
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        vm.commit_runtime_state(prepared)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        let last_load = vm
            .prepare_runtime_state(VmRuntimeStateTransaction::SetLastLoad {
                version,
                slot: i64::from(slot),
                text: description,
            })
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        vm.commit_runtime_state(last_load)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        self.save_extensions = decoded.opaque_extensions;
        self.advance_epoch();
        self.controller.clear();
        self.controller.flow = Some(SystemFlow::Shop);
        self.controller.step = SystemStep::PostLoadShop;
        self.controller.prepare_load_sequence(vm.vm().artifact());
        if self.controller.is_complete() {
            self.continue_system_flow(&mut vm)?;
        } else {
            self.spawn_next_event(&mut vm)?;
        }
        self.vm = Some(vm);
        self.set_phase(RuntimePhase::Running)?;
        if let Some(random) = random_before_load {
            self.establish_input_undo_checkpoint(slot, bytes.to_vec(), random)?;
        }
        Ok(())
    }

    pub(in super::super) fn complete_character_load(
        &mut self,
        request: erabasic_vm::HostRequestId,
        bytes: &[u8],
    ) -> Result<(), RuntimeError> {
        let mut vm = self
            .vm
            .take()
            .ok_or_else(|| RuntimeError::Internal("character load has no VM".into()))?;
        let Ok(decoded) = decode_scoped_save(
            bytes,
            vm.vm().artifact(),
            era_runtime_save::SaveFileKind::Character,
        ) else {
            let writes = global_place(&vm, "RESULT")
                .map(|target| {
                    vec![HostWrite {
                        target,
                        value: VmValue::Integer(0),
                    }]
                })
                .unwrap_or_default();
            commit_completion(
                &mut vm,
                request,
                VmHostCompletion::Ready(HostReady {
                    value: None,
                    writes,
                }),
            )?;
            self.vm = Some(vm);
            return self.set_phase(RuntimePhase::Running);
        };
        let prepared = vm
            .prepare_runtime_state(VmRuntimeStateTransaction::AppendCharacters(Box::new(
                decoded.state,
            )))
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        vm.commit_runtime_state(prepared)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        let writes = global_place(&vm, "RESULT")
            .map(|target| {
                vec![HostWrite {
                    target,
                    value: VmValue::Integer(1),
                }]
            })
            .unwrap_or_default();
        commit_completion(
            &mut vm,
            request,
            VmHostCompletion::Ready(HostReady {
                value: None,
                writes,
            }),
        )?;
        self.vm = Some(vm);
        self.set_phase(RuntimePhase::Running)
    }
}
