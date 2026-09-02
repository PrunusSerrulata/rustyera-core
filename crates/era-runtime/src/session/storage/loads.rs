// This is part of the split RuntimeSession storage implementation.
#[allow(clippy::wildcard_imports)]
use super::super::*;

struct PreparedOrdinaryLoad {
    prepared: Box<PreparedRuntimeState>,
    opaque_extensions: Vec<era_runtime_save::OpaqueSaveExtension>,
}

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
        storage_path: &str,
    ) -> Result<(), RuntimeError> {
        let replay_origin = self.prepare_input_replay(ReplayOriginDetails::ExternalDataLoad {
            storage_path: storage_path.to_owned(),
            payload_digest: crate::input_replay::digest_hex(bytes),
            data_type: crate::input_replay::ReplayExternalDataType::Global,
        })?;
        let vm = self
            .vm
            .as_ref()
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
        let writes = global_place(vm, "RESULT")
            .map(|target| {
                vec![HostWrite {
                    target,
                    value: VmValue::Integer(1),
                }]
            })
            .unwrap_or_default();
        // Decode, state preparation and host validation must not remove the live VM.
        // A corrupt or incompatible GLOBAL load leaves both state and replay intact.
        let completion = vm
            .validate_host_completion(
                request,
                VmHostCompletion::Ready(HostReady {
                    value: None,
                    writes,
                }),
            )
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        let vm = self
            .vm
            .as_mut()
            .expect("global load was prepared against the live VM");
        vm.commit_runtime_state(prepared)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        vm.commit_host_completion(completion)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        self.save_extensions =
            merge_opaque_extensions(&self.save_extensions, decoded.opaque_extensions);
        self.set_phase(RuntimePhase::Running)?;
        self.install_input_replay(replay_origin);
        self.emit_snake_save_load_diagnostic(SaveLoadScope::Global);
        Ok(())
    }

    pub(in super::super) fn complete_ordinary_load(
        &mut self,
        slot: u32,
        bytes: &[u8],
        host_request: Option<erabasic_vm::HostRequestId>,
    ) -> Result<(), RuntimeError> {
        let (decoded, snake_profile) = {
            let vm = self
                .vm
                .as_ref()
                .ok_or_else(|| RuntimeError::Internal("ordinary load has no VM".into()))?;
            (
                decode_scoped_save(
                    bytes,
                    vm.vm().artifact(),
                    era_runtime_save::SaveFileKind::Normal,
                ),
                vm.vm().artifact().manifest.compatibility.profile
                    == erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
            )
        };
        let decoded = match decoded {
            Ok(decoded) => decoded,
            Err(error) if snake_profile => {
                return self.finish_snake_save_load_failure(
                    host_request,
                    &format!("invalid snake save: {error}"),
                );
            }
            Err(error) => {
                return Err(RuntimeError::Internal(format!(
                    "invalid ordinary save: {error}"
                )));
            }
        };
        self.complete_decoded_ordinary_load(slot, bytes, decoded, host_request)
    }

    pub(in super::super) fn complete_decoded_ordinary_load(
        &mut self,
        slot: u32,
        bytes: &[u8],
        decoded: DecodedEraSave,
        host_request: Option<erabasic_vm::HostRequestId>,
    ) -> Result<(), RuntimeError> {
        let snake = self.vm.as_ref().is_some_and(|vm| {
            vm.vm().artifact().manifest.compatibility.profile
                == erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake
        });
        let load = match self.prepare_decoded_ordinary_load(slot, decoded) {
            Ok(load) => load,
            Err(error) if snake => {
                return self.finish_snake_save_load_failure(host_request, &error.to_string());
            }
            Err(error) => return Err(error),
        };
        self.complete_prepared_ordinary_load(slot, bytes, load)
    }

    fn prepare_decoded_ordinary_load(
        &self,
        slot: u32,
        decoded: DecodedEraSave,
    ) -> Result<PreparedOrdinaryLoad, RuntimeError> {
        let DecodedEraSave {
            state,
            description,
            opaque_extensions,
            structured_extensions,
        } = decoded;
        let vm = self
            .vm
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("ordinary load has no VM".into()))?;
        let (prepared, _) = vm
            .prepare_runtime_state_with_extensions(
                VmRuntimeStateTransaction::RestoreOrdinaryWithLastLoad {
                    state: Box::new(state),
                    slot: i64::from(slot),
                    text: description,
                },
                StructuredScope::Ordinary,
                &structured_extensions,
            )
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        Ok(PreparedOrdinaryLoad {
            prepared: Box::new(prepared),
            opaque_extensions,
        })
    }

    #[allow(clippy::too_many_lines)]
    fn complete_prepared_ordinary_load(
        &mut self,
        slot: u32,
        bytes: &[u8],
        load: PreparedOrdinaryLoad,
    ) -> Result<(), RuntimeError> {
        let establish_undo = self.undo_replay.is_none();
        let replay_details = if let Some(replay) = &self.undo_replay {
            ReplayOriginDetails::InputUndo {
                checkpoint_slot: slot,
                save_digest: crate::input_replay::digest_hex(bytes),
                retained_input_count: replay.remaining.len(),
            }
        } else {
            ReplayOriginDetails::OrdinarySave {
                slot,
                storage_path: save_slot_path(slot),
                payload_digest: crate::input_replay::digest_hex(bytes),
            }
        };
        let replay_origin = self.prepare_input_replay(replay_details)?;
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
        if let Err(error) = vm.commit_runtime_state(*load.prepared) {
            self.vm = Some(vm);
            return Err(RuntimeError::Internal(error.to_string()));
        }
        self.system_menu_host_request = None;
        self.save_extensions = load.opaque_extensions;
        self.advance_epoch();
        self.queued_input.clear();
        self.active_input_source = None;
        self.input_controller = if self.undo_replay.is_some() {
            self.undo_checkpoint
                .as_ref()
                .map(|checkpoint| checkpoint.input_controller.clone())
                .unwrap_or_default()
        } else {
            // Bare saves do not serialize process-owned input controls. A LOAD
            // in the same process retains the live switch and single pending slot.
            self.input_controller.clone()
        };
        self.controller.clear();
        self.controller.flow = Some(SystemFlow::Shop);
        self.controller.step = SystemStep::PostLoadShop;
        self.controller.prepare_load_sequence(vm.vm().artifact());
        let flow = if self.controller.is_complete() {
            self.continue_system_flow(&mut vm)
        } else {
            self.spawn_next_event(&mut vm)
        };
        if let Err(error) = flow {
            self.vm = Some(vm);
            return Err(error);
        }
        if let Err(error) = self.set_phase(RuntimePhase::Running) {
            self.vm = Some(vm);
            return Err(error);
        }
        if let Some(random) = random_before_load
            && let Err(error) = self.establish_input_undo_checkpoint(slot, bytes.to_vec(), random)
        {
            self.vm = Some(vm);
            return Err(error);
        }
        self.install_input_replay(replay_origin);
        self.vm = Some(vm);
        self.emit_snake_save_load_diagnostic(SaveLoadScope::Ordinary);
        Ok(())
    }

    pub(in crate::session) fn emit_snake_save_load_diagnostic(&mut self, scope: SaveLoadScope) {
        let snake = self.project_snapshot.as_ref().is_some_and(|project| {
            project.manifest.compatibility.profile
                == erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake
        });
        if !snake {
            return;
        }
        let (code, message) = match scope {
            SaveLoadScope::Ordinary => (
                "runtime.interoperable_save_external_state_preserved",
                "loaded a standard Emuera 1808 ordinary save; the file has no recoverable RNG or SQL snapshot, so the live SFMT stream and external SQL state were preserved",
            ),
            SaveLoadScope::Global => (
                "runtime.interoperable_global_external_state_preserved",
                "loaded a standard Emuera 1808 GLOBAL save; only GLOBAL scope was overlaid, and the live SFMT stream and external SQL state were preserved",
            ),
        };
        // Loading is already committed at this point. A saturated outbound diagnostic journal
        // must not turn a successful VM/SQL publication into an apparent restore failure.
        let _ = self.emit(
            RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                context: None,
                code: code.into(),
                level: RuntimeLogLevel::Info,
                message: message.into(),
                source: None,
                notification: DiagnosticNotification::default(),
            }),
            None,
        );
    }

    pub(in super::super) fn finish_snake_save_load_failure(
        &mut self,
        host_request: Option<erabasic_vm::HostRequestId>,
        message: &str,
    ) -> Result<(), RuntimeError> {
        self.emit(
            RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                context: None,
                code: "runtime.snake_save_restore_failed".into(),
                level: RuntimeLogLevel::Warning,
                message: message.into(),
                source: None,
                notification: DiagnosticNotification::default(),
            }),
            None,
        )?;
        if let Some(request) = host_request {
            self.resume_storage_host(request, Vec::new())
        } else if self.system_menu == SystemMenuState::LoadSlots {
            self.presentation.append_system_text(
                localized_system_text(&self.selected_locale, SystemTextKey::InvalidValue),
                SystemTextKey::InvalidValue,
                Vec::new(),
                true,
            );
            self.render_slot_menu(false)
        } else {
            self.set_phase(RuntimePhase::Running)
        }
    }

    pub(in super::super) fn complete_character_load(
        &mut self,
        request: erabasic_vm::HostRequestId,
        bytes: &[u8],
        storage_path: &str,
    ) -> Result<(), RuntimeError> {
        let replay_origin = self.prepare_input_replay(ReplayOriginDetails::ExternalDataLoad {
            storage_path: storage_path.to_owned(),
            payload_digest: crate::input_replay::digest_hex(bytes),
            data_type: crate::input_replay::ReplayExternalDataType::Character,
        })?;
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
        self.set_phase(RuntimePhase::Running)?;
        self.install_input_replay(replay_origin);
        Ok(())
    }
}
