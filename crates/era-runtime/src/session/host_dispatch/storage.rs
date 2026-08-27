#[allow(clippy::wildcard_imports)]
use super::*;

impl RuntimeSession {
    #[allow(clippy::too_many_lines)]
    pub(super) fn dispatch_storage(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        let snake_resources = vm.vm().artifact().manifest.compatibility.profile
            == erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake;
        if matches!(name.as_str(), "SAVEVAR" | "LOADVAR") {
            *status = HostDispatchStatus::Handled;
            return self.fault(
                FaultCode::UnsupportedRuntimeFeature,
                &format!("{name} is not implemented by the pinned reference runtime"),
                Some(request.origin.clone()),
            );
        }
        if name == "PUTFORM" {
            *status = HostDispatchStatus::Handled;
            let suffix = request
                .arguments
                .first()
                .map(display_value)
                .unwrap_or_default();
            let variable = runtime_variable_key(vm, "SAVEDATA_TEXT")?;
            let current = vm
                .read_runtime_state(&[erabasic_vm::VmRuntimeRead {
                    variable,
                    indices: Vec::new(),
                    character: None,
                }])
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            let [VmValue::String(value)] = current.as_slice() else {
                return Err(RuntimeError::Internal(
                    "SAVEDATA_TEXT is not a scalar string".into(),
                ));
            };
            let mut value = value.clone();
            value.push_str(&suffix);
            write_runtime_string(vm, "SAVEDATA_TEXT", value)?;
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "SAVENOS" {
            *status = HostDispatchStatus::Handled;
            let count = self
                .project_snapshot
                .as_ref()
                .map_or(20, |snapshot| snapshot.save_slot_count);
            let value = VmValue::Integer(i64::from(count));
            let writes = request
                .arguments
                .first()
                .and_then(vm_place)
                .map(|target| {
                    vec![HostWrite {
                        target,
                        value: value.clone(),
                    }]
                })
                .unwrap_or_default();
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: writes.is_empty().then_some(value),
                    writes,
                }),
            );
        }
        if matches!(name.as_str(), "SAVEGAME" | "LOADGAME") {
            *status = HostDispatchStatus::Handled;
            if !matches!(
                self.controller.flow,
                Some(SystemFlow::Title | SystemFlow::Shop | SystemFlow::Normal)
            ) {
                return self.fault(
                    FaultCode::VmFault,
                    &format!("{name} cannot open outside the reference __CAN_SAVE__ states"),
                    Some(request.origin.clone()),
                );
            }
            commit_completion(
                vm,
                request.id,
                VmHostCompletion::Pending {
                    stability: HostWaitStability::StableInput,
                    rebind_payload: name.as_bytes().to_vec(),
                },
            )?;
            self.system_menu_host_request = Some(request.id);
            let save = name == "SAVEGAME";
            self.system_menu = if save {
                SystemMenuState::SaveSlots
            } else {
                SystemMenuState::LoadSlots
            };
            return self.issue_storage(
                if save {
                    PendingStorage::ListSaveSlots
                } else {
                    PendingStorage::ListLoadSlots
                },
                StorageNamespace::Save,
                StorageOperation::List {
                    pattern: Some("save*.sav".into()),
                    recursive: false,
                },
                String::new(),
            );
        }
        if matches!(name.as_str(), "RESETDATA" | "RESETGLOBAL") {
            *status = HostDispatchStatus::Handled;
            let transaction = if name == "RESETDATA" {
                VmRuntimeStateTransaction::ResetGameData
            } else {
                VmRuntimeStateTransaction::ResetGlobalData
            };
            let prepared = vm
                .prepare_runtime_state(transaction)
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            vm.commit_runtime_state(prepared)
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "SAVEDATA" {
            *status = HostDispatchStatus::Handled;
            let slot = save_slot_argument(&request.arguments, 0, "SAVEDATA")?;
            let description = string_argument_value(&request.arguments, 1, "SAVEDATA")?;
            if description.contains(['\r', '\n']) {
                return self.fault(
                    FaultCode::VmFault,
                    "SAVEDATA description cannot contain a newline",
                    Some(request.origin.clone()),
                );
            }
            let bytes = encode_scoped_save(
                &vm.export_era_state(),
                vm.vm().artifact(),
                era_runtime_save::SaveFileKind::Normal,
                description.to_owned(),
                merge_structured_extensions(
                    &self.save_extensions,
                    vm.structured_extensions(StructuredScope::Ordinary)
                        .map_err(|error| RuntimeError::Internal(error.to_string()))?,
                )
                .map_err(|error| RuntimeError::Internal(error.to_string()))?,
                self.traditional_save_format(),
            )
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostWrite {
                    request: request.id,
                },
                StorageNamespace::Save,
                StorageOperation::Write {
                    data: ProtocolBytes::new(bytes),
                    atomic_replace: true,
                    precondition: StoragePrecondition::Any,
                },
                save_slot_path(slot),
            );
        }
        if name == "LOADDATA" {
            *status = HostDispatchStatus::Handled;
            let slot = save_slot_argument(&request.arguments, 0, "LOADDATA")?;
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostLoadOrdinary { slot },
                StorageNamespace::Save,
                StorageOperation::Read,
                save_slot_path(slot),
            );
        }
        if name == "DELDATA" {
            *status = HostDispatchStatus::Handled;
            let slot = save_slot_argument(&request.arguments, 0, "DELDATA")?;
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostDelete {
                    request: request.id,
                },
                StorageNamespace::Save,
                StorageOperation::Delete {
                    precondition: StoragePrecondition::Any,
                },
                save_slot_path(slot),
            );
        }
        if name == "SAVEGLOBAL" {
            *status = HostDispatchStatus::Handled;
            let state = vm.vm().export_era_state_for(EraSaveScope::Global);
            let bytes = encode_scoped_save(
                &state,
                vm.vm().artifact(),
                era_runtime_save::SaveFileKind::Global,
                String::new(),
                merge_structured_extensions(
                    &self.save_extensions,
                    vm.structured_extensions(StructuredScope::Global)
                        .map_err(|error| RuntimeError::Internal(error.to_string()))?,
                )
                .map_err(|error| RuntimeError::Internal(error.to_string()))?,
                self.traditional_save_format(),
            )
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostWrite {
                    request: request.id,
                },
                StorageNamespace::GlobalSave,
                StorageOperation::Write {
                    data: ProtocolBytes::new(bytes),
                    atomic_replace: true,
                    precondition: StoragePrecondition::Any,
                },
                "global.sav".into(),
            );
        }
        if name == "LOADGLOBAL" {
            *status = HostDispatchStatus::Handled;
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostLoadGlobal {
                    request: request.id,
                    storage_path: "global.sav".into(),
                },
                StorageNamespace::GlobalSave,
                StorageOperation::Read,
                "global.sav".into(),
            );
        }
        if name == "SAVECHARA" {
            *status = HostDispatchStatus::Handled;
            let filename =
                dat_filename(string_argument_value(&request.arguments, 0, "SAVECHARA")?)?;
            let description = string_argument_value(&request.arguments, 1, "SAVECHARA")?;
            let exported = vm.vm().export_era_state_for(EraSaveScope::Characters);
            let mut selected = Vec::new();
            let mut seen = std::collections::BTreeSet::new();
            for index in 2..request.arguments.len() {
                let value = usize::try_from(integer_argument_value(&request.arguments, index)?)
                    .map_err(|_| {
                        RuntimeError::Internal(format!(
                            "SAVECHARA argument {} must be non-negative",
                            index + 1
                        ))
                    })?;
                if value >= exported.characters.len() {
                    return Err(RuntimeError::Internal(format!(
                        "SAVECHARA argument {} is not a character",
                        index + 1
                    )));
                }
                if !seen.insert(value) {
                    return Err(RuntimeError::Internal(format!(
                        "SAVECHARA character {value} is duplicated"
                    )));
                }
                selected.push(exported.characters[value].clone());
            }
            let state = EraState {
                unique_code: exported.unique_code,
                version: exported.version,
                variables: BTreeMap::new(),
                characters: selected,
            };
            let bytes = encode_scoped_save(
                &state,
                vm.vm().artifact(),
                era_runtime_save::SaveFileKind::Character,
                description.to_owned(),
                Vec::new(),
                era_runtime_save::SaveFormat::Binary1808,
            )
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostWrite {
                    request: request.id,
                },
                StorageNamespace::Data,
                StorageOperation::Write {
                    data: ProtocolBytes::new(bytes),
                    atomic_replace: true,
                    precondition: StoragePrecondition::Any,
                },
                format!("chara_{filename}.dat"),
            );
        }
        if name == "LOADCHARA" {
            *status = HostDispatchStatus::Handled;
            let filename =
                dat_filename(string_argument_value(&request.arguments, 0, "LOADCHARA")?)?;
            let storage_path = format!("chara_{filename}.dat");
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostLoadCharacters {
                    request: request.id,
                    storage_path: storage_path.clone(),
                },
                StorageNamespace::Data,
                StorageOperation::Read,
                storage_path,
            );
        }
        if name == "CHKDATA" {
            *status = HostDispatchStatus::Handled;
            let slot = save_slot_argument(&request.arguments, 0, "CHKDATA")?;
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostCheck {
                    request: request.id,
                    kind: era_runtime_save::SaveFileKind::Normal,
                },
                StorageNamespace::Save,
                StorageOperation::Read,
                save_slot_path(slot),
            );
        }
        if name == "CHKCHARADATA" {
            *status = HostDispatchStatus::Handled;
            let filename = dat_filename(string_argument_value(
                &request.arguments,
                0,
                "CHKCHARADATA",
            )?)?;
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostCheck {
                    request: request.id,
                    kind: era_runtime_save::SaveFileKind::Character,
                },
                StorageNamespace::Data,
                StorageOperation::Read,
                format!("chara_{filename}.dat"),
            );
        }
        if name == "SAVETEXT" {
            *status = HostDispatchStatus::Handled;
            let text = string_argument_value(&request.arguments, 0, "SAVETEXT")?;
            let Ok((namespace, mut path)) = text_storage_target(
                request
                    .arguments
                    .get(1)
                    .ok_or_else(|| RuntimeError::Internal("SAVETEXT target is missing".into()))?,
            ) else {
                return commit_integer_result(vm, request.id, 0);
            };
            if snake_resources && namespace == StorageNamespace::Data {
                let Some(normalized) = Self::resource_storage_path(&path, false) else {
                    return commit_integer_result(vm, request.id, 0);
                };
                path = normalized;
            }
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostFunctionWrite {
                    request: request.id,
                },
                namespace,
                StorageOperation::Write {
                    data: ProtocolBytes::new(text.as_bytes().to_vec()),
                    atomic_replace: true,
                    precondition: StoragePrecondition::Any,
                },
                path,
            );
        }
        if name == "LOADTEXT" {
            *status = HostDispatchStatus::Handled;
            let Ok((namespace, path)) = text_storage_target(
                request
                    .arguments
                    .first()
                    .ok_or_else(|| RuntimeError::Internal("LOADTEXT target is missing".into()))?,
            ) else {
                return commit_completion(
                    vm,
                    request.id,
                    VmHostCompletion::Ready(HostReady {
                        value: Some(VmValue::String(String::new())),
                        writes: Vec::new(),
                    }),
                );
            };
            let pending = if snake_resources && namespace == StorageNamespace::Data {
                let Some(normalized) = Self::resource_storage_path(&path, false) else {
                    return commit_completion(
                        vm,
                        request.id,
                        VmHostCompletion::Ready(HostReady {
                            value: Some(VmValue::String(String::new())),
                            writes: Vec::new(),
                        }),
                    );
                };
                PendingStorage::HostResourceText {
                    request: request.id,
                    path: normalized,
                    resource: false,
                }
            } else {
                PendingStorage::HostReadText {
                    request: request.id,
                }
            };
            let path = match &pending {
                PendingStorage::HostResourceText { path, .. } => path.clone(),
                _ => path,
            };
            return self.issue_host_storage(
                vm,
                request,
                pending,
                namespace,
                StorageOperation::Read,
                path,
            );
        }
        if name == "EXISTFILE" {
            *status = HostDispatchStatus::Handled;
            let Ok(path) =
                safe_relative_path(string_argument_value(&request.arguments, 0, "EXISTFILE")?)
            else {
                return commit_integer_result(vm, request.id, 0);
            };
            let path = if snake_resources {
                let Some(normalized) = Self::resource_storage_path(&path, false) else {
                    return commit_integer_result(vm, request.id, 0);
                };
                normalized
            } else {
                path
            };
            let pending = if snake_resources {
                PendingStorage::HostResourceStat {
                    request: request.id,
                    path: path.clone(),
                    resource: false,
                }
            } else {
                PendingStorage::HostStat {
                    request: request.id,
                }
            };
            return self.issue_host_storage(
                vm,
                request,
                pending,
                StorageNamespace::Data,
                StorageOperation::Stat,
                path,
            );
        }
        if name == "ENUMFILES" {
            *status = HostDispatchStatus::Handled;
            let Ok(directory) =
                safe_relative_directory(string_argument_value(&request.arguments, 0, "ENUMFILES")?)
            else {
                return commit_integer_result(vm, request.id, -1);
            };
            let pattern = request.arguments.get(1).and_then(|value| match value {
                VmValue::String(value) => Some(value.clone()),
                _ => None,
            });
            let recursive =
                matches!(request.arguments.get(2), Some(VmValue::Integer(value)) if *value != 0);
            let target = request.arguments.get(3).and_then(|value| match value {
                VmValue::StringPlace(place) => Some(place.as_ref().clone()),
                _ => None,
            });
            let directory = if snake_resources {
                let Some(normalized) = Self::resource_storage_path(&directory, true) else {
                    return commit_integer_result(vm, request.id, -1);
                };
                if !Self::resource_storage_pattern_valid(pattern.as_deref()) {
                    return commit_integer_result(vm, request.id, -1);
                }
                normalized
            } else {
                directory
            };
            let pending = if snake_resources {
                PendingStorage::HostResourceList {
                    request: request.id,
                    target,
                    directory: directory.clone(),
                    pattern: pattern.clone(),
                    recursive,
                    data_paths: None,
                }
            } else {
                PendingStorage::HostListFiles {
                    request: request.id,
                    target,
                    strip_character_dat: false,
                }
            };
            return self.issue_host_storage(
                vm,
                request,
                pending,
                StorageNamespace::Data,
                StorageOperation::List { pattern, recursive },
                directory,
            );
        }
        if name == "FIND_CHARADATA" {
            *status = HostDispatchStatus::Handled;
            let pattern = request
                .arguments
                .first()
                .and_then(|value| match value {
                    VmValue::String(value) => Some(value.as_str()),
                    _ => None,
                })
                .unwrap_or("*");
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostListFiles {
                    request: request.id,
                    target: None,
                    strip_character_dat: true,
                },
                StorageNamespace::Data,
                StorageOperation::List {
                    pattern: Some(format!("chara_{pattern}.dat")),
                    recursive: false,
                },
                String::new(),
            );
        }
        if name == "OUTPUTLOG" {
            *status = HostDispatchStatus::Handled;
            let filename = request
                .arguments
                .first()
                .and_then(|value| match value {
                    VmValue::String(value) if !value.is_empty() => Some(value.as_str()),
                    _ => None,
                })
                .unwrap_or("emuera.log");
            let path = safe_relative_path(filename)?;
            let hide_info = matches!(request.arguments.get(1), Some(VmValue::Integer(1)));
            let context = self.presentation_observation_context()?;
            return self.issue_host_service(
                vm,
                request,
                ExternalCompletion::SerializePhysicalHistory {
                    request: request.id,
                    context,
                    relative_path: path,
                },
                ServiceKind::PresentationQuery,
                SERIALIZE_PHYSICAL_HISTORY_OPERATION,
                SERIALIZE_PHYSICAL_HISTORY_OPERATION_VERSION,
                &SerializePhysicalHistoryRequest {
                    context,
                    title: self.presentation.snapshot().title,
                    hide_information: hide_info,
                },
            );
        }

        Ok(())
    }
}
