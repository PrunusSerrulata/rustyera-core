#[allow(clippy::needless_borrow, clippy::single_match_else)]
impl RuntimeSession {
    fn dispatch_control_runtime_queries(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        Self::dispatch_control_clear_memory(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_control_await(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_control_exit_and_font(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_control_configuration(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        Self::dispatch_control_varsize(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        Self::dispatch_control_existence(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        Self::dispatch_control_enumeration(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        Ok(())
    }

    fn dispatch_control_clear_memory(
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "CLEARMEMORY" {
            *status = HostDispatchStatus::Handled;
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::Integer(0)),
                    writes: Vec::new(),
                }),
            );
        }
        Ok(())
    }

    fn dispatch_control_await(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "AWAIT" {
            *status = HostDispatchStatus::Handled;
            let milliseconds = match request.argument(0) {
                None | Some(VmValue::Integer(0)) => 0,
                Some(VmValue::Integer(value @ 1..=10_000)) => *value,
                Some(VmValue::Integer(_)) => {
                    return complete_script_fault(
                        vm,
                        request,
                        erabasic_vm::ScriptFaultKind::Argument,
                        "AWAIT duration must be between 0 and 10000 milliseconds",
                    );
                }
                _ => {
                    return self.fault(
                        FaultCode::VmFault,
                        "AWAIT duration must be between 0 and 10000 milliseconds",
                        Some(request.origin.clone()),
                    );
                }
            };
            if vm
                .vm()
                .artifact()
                .manifest
                .compatibility
                .supports_snake_input()
            {
                if !self.environment.has(INPUT_DEVICE_PUMP_CAPABILITY, 1) {
                    return self.fault(
                        FaultCode::ServiceFailure,
                        "AWAIT requires negotiated device_pump",
                        Some(request.origin.clone()),
                    );
                }
                self.flush_presentation_for_observation()?;
                let after_event_sequence = self.device_input.event_sequence;
                self.device_input.clear_latches();
                return self.issue_host_service(
                    vm,
                    request,
                    ExternalCompletion::DevicePump {
                        request: request.id,
                        epoch: self.epoch.0,
                        after_event_sequence,
                        milliseconds: milliseconds.cast_unsigned(),
                    },
                    ServiceKind::InputState,
                    DEVICE_PUMP_OPERATION,
                    DEVICE_PUMP_OPERATION_VERSION,
                    &DevicePumpRequest {
                        epoch: self.epoch.0,
                        after_event_sequence,
                    },
                );
            }
            if milliseconds == 0 {
                return commit_completion(
                    vm,
                    request.id,
                    VmHostCompletion::Ready(HostReady::empty()),
                );
            }
            commit_completion(
                vm,
                request.id,
                VmHostCompletion::Pending {
                    stability: HostWaitStability::Transient,
                    rebind_payload: Vec::new(),
                },
            )?;
            self.operations.insert_delay(
                request.id,
                self.logical_time_ns
                    .saturating_add(milliseconds.cast_unsigned().saturating_mul(1_000_000)),
            );
            return Ok(());
        }

        Ok(())
    }

    fn dispatch_control_exit_and_font(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if matches!(
            name,
            "QUIT" | "FORCE_QUIT" | "QUIT_AND_RESTART" | "FORCE_QUIT_AND_RESTART"
        ) {
            *status = HostDispatchStatus::Handled;
            let exit = ExitRequested {
                reason: if name.ends_with("AND_RESTART") {
                    ExitReason::Restart
                } else {
                    ExitReason::Quit
                },
                force: name.starts_with("FORCE_"),
                runtime_revision: self.revision.saturating_add(1),
            };
            vm.cancel_fiber(request.fiber)
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            self.exit_requested = Some(exit);
            self.emit(RuntimeMessage::ExitRequested(exit), None)?;
            return self.set_phase(RuntimePhase::Stopping);
        }
        if name == "CHKFONT" {
            *status = HostDispatchStatus::Handled;
            let font = string_argument_value(request, 0, "CHKFONT")?;
            let available = self.available_fonts.contains(&font.to_lowercase());
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::Integer(i64::from(available))),
                    writes: Vec::new(),
                }),
            );
        }
        Ok(())
    }

    fn dispatch_control_configuration(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if matches!(name, "GETCONFIG" | "GETCONFIGS") {
            *status = HostDispatchStatus::Handled;
            return self.complete_control_configuration(vm, request, name);
        }
        Ok(())
    }

    fn complete_control_configuration(
        &self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
    ) -> Result<(), RuntimeError> {
        let key = string_argument_value(request, 0, name)?;
        let project = self
            .project_snapshot
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("GETCONFIG has no loaded project".into()))?;
        let value = if let Some(value) = project.configuration.get(key) {
            match (name, value.script_value()) {
                ("GETCONFIG", era_config::ScriptConfigValue::Integer(value)) => {
                    VmValue::Integer(value)
                }
                ("GETCONFIGS", era_config::ScriptConfigValue::String(value)) => {
                    VmValue::String(value)
                }
                ("GETCONFIG", era_config::ScriptConfigValue::String(_)) => {
                    return complete_script_fault(
                        vm,
                        request,
                        erabasic_vm::ScriptFaultKind::Argument,
                        "GETCONFIG value is a string; use GETCONFIGS",
                    );
                }
                ("GETCONFIGS", era_config::ScriptConfigValue::Integer(_)) => {
                    return complete_script_fault(
                        vm,
                        request,
                        erabasic_vm::ScriptFaultKind::Argument,
                        "GETCONFIGS value is an integer; use GETCONFIG",
                    );
                }
                _ => unreachable!(),
            }
        } else if name == "GETCONFIG" {
            let Some(value) = Self::integer_configuration_alias(vm, project, key) else {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Resolve,
                    format!("GETCONFIG does not expose configuration key {key:?}"),
                );
            };
            VmValue::Integer(value)
        } else {
            let Some(value) = Self::string_configuration_alias(vm, project, key) else {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Resolve,
                    format!("GETCONFIGS does not expose configuration key {key:?}"),
                );
            };
            VmValue::String(value)
        };
        commit_completion(
            vm,
            request.id,
            VmHostCompletion::Ready(HostReady {
                value: Some(value),
                writes: Vec::new(),
            }),
        )
    }

    fn integer_configuration_alias(
        vm: &RuntimeVm,
        project: &crate::project::NormalizedProjectSnapshot,
        key: &str,
    ) -> Option<i64> {
        let replace = &vm.vm().artifact().project_data.static_data.replace;
        Some(match key {
            "オートセーブを行なう" | "Make autosaves" => i64::from(project.auto_save),
            "単位の位置" | "Currency symbol position" => i64::from(project.money_first),
            "ウィンドウ幅" | "Window width" => i64::from(project.viewport_width),
            "PRINTCを並べる数" | "Items per line for PRINTC" => {
                i64::from(project.print_c_per_line)
            }
            "PRINTCの文字数" | "Number of Item characters for PRINTC" => {
                i64::from(project.print_c_length)
            }
            "フォントサイズ" | "Font size" => i64::from(project.font_size),
            "一行の高さ" | "Line height" => i64::from(project.line_height),
            "表示するセーブデータ数" | "Save data count per page" => {
                i64::from(project.save_slot_count)
            }
            "販売アイテム数" | "Max shop item storage" => {
                i64::from(project.maximum_shop_items)
            }
            "COM_ABLE初期値" | "COM_ABLE initial value" => {
                i64::from(replace.com_able_default)
            }
            "PBANDの初期値" | "PBAND initial value" => replace.pband_default,
            "RELATIONの初期値" | "RELATION initial value" => replace.relation_default,
            _ => return None,
        })
    }

    fn string_configuration_alias(
        vm: &RuntimeVm,
        project: &crate::project::NormalizedProjectSnapshot,
        key: &str,
    ) -> Option<String> {
        let replace = &vm.vm().artifact().project_data.static_data.replace;
        Some(match key {
            key if ["TextDrawingMode", "描画インターフェース", "Drawing interface"]
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(key.trim())) =>
            {
                "TEXTRENDERER".into()
            }
            "お金の単位" | "Currency symbol" => project.money_label.clone(),
            "起動時簡略表示" | "Loading message" => replace.load_label.clone(),
            "DRAWLINE文字" | "DRAWLINE characters" => replace.draw_line_string.clone(),
            "システムメニュー0" | "System menu 0" => replace.title_menu_string_0.clone(),
            "システムメニュー1" | "System menu 1" => replace.title_menu_string_1.clone(),
            "時間切れ表示" | "Time-up message" => replace.timeup_label.clone(),
            "BAR文字1" | "BAR character 1" => replace.bar_char_1.to_string(),
            "BAR文字2" | "BAR character 2" => replace.bar_char_2.to_string(),
            _ => return None,
        })
    }

    fn dispatch_control_varsize(
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "VARSIZE" {
            *status = HostDispatchStatus::Handled;
            if let Some(VmValue::IntegerPlace(place) | VmValue::StringPlace(place)) =
                request.argument(0)
            {
                let dimensions =
                    vm.host_place_dimensions(request.fiber, place)
                        .map_err(|error| {
                            RuntimeError::Internal(format!(
                                "VARSIZE variable reference is invalid: {error}"
                            ))
                        })?;
                let Some(VmValue::IntegerPlace(result)) = request.argument(1) else {
                    return Err(RuntimeError::Internal(
                        "statement VARSIZE requires a RESULT output place".into(),
                    ));
                };
                let writes = dimensions
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| {
                        let mut target = result.as_ref().clone();
                        target
                            .indices
                            .push(u64::try_from(index).unwrap_or(u64::MAX));
                        HostWrite {
                            target,
                            value: VmValue::Integer(i64::try_from(value).unwrap_or(i64::MAX)),
                        }
                    })
                    .collect();
                return commit_completion(
                    vm,
                    request.id,
                    VmHostCompletion::Ready(HostReady {
                        value: None,
                        writes,
                    }),
                );
            }
            let variable = string_argument_value(request, 0, "VARSIZE")?;
            let Some(dimensions) = vm.variable_dimensions(request.fiber, variable) else {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Resolve,
                    format!("VARSIZE argument is not a variable: {variable}"),
                );
            };
            let dimension = request
                .argument(1)
                .map(|_| integer_argument_value(request, 1))
                .transpose()?
                .unwrap_or(0);
            // Fixed VarsizeMethod narrows only this getter to a signed Int32.
            // Preserve its low bits; do not apply this conversion to timer/input APIs.
            let [b0, b1, b2, b3, ..] = dimension.to_le_bytes();
            let Ok(dimension) = usize::try_from(i32::from_le_bytes([b0, b1, b2, b3])) else {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Bounds,
                    "VARSIZE dimension must be non-negative",
                );
            };
            let Some(value) = dimensions.get(dimension).copied() else {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Bounds,
                    "VARSIZE dimension exceeds the variable rank",
                );
            };
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::Integer(i64::try_from(value).unwrap_or(i64::MAX))),
                    writes: Vec::new(),
                }),
            );
        }
        Ok(())
    }

    fn dispatch_control_existence(
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "EXISTFUNCTION" {
            *status = HostDispatchStatus::Handled;
            let function = string_argument_value(request, 0, "EXISTFUNCTION")?;
            let insensitive = request
                .argument(1)
                .map(|_| integer_argument_value(request, 1))
                .transpose()?
                .unwrap_or(0)
                != 0;
            let found = vm.vm().artifact().functions.iter().find(|candidate| {
                if insensitive {
                    candidate.name.eq_ignore_ascii_case(function)
                } else {
                    candidate.name == function
                }
            });
            let value = found.map_or(0, |function| match function.result {
                Some(erabasic_bytecode::BytecodeType::Integer) => 2,
                Some(erabasic_bytecode::BytecodeType::String) => 3,
                _ => 1,
            });
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::Integer(value)),
                    writes: Vec::new(),
                }),
            );
        }
        if name == "EXISTVAR" {
            *status = HostDispatchStatus::Handled;
            let variable = string_argument_value(request, 0, "EXISTVAR")?;
            let value = vm
                .vm()
                .artifact()
                .globals
                .iter()
                .find(|definition| {
                    definition.owner.is_none() && definition.name.eq_ignore_ascii_case(variable)
                })
                .map_or(0, |definition| {
                    let mut flags = match definition.value_type {
                        erabasic_bytecode::BytecodeType::Integer
                        | erabasic_bytecode::BytecodeType::IntegerPlace => 1,
                        erabasic_bytecode::BytecodeType::String
                        | erabasic_bytecode::BytecodeType::StringPlace => 2,
                    };
                    if !definition.mutable {
                        flags |= 4;
                    }
                    if definition.dimensions.len() == 2 {
                        flags |= 8;
                    } else if definition.dimensions.len() == 3 {
                        flags |= 16;
                    }
                    flags
                });
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::Integer(value)),
                    writes: Vec::new(),
                }),
            );
        }
        Ok(())
    }

    fn dispatch_control_enumeration(
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "GETDOINGFUNCTION" {
            *status = HostDispatchStatus::Handled;
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::String(request.origin.function_name.clone())),
                    writes: Vec::new(),
                }),
            );
        }
        if matches!(
            name,
            "ENUMFUNCBEGINSWITH"
                | "ENUMFUNCENDSWITH"
                | "ENUMFUNCWITH"
                | "ENUMVARBEGINSWITH"
                | "ENUMVARENDSWITH"
                | "ENUMVARWITH"
        ) {
            *status = HostDispatchStatus::Handled;
            let query = string_argument_value(request, 0, name)?;
            let target = request.argument(1).and_then(|value| match value {
                VmValue::StringPlace(place) => Some(place.as_ref().clone()),
                _ => None,
            });
            let mut names = Vec::new();
            if !query.is_empty() {
                let event_functions: BTreeSet<_> = vm
                    .vm()
                    .artifact()
                    .event_groups
                    .iter()
                    .flat_map(|group| {
                        group
                            .only
                            .iter()
                            .chain(&group.priority)
                            .chain(&group.normal)
                            .chain(&group.later)
                    })
                    .map(|entry| entry.function)
                    .collect();
                let candidates: Vec<&str> = if name.starts_with("ENUMFUNC") {
                    vm.vm()
                        .artifact()
                        .functions
                        .iter()
                        .filter(|function| !event_functions.contains(&function.key))
                        .map(|function| function.name.as_str())
                        .collect()
                } else {
                    let mut seen = BTreeSet::new();
                    vm.vm()
                        .artifact()
                        .globals
                        .iter()
                        .filter(|variable| {
                            variable.owner.is_none()
                                && seen.insert(variable.name.to_ascii_uppercase())
                        })
                        .map(|variable| variable.name.as_str())
                        .collect()
                };
                names.extend(
                    candidates
                        .into_iter()
                        .filter(|candidate| enum_name_matches(name, candidate, query))
                        .map(str::to_owned),
                );
            }
            let writes = string_array_writes(vm, target, &names);
            let output_length = i64::try_from(writes.len()).unwrap_or(i64::MAX);
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::Integer(output_length)),
                    writes,
                }),
            );
        }
        Ok(())
    }
}
