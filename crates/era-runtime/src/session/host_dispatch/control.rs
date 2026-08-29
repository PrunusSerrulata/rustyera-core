#[allow(clippy::wildcard_imports)]
use super::*;

#[allow(clippy::needless_borrow, clippy::single_match_else)]
impl RuntimeSession {
    #[allow(clippy::too_many_lines)]
    pub(super) fn dispatch_control(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
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

        if matches!(
            name.as_str(),
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
        if matches!(name.as_str(), "GETCONFIG" | "GETCONFIGS") {
            *status = HostDispatchStatus::Handled;
            let key = string_argument_value(request, 0, &name)?;
            let project = self
                .project_snapshot
                .as_ref()
                .ok_or_else(|| RuntimeError::Internal("GETCONFIG has no loaded project".into()))?;
            let value = if let Some(value) = project.configuration.get(key) {
                match (name.as_str(), value.script_value()) {
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
            } else {
                // Replace.csv is project data rather than ConfigData, but the reference
                // exposes these aliases through the same script API.
                let replace = &vm.vm().artifact().project_data.static_data.replace;
                if name == "GETCONFIG" {
                    let value = match key {
                        "オートセーブを行なう" | "Make autosaves" => {
                            i64::from(project.auto_save)
                        }
                        "単位の位置" | "Currency symbol position" => {
                            i64::from(project.money_first)
                        }
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
                        "RELATIONの初期値" | "RELATION initial value" => {
                            replace.relation_default
                        }
                        _ => {
                            return complete_script_fault(
                                vm,
                                request,
                                erabasic_vm::ScriptFaultKind::Resolve,
                                format!("GETCONFIG does not expose configuration key {key:?}"),
                            );
                        }
                    };
                    VmValue::Integer(value)
                } else {
                    let value = match key {
                        key if [
                            "TextDrawingMode",
                            "描画インターフェース",
                            "Drawing interface",
                        ]
                        .iter()
                        .any(|alias| alias.eq_ignore_ascii_case(key.trim())) =>
                        {
                            "TEXTRENDERER".into()
                        }
                        "お金の単位" | "Currency symbol" => project.money_label.clone(),
                        "起動時簡略表示" | "Loading message" => replace.load_label.clone(),
                        "DRAWLINE文字" | "DRAWLINE characters" => {
                            replace.draw_line_string.clone()
                        }
                        "システムメニュー0" | "System menu 0" => {
                            replace.title_menu_string_0.clone()
                        }
                        "システムメニュー1" | "System menu 1" => {
                            replace.title_menu_string_1.clone()
                        }
                        "時間切れ表示" | "Time-up message" => replace.timeup_label.clone(),
                        "BAR文字1" | "BAR character 1" => replace.bar_char_1.to_string(),
                        "BAR文字2" | "BAR character 2" => replace.bar_char_2.to_string(),
                        _ => {
                            return complete_script_fault(
                                vm,
                                request,
                                erabasic_vm::ScriptFaultKind::Resolve,
                                format!("GETCONFIGS does not expose configuration key {key:?}"),
                            );
                        }
                    };
                    VmValue::String(value)
                }
            };
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(value),
                    writes: Vec::new(),
                }),
            );
        }
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
            name.as_str(),
            "ENUMFUNCBEGINSWITH"
                | "ENUMFUNCENDSWITH"
                | "ENUMFUNCWITH"
                | "ENUMVARBEGINSWITH"
                | "ENUMVARENDSWITH"
                | "ENUMVARWITH"
        ) {
            *status = HostDispatchStatus::Handled;
            let query = string_argument_value(request, 0, &name)?;
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
                        .filter(|candidate| enum_name_matches(&name, candidate, query))
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
        if name == "GETTEXTBOX" {
            *status = HostDispatchStatus::Handled;
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::String(self.text_box.clone())),
                    writes: Vec::new(),
                }),
            );
        }
        if name == "SETTEXTBOX" {
            *status = HostDispatchStatus::Handled;
            string_argument_value(request, 0, &name)?.clone_into(&mut self.text_box);
            commit_integer_result(vm, request.id, 1)?;
            return self.emit_projection_state();
        }
        if name == "CLEARTEXTBOX" {
            *status = HostDispatchStatus::Handled;
            self.text_box.clear();
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_projection_state();
        }
        if matches!(name.as_str(), "MOVETEXTBOX" | "RESUMETEXTBOX") {
            *status = HostDispatchStatus::Handled;
            self.text_box_layout = if name == "MOVETEXTBOX" {
                TextBoxLayout {
                    x: integer_argument_value(request, 0)?,
                    y: integer_argument_value(request, 1)?,
                    width: integer_argument_value(request, 2)?,
                }
            } else {
                TextBoxLayout::default()
            };
            commit_integer_result(vm, request.id, 1)?;
            return self.emit_projection_state();
        }
        if name == "BITMAP_CACHE_ENABLE" {
            *status = HostDispatchStatus::Handled;
            // Reference compatibility no-op: bitmap line caching is only a
            // renderer performance hint and cannot affect portable semantics.
            let snake = self.project_snapshot.as_ref().is_some_and(|project| {
                project
                    .manifest
                    .compatibility
                    .supports_snake_display_state()
            });
            if snake && !self.bitmap_cache_notice_emitted {
                let code = "compat.bitmap_cache_enable_noop";
                self.emit(
                    RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                        context: self.vm_diagnostic_context(vm, &request.origin, code),
                        code: code.into(),
                        level: RuntimeLogLevel::Warning,
                        message: "BITMAP_CACHE_ENABLE is accepted as a compatibility no-op; RustyEra does not expose renderer bitmap-cache policy"
                            .into(),
                        source: protocol_execution_origin(request.origin.clone()).source,
                        notification: DiagnosticNotification::LogOnly,
                    }),
                    None,
                )?;
                self.bitmap_cache_notice_emitted = true;
            }
            return if snake {
                commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))
            } else {
                commit_integer_result(vm, request.id, 0)
            };
        }
        if name == "HOTKEY_STATE_INIT" {
            *status = HostDispatchStatus::Handled;
            let raw_size = integer_argument_value(request, 0)?;
            if raw_size < 0 {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Bounds,
                    "HOTKEY_STATE_INIT size must be non-negative",
                );
            }
            let size = usize::try_from(raw_size).map_err(|_| {
                RuntimeError::Internal("HOTKEY_STATE_INIT size must be non-negative".into())
            })?;
            self.hotkey_state = vec![0; size];
            commit_integer_result(vm, request.id, 0)?;
            return self.emit_projection_state();
        }
        if name == "HOTKEY_STATE" {
            *status = HostDispatchStatus::Handled;
            let Ok(index) = usize::try_from(integer_argument_value(request, 0)?) else {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Bounds,
                    "HOTKEY_STATE index must be non-negative",
                );
            };
            if request.arguments.len() < 2 {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Operation,
                    "HOTKEY_STATE dereferenced its absent second source argument",
                );
            }
            let value = integer_argument_value(request, 1)?;
            let Some(slot) = self.hotkey_state.get_mut(index) else {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Bounds,
                    "HOTKEY_STATE requires an initialized in-range index",
                );
            };
            *slot = value;
            commit_integer_result(vm, request.id, 0)?;
            return self.emit_projection_state();
        }
        if name == "FLOWINPUT" {
            *status = HostDispatchStatus::Handled;
            self.flow_input_default = integer_argument_value(request, 0)?;
            if request.arguments.len() > 1 {
                self.flow_input_enabled = integer_argument_value(request, 1)? != 0;
            }
            if request.arguments.len() > 2 {
                self.flow_input_can_skip = integer_argument_value(request, 2)? != 0;
            }
            if request.arguments.len() > 3 {
                self.flow_input_force_skip = integer_argument_value(request, 3)? != 0;
            }
            return commit_integer_result(vm, request.id, 0);
        }
        if name == "FLOWINPUTS" {
            *status = HostDispatchStatus::Handled;
            self.flow_input_string = integer_argument_value(request, 0)? != 0;
            if request.arguments.len() > 1 {
                string_argument_value(request, 1, &name)?
                    .clone_into(&mut self.flow_input_default_string);
            }
            return commit_integer_result(vm, request.id, 0);
        }
        if name == "BREAKBUTTON" {
            *status = HostDispatchStatus::Handled;
            self.button_generation = self.button_generation.saturating_add(1);
            self.presentation
                .set_button_generation(self.button_generation);
            self.command_intents.clear();
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_projection_state();
        }
        if name == "HTML_TAGSPLIT" {
            *status = HostDispatchStatus::Handled;
            let source = string_argument_value(request, 0, &name)?;
            let string_target = request.argument(1).and_then(vm_place);
            let integer_target = request
                .argument(2)
                .and_then(vm_place)
                .or_else(|| global_place(vm, "RESULT"));
            let Ok(values) = split_html_tags(source) else {
                let writes = integer_target
                    .into_iter()
                    .map(|target| HostWrite {
                        target,
                        value: VmValue::Integer(-1),
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
            };
            let mut writes = string_array_writes(vm, string_target, &values);
            if let Some(target) = integer_target {
                writes.push(HostWrite {
                    target,
                    value: VmValue::Integer(i64::try_from(values.len()).unwrap_or(i64::MAX)),
                });
            }
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: None,
                    writes,
                }),
            );
        }
        if name == "BARSTR" {
            *status = HostDispatchStatus::Handled;
            let value = integer_argument_value(request, 0)?;
            let maximum = integer_argument_value(request, 1)?;
            let length = integer_argument_value(request, 2)?;
            let replace = &vm.vm().artifact().project_data.static_data.replace;
            let bar = match format_bar_string(
                value,
                maximum,
                length,
                replace.bar_char_1,
                replace.bar_char_2,
            ) {
                Ok(value) => value,
                Err(BarStringError::NonPositiveMaximum) => {
                    return complete_script_fault(
                        vm,
                        request,
                        erabasic_vm::ScriptFaultKind::Argument,
                        "BARSTR maximum must be positive",
                    );
                }
                Err(BarStringError::InvalidLength) => {
                    return complete_script_fault(
                        vm,
                        request,
                        erabasic_vm::ScriptFaultKind::Argument,
                        "BARSTR length must be between 1 and 99",
                    );
                }
            };
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::String(bar)),
                    writes: Vec::new(),
                }),
            );
        }
        if matches!(name.as_str(), "MONEYSTR" | "TOSTR") {
            *status = HostDispatchStatus::Handled;
            let value = integer_argument_value(request, 0)?;
            let format = match request.argument(1) {
                None => None,
                Some(VmValue::String(format)) => Some(format.as_str()),
                Some(_) => {
                    return self.fault(
                        FaultCode::VmFault,
                        &format!("{name} argument 2 must be a string"),
                        Some(request.origin.clone()),
                    );
                }
            };
            if name == "TOSTR" {
                let formatted = match format_optional_era_integer(value, format) {
                    Ok(value) => value,
                    Err(error) => {
                        return complete_script_fault(
                            vm,
                            request,
                            erabasic_vm::ScriptFaultKind::Parse,
                            format!("{name} format is invalid: {error}"),
                        );
                    }
                };
                return commit_completion(
                    vm,
                    request.id,
                    VmHostCompletion::Ready(HostReady {
                        value: Some(VmValue::String(formatted)),
                        writes: Vec::new(),
                    }),
                );
            }
            let formatted = match format_optional_era_integer(value, format) {
                Ok(value) => value,
                Err(error) => {
                    return complete_script_fault(
                        vm,
                        request,
                        erabasic_vm::ScriptFaultKind::Parse,
                        format!("{name} format is invalid: {error}"),
                    );
                }
            };
            let project = self
                .project_snapshot
                .as_ref()
                .ok_or_else(|| RuntimeError::Internal("MONEYSTR has no loaded project".into()))?;
            let value = decorate_money_value(&formatted, project.money_first, &project.money_label);
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::String(value)),
                    writes: Vec::new(),
                }),
            );
        }
        if matches!(name.as_str(), "TOFULL" | "TOHALF") {
            *status = HostDispatchStatus::Handled;
            let value = string_argument_value(request, 0, &name)?;
            let converted = if name == "TOFULL" {
                to_full_width(value)
            } else {
                to_half_width(value)
            };
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::String(converted)),
                    writes: Vec::new(),
                }),
            );
        }
        if name == "CALLTRAIN" {
            *status = HostDispatchStatus::Handled;
            let Ok(count) = usize::try_from(integer_argument_value(request, 0)?) else {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Bounds,
                    "CALLTRAIN count must be non-negative",
                );
            };
            let Some(capacity) = vm
                .variable_dimensions(request.fiber, "SELECTCOM")
                .and_then(|dimensions| dimensions.first().copied())
                .and_then(|value| usize::try_from(value).ok())
            else {
                // Missing/malformed implicit storage is not a script count error.
                return self.fault(
                    FaultCode::VmFault,
                    "CALLTRAIN count must be smaller than SELECTCOM capacity",
                    Some(request.origin.clone()),
                );
            };
            if count >= capacity {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Bounds,
                    "CALLTRAIN count must be smaller than SELECTCOM capacity",
                );
            }
            self.controller.clear_continuous_train();
            for index in 0..count {
                self.controller
                    .continuous_commands
                    .push_back(read_runtime_integer(
                        vm,
                        "SELECTCOM",
                        &[u64::try_from(index + 1).unwrap_or(u64::MAX)],
                        None,
                    )?);
            }
            self.controller.continuous_train = true;
            self.controller.continuous_total = count;
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "STOPCALLTRAIN" {
            *status = HostDispatchStatus::Handled;
            if !self.controller.continuous_train {
                return commit_completion(
                    vm,
                    request.id,
                    VmHostCompletion::Ready(HostReady::empty()),
                );
            }
            // ClearCommands invokes CALLTRAINEND without a return address in the
            // reference engine. The STOPCALLTRAIN caller must therefore be
            // discarded before the system controller resumes its current phase.
            vm.cancel_fiber(request.fiber)
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            self.controller.clear();
            self.controller.clear_continuous_train();
            // STOPCALLTRAIN discards the active COM caller. Resume at the
            // post-command source-check phase explicitly instead of depending
            // on whichever value CALLTRAINEND leaves in RESULT:0.
            self.controller.step = SystemStep::TrainSourceCheck;
            self.skip_print = false;
            if !self.dispatch_system_function(vm, "CALLTRAINEND", false)? {
                return self.continue_system_flow(vm);
            }
            return Ok(());
        }
        if name == "DOTRAIN" {
            *status = HostDispatchStatus::Handled;
            let command = integer_argument_value(request, 0)?;
            let allowed_step = self.controller.allows_dotrain();
            let train_name_count = vm
                .vm()
                .artifact()
                .project_data
                .static_data
                .name_tables
                .get(&erabasic_data::NameTableKind::Train)
                .map_or(0, |table| table.names.len());
            if command < 0
                || self.controller.flow != Some(SystemFlow::Train)
                || !allowed_step
                || usize::try_from(command).map_or(true, |value| value >= train_name_count)
            {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Operation,
                    "DOTRAIN is not valid in this TRAIN phase or its command is outside TRAINNAME",
                );
            }
            vm.cancel_fiber(request.fiber)
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            self.controller.clear();
            self.controller.clear_continuous_train();
            reset_after_show_user(vm)?;
            self.controller.selected_command = Some(command);
            write_runtime_integer(vm, "SELECTCOM", &[], None, command)?;
            fill_runtime_variable(vm, "NOWEX", VmValue::Integer(0), true)?;
            self.controller.step = SystemStep::TrainEventCom;
            if !self.dispatch_system_event(vm, "EVENTCOM")? {
                return self.continue_system_flow(vm);
            }
            return Ok(());
        }
        if matches!(name.as_str(), "BEGIN" | "FORCE_BEGIN") {
            *status = HostDispatchStatus::Handled;
            let Some(VmValue::String(keyword)) = request.argument(0) else {
                return self.fault(
                    FaultCode::VmFault,
                    "BEGIN expects a system keyword",
                    Some(request.origin.clone()),
                );
            };
            let Some(flow) = SystemFlow::parse(keyword) else {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Argument,
                    format!("unknown BEGIN system keyword: {keyword}"),
                );
            };
            // BEGIN resets the console style at instruction execution time, even
            // when the actual system transition is deferred by EVENTSHOP.
            self.presentation.reset_style();
            if flow == SystemFlow::Shop {
                self.controller.shop_called_when_normal =
                    self.controller.flow == Some(SystemFlow::Normal);
            }
            self.controller.deferred_flow = Some(flow);
            if vm.fiber_frame_count(request.fiber).unwrap_or(0) > 1 {
                return commit_completion(vm, request.id, VmHostCompletion::ReturnCurrent(None));
            }
            // The pinned fork treats BEGIN and FORCE_BEGIN as the same forced
            // transition. Returning the issuing root approximates ProcessState's
            // Return(0), while retaining the remaining event handlers.
            vm.cancel_fiber(request.fiber)
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            let was_system_handler = self.controller.completed(request.fiber, None);
            if was_system_handler && !self.controller.is_complete() {
                return self.spawn_next_event(vm);
            }
            if self.controller.flow == Some(SystemFlow::Shop)
                && self.controller.step == SystemStep::ShopEvent
            {
                return self.continue_system_flow(vm);
            }
            self.controller.clear();
            if self.controller.continuous_train {
                self.controller.clear_continuous_train();
                self.controller.step = SystemStep::TrainBeginAfterCallTrainEnd;
                self.skip_print = true;
                if self.dispatch_system_function(vm, "CALLTRAINEND", false)? {
                    return Ok(());
                }
                self.skip_print = false;
            }
            let flow = self.controller.deferred_flow.take().unwrap_or(flow);
            self.controller.flow = Some(flow);
            return self.begin_flow(vm, flow);
        }

        Ok(())
    }
}
