//! Translation of VM host requests into runtime-owned semantic operations.

// This is one part of the same split `RuntimeSession` implementation.
#[allow(clippy::wildcard_imports)]
use super::*;

impl RuntimeSession {
    #[allow(clippy::single_match_else, clippy::too_many_lines)]
    pub(super) fn handle_host_call(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
    ) -> Result<(), RuntimeError> {
        if let Some(time) = self.candidate_clock {
            match request.import.contract.candidate {
                erabasic_bytecode::CandidatePolicy::Forbidden => {
                    return Err(RuntimeError::Internal(format!(
                        "{} is forbidden during candidate SAVEINFO execution",
                        request.import.import.name
                    )));
                }
                erabasic_bytecode::CandidatePolicy::FrozenClock => {
                    return complete_frozen_clock(vm, request, time);
                }
                erabasic_bytecode::CandidatePolicy::ReadOnly
                | erabasic_bytecode::CandidatePolicy::CloneCommit
                | erabasic_bytecode::CandidatePolicy::BufferedEffect => {}
            }
        }
        if request
            .import
            .import
            .namespace
            .eq_ignore_ascii_case("rustyera.extension")
        {
            return self.issue_extension(vm, request);
        }
        let name = request.import.import.name.to_ascii_uppercase();
        if name == "SKIPDISP" {
            self.skip_print = integer_argument_value(&request.arguments, 0)? != 0;
            self.user_defined_skip = self.skip_print;
            // Host calls execute while the caller-pumped drive loop temporarily
            // owns the VM, so RESULT must be resolved through that VM rather than
            // through the session's temporarily empty VM slot.
            return commit_host_result_write(vm, request.id, i64::from(self.skip_print));
        }
        if name == "SKIPLOG" {
            self.message_skip = integer_argument_value(&request.arguments, 0)? != 0;
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "NOSKIP" {
            self.saved_skip = self.skip_print;
            self.skip_print = false;
            return commit_integer_result(vm, request.id, 1);
        }
        if name == "ENDNOSKIP" {
            if self.saved_skip {
                self.skip_print = true;
            }
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "ASSERT" {
            if integer_argument_value(&request.arguments, 0)? == 0 {
                return self.fault(
                    FaultCode::VmFault,
                    "ASSERT failed",
                    Some(request.origin.clone()),
                );
            }
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "THROW" {
            let message = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            return self.fault(FaultCode::VmFault, &message, Some(request.origin.clone()));
        }
        if name == "FORCEKANA" {
            let mode = integer_argument_value(&request.arguments, 0)?;
            let Ok(mode) = u8::try_from(mode) else {
                return self.fault(
                    FaultCode::VmFault,
                    "FORCEKANA mode must be between 0 and 3",
                    Some(request.origin.clone()),
                );
            };
            if mode > 3 {
                return self.fault(
                    FaultCode::VmFault,
                    "FORCEKANA mode must be between 0 and 3",
                    Some(request.origin.clone()),
                );
            }
            self.force_kana_mode = mode;
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if matches!(name.as_str(), "UPCHECK" | "CUPCHECK") {
            let (character, character_scoped) = if name == "CUPCHECK" {
                let character = u64::try_from(integer_argument_value(&request.arguments, 0)?)
                    .map_err(|_| RuntimeError::Internal("character index is negative".into()))?;
                (character, true)
            } else {
                let target = read_runtime_integer(vm, "TARGET", &[], None)?;
                let Ok(character) = u64::try_from(target) else {
                    clear_upcheck_arrays(vm, false, None)?;
                    return commit_completion(
                        vm,
                        request.id,
                        VmHostCompletion::Ready(HostReady::empty()),
                    );
                };
                (character, false)
            };
            let lines = apply_upcheck(vm, character, character_scoped)?;
            if !self.skip_print {
                for line in lines {
                    self.presentation.append_print_text(line, false, true);
                }
            }
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if matches!(
            name.as_str(),
            "ISSKIP" | "MESSKIP" | "MOUSESKIP" | "LINEISEMPTY" | "ISACTIVE"
        ) {
            let value = match name.as_str() {
                "ISSKIP" => self.skip_print,
                "MESSKIP" | "MOUSESKIP" => self.message_skip,
                "LINEISEMPTY" => self.presentation.last_line_is_empty(),
                "ISACTIVE" => self.client_focused,
                _ => unreachable!(),
            };
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::Integer(i64::from(value))),
                    writes: Vec::new(),
                }),
            );
        }
        if name == "SETANIMETIMER" {
            let milliseconds = integer_argument_value(&request.arguments, 0)?;
            self.project_snapshot
                .as_mut()
                .ok_or_else(|| RuntimeError::Internal("SETANIMETIMER has no project".into()))?
                .resource_graph
                .set_animation_timer(milliseconds);
            self.sync_resource_replay();
            commit_integer_result(vm, request.id, 1)?;
            return self.emit_presentation();
        }
        if self.controller.step == SystemStep::TrainEventComEnd
            && matches!(name.as_str(), "WAIT" | "WAITANYKEY" | "FORCEWAIT" | "TWAIT")
        {
            self.controller.event_com_end_wait_required = false;
        }
        if self.skip_print && is_runtime_print_command(&name) {
            if self.user_defined_skip && is_input_command(&name) {
                return self.fault(
                    FaultCode::VmFault,
                    "an input command cannot execute while user SKIPDISP is active; wrap it in NOSKIP/ENDNOSKIP",
                    Some(request.origin.clone()),
                );
            }
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "AWAIT" {
            let milliseconds = match request.arguments.first() {
                None | Some(VmValue::Integer(0)) => 0,
                Some(VmValue::Integer(value @ 1..=10_000)) => *value,
                _ => {
                    return self.fault(
                        FaultCode::VmFault,
                        "AWAIT duration must be between 0 and 10000 milliseconds",
                        Some(request.origin.clone()),
                    );
                }
            };
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
            let font = string_argument_value(&request.arguments, 0, "CHKFONT")?;
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
            let key = string_argument_value(&request.arguments, 0, &name)?;
            let project = self
                .project_snapshot
                .as_ref()
                .ok_or_else(|| RuntimeError::Internal("GETCONFIG has no loaded project".into()))?;
            let value = if let Some(value) = project.configuration.get(key) {
                match (name.as_str(), value.script_value()) {
                    ("GETCONFIG", erabasic_config::ScriptConfigValue::Integer(value)) => {
                        VmValue::Integer(value)
                    }
                    ("GETCONFIGS", erabasic_config::ScriptConfigValue::String(value)) => {
                        VmValue::String(value)
                    }
                    ("GETCONFIG", erabasic_config::ScriptConfigValue::String(_)) => {
                        return self.fault(
                            FaultCode::VmFault,
                            "GETCONFIG value is a string; use GETCONFIGS",
                            Some(request.origin.clone()),
                        );
                    }
                    ("GETCONFIGS", erabasic_config::ScriptConfigValue::Integer(_)) => {
                        return self.fault(
                            FaultCode::VmFault,
                            "GETCONFIGS value is an integer; use GETCONFIG",
                            Some(request.origin.clone()),
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
                            return self.fault(
                                FaultCode::VmFault,
                                &format!("GETCONFIG does not expose configuration key {key:?}"),
                                Some(request.origin.clone()),
                            );
                        }
                    };
                    VmValue::Integer(value)
                } else {
                    let value = match key {
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
                            return self.fault(
                                FaultCode::VmFault,
                                &format!("GETCONFIGS does not expose configuration key {key:?}"),
                                Some(request.origin.clone()),
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
            if let Some(VmValue::IntegerPlace(place) | VmValue::StringPlace(place)) =
                request.arguments.first()
            {
                let dimensions =
                    vm.host_place_dimensions(request.fiber, place)
                        .map_err(|error| {
                            RuntimeError::Internal(format!(
                                "VARSIZE variable reference is invalid: {error}"
                            ))
                        })?;
                let Some(VmValue::IntegerPlace(result)) = request.arguments.get(1) else {
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
            let variable = string_argument_value(&request.arguments, 0, "VARSIZE")?;
            let dimensions = vm
                .variable_dimensions(request.fiber, variable)
                .ok_or_else(|| {
                    RuntimeError::Internal(format!(
                        "VARSIZE argument is not a variable: {variable}"
                    ))
                })?;
            let dimension = request
                .arguments
                .get(1)
                .map(|_| integer_argument_value(&request.arguments, 1))
                .transpose()?
                .unwrap_or(0);
            let dimension = usize::try_from(dimension).map_err(|_| {
                RuntimeError::Internal("VARSIZE dimension must be non-negative".into())
            })?;
            let value = dimensions.get(dimension).copied().ok_or_else(|| {
                RuntimeError::Internal("VARSIZE dimension exceeds the variable rank".into())
            })?;
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
            let function = string_argument_value(&request.arguments, 0, "EXISTFUNCTION")?;
            let insensitive = request
                .arguments
                .get(1)
                .map(|_| integer_argument_value(&request.arguments, 1))
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
            let variable = string_argument_value(&request.arguments, 0, "EXISTVAR")?;
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
            let query = string_argument_value(&request.arguments, 0, &name)?;
            let target = request.arguments.get(1).and_then(|value| match value {
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
            string_argument_value(&request.arguments, 0, &name)?.clone_into(&mut self.text_box);
            commit_integer_result(vm, request.id, 1)?;
            return self.emit_projection_state();
        }
        if name == "CLEARTEXTBOX" {
            self.text_box.clear();
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_projection_state();
        }
        if matches!(name.as_str(), "MOVETEXTBOX" | "RESUMETEXTBOX") {
            self.text_box_layout = if name == "MOVETEXTBOX" {
                TextBoxLayout {
                    x: integer_argument_value(&request.arguments, 0)?,
                    y: integer_argument_value(&request.arguments, 1)?,
                    width: integer_argument_value(&request.arguments, 2)?,
                }
            } else {
                TextBoxLayout::default()
            };
            commit_integer_result(vm, request.id, 1)?;
            return self.emit_projection_state();
        }
        if name == "BITMAP_CACHE_ENABLE" {
            // Reference compatibility no-op: bitmap line caching is only a
            // renderer performance hint and cannot affect portable semantics.
            return commit_integer_result(vm, request.id, 0);
        }
        if name == "HOTKEY_STATE_INIT" {
            let size =
                usize::try_from(integer_argument_value(&request.arguments, 0)?).map_err(|_| {
                    RuntimeError::Internal("HOTKEY_STATE_INIT size must be non-negative".into())
                })?;
            self.hotkey_state = vec![0; size];
            commit_integer_result(vm, request.id, 0)?;
            return self.emit_projection_state();
        }
        if name == "HOTKEY_STATE" {
            let index =
                usize::try_from(integer_argument_value(&request.arguments, 0)?).map_err(|_| {
                    RuntimeError::Internal("HOTKEY_STATE index must be non-negative".into())
                })?;
            let value = integer_argument_value(&request.arguments, 1)?;
            let Some(slot) = self.hotkey_state.get_mut(index) else {
                return self.fault(
                    FaultCode::VmFault,
                    "HOTKEY_STATE requires an initialized in-range index",
                    Some(request.origin.clone()),
                );
            };
            *slot = value;
            commit_integer_result(vm, request.id, 0)?;
            return self.emit_projection_state();
        }
        if name == "FLOWINPUT" {
            self.flow_input_default = integer_argument_value(&request.arguments, 0)?;
            if request.arguments.len() > 1 {
                self.flow_input_enabled = integer_argument_value(&request.arguments, 1)? != 0;
            }
            if request.arguments.len() > 2 {
                self.flow_input_can_skip = integer_argument_value(&request.arguments, 2)? != 0;
            }
            if request.arguments.len() > 3 {
                self.flow_input_force_skip = integer_argument_value(&request.arguments, 3)? != 0;
            }
            return commit_integer_result(vm, request.id, 0);
        }
        if name == "FLOWINPUTS" {
            self.flow_input_string = integer_argument_value(&request.arguments, 0)? != 0;
            if request.arguments.len() > 1 {
                string_argument_value(&request.arguments, 1, &name)?
                    .clone_into(&mut self.flow_input_default_string);
            }
            return commit_integer_result(vm, request.id, 0);
        }
        if name == "BREAKBUTTON" {
            self.button_generation = self.button_generation.saturating_add(1);
            self.presentation
                .set_button_generation(self.button_generation);
            self.command_intents.clear();
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_projection_state();
        }
        if matches!(name.as_str(), "HTML_ESCAPE" | "HTML_TOPLAINTEXT") {
            let source = string_argument_value(&request.arguments, 0, &name)?;
            let value = if name == "HTML_ESCAPE" {
                erabasic_html::escape(source)
            } else {
                match erabasic_html::to_plain_text(source) {
                    Ok(value) => value,
                    Err(_) => {
                        return self.fault(
                            FaultCode::VmFault,
                            "malformed HTML text",
                            Some(request.origin.clone()),
                        );
                    }
                }
            };
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::String(value)),
                    writes: Vec::new(),
                }),
            );
        }
        if name == "HTML_TAGSPLIT" {
            let source = string_argument_value(&request.arguments, 0, &name)?;
            let string_target = request.arguments.get(1).and_then(vm_place);
            let integer_target = request
                .arguments
                .get(2)
                .and_then(vm_place)
                .or_else(|| global_place(vm, "RESULT"));
            let values = match erabasic_html::split_tags(source) {
                Ok(tokens) => tokens
                    .into_iter()
                    .map(|token| match token {
                        erabasic_html::Token::Text(value) | erabasic_html::Token::Tag(value) => {
                            value
                        }
                    })
                    .collect::<Vec<_>>(),
                Err(_) => {
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
                }
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
            let value = integer_argument_value(&request.arguments, 0)?;
            let maximum = integer_argument_value(&request.arguments, 1)?;
            let length = integer_argument_value(&request.arguments, 2)?;
            if maximum <= 0 {
                return self.fault(
                    FaultCode::VmFault,
                    "BARSTR maximum must be positive",
                    Some(request.origin.clone()),
                );
            }
            if !(1..100).contains(&length) {
                return self.fault(
                    FaultCode::VmFault,
                    "BARSTR length must be between 1 and 99",
                    Some(request.origin.clone()),
                );
            }
            let replace = &vm.vm().artifact().project_data.static_data.replace;
            // Emuera performs the multiplication in an unchecked Int64 context.
            let filled = value.wrapping_mul(length) / maximum;
            let filled = filled.clamp(0, length);
            let empty = length - filled;
            let mut bar = String::from("[");
            bar.push_str(
                &replace
                    .bar_char_1
                    .to_string()
                    .repeat(usize::try_from(filled).unwrap_or(0)),
            );
            bar.push_str(
                &replace
                    .bar_char_2
                    .to_string()
                    .repeat(usize::try_from(empty).unwrap_or(0)),
            );
            bar.push(']');
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
            let value = integer_argument_value(&request.arguments, 0)?;
            let formatted = match request.arguments.get(1) {
                None => value.to_string(),
                Some(VmValue::String(format)) => match format_era_integer(value, format) {
                    Ok(value) => value,
                    Err(error) => {
                        return self.fault(
                            FaultCode::VmFault,
                            &format!("{name} format is invalid: {error}"),
                            Some(request.origin.clone()),
                        );
                    }
                },
                Some(_) => {
                    return self.fault(
                        FaultCode::VmFault,
                        &format!("{name} argument 2 must be a string"),
                        Some(request.origin.clone()),
                    );
                }
            };
            if name == "TOSTR" {
                return commit_completion(
                    vm,
                    request.id,
                    VmHostCompletion::Ready(HostReady {
                        value: Some(VmValue::String(formatted)),
                        writes: Vec::new(),
                    }),
                );
            }
            let project = self
                .project_snapshot
                .as_ref()
                .ok_or_else(|| RuntimeError::Internal("MONEYSTR has no loaded project".into()))?;
            let value = if project.money_first {
                format!("{}{formatted}", project.money_label)
            } else {
                format!("{formatted}{}", project.money_label)
            };
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
            let value = string_argument_value(&request.arguments, 0, &name)?;
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
            let count =
                usize::try_from(integer_argument_value(&request.arguments, 0)?).map_err(|_| {
                    RuntimeError::Internal("CALLTRAIN count must be non-negative".into())
                })?;
            let capacity = vm
                .variable_dimensions(request.fiber, "SELECTCOM")
                .and_then(|dimensions| dimensions.first().copied())
                .and_then(|value| usize::try_from(value).ok())
                .unwrap_or(0);
            if count >= capacity {
                return self.fault(
                    FaultCode::VmFault,
                    "CALLTRAIN count must be smaller than SELECTCOM capacity",
                    Some(request.origin.clone()),
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
            let command = integer_argument_value(&request.arguments, 0)?;
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
                return self.fault(
                    FaultCode::VmFault,
                    "DOTRAIN is not valid in this TRAIN phase or its command is outside TRAINNAME",
                    Some(request.origin.clone()),
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
            let Some(VmValue::String(keyword)) = request.arguments.first() else {
                return self.fault(
                    FaultCode::VmFault,
                    "BEGIN expects a system keyword",
                    Some(request.origin.clone()),
                );
            };
            let Some(flow) = SystemFlow::parse(keyword) else {
                return self.fault(
                    FaultCode::VmFault,
                    &format!("unknown BEGIN system keyword: {keyword}"),
                    Some(request.origin.clone()),
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
        if matches!(name.as_str(), "SAVEVAR" | "LOADVAR") {
            return self.fault(
                FaultCode::UnsupportedRuntimeFeature,
                &format!("{name} is not implemented by the pinned reference runtime"),
                Some(request.origin.clone()),
            );
        }
        if name == "PUTFORM" {
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
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostLoadGlobal {
                    request: request.id,
                },
                StorageNamespace::GlobalSave,
                StorageOperation::Read,
                "global.sav".into(),
            );
        }
        if name == "SAVECHARA" {
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
            let filename =
                dat_filename(string_argument_value(&request.arguments, 0, "LOADCHARA")?)?;
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostLoadCharacters {
                    request: request.id,
                },
                StorageNamespace::Data,
                StorageOperation::Read,
                format!("chara_{filename}.dat"),
            );
        }
        if name == "CHKDATA" {
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
            let text = string_argument_value(&request.arguments, 0, "SAVETEXT")?;
            let Ok((namespace, path)) = text_storage_target(
                request
                    .arguments
                    .get(1)
                    .ok_or_else(|| RuntimeError::Internal("SAVETEXT target is missing".into()))?,
            ) else {
                return commit_integer_result(vm, request.id, 0);
            };
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
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostReadText {
                    request: request.id,
                },
                namespace,
                StorageOperation::Read,
                path,
            );
        }
        if name == "EXISTFILE" {
            let Ok(path) =
                safe_relative_path(string_argument_value(&request.arguments, 0, "EXISTFILE")?)
            else {
                return commit_integer_result(vm, request.id, 0);
            };
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostStat {
                    request: request.id,
                },
                StorageNamespace::Data,
                StorageOperation::Stat,
                path,
            );
        }
        if name == "ENUMFILES" {
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
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostListFiles {
                    request: request.id,
                    target,
                    strip_character_dat: false,
                },
                StorageNamespace::Data,
                StorageOperation::List { pattern, recursive },
                directory,
            );
        }
        if name == "FIND_CHARADATA" {
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
            let context = self.projection_query_context();
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
        if let Some(mut pending) = input_wait(
            request,
            self.allocate_wait(),
            self.allocate_interaction(),
            self.logical_time_ns,
        ) {
            // Bind automatic buttons still buffered on the current logical line before
            // transferring the menu choices into the wait. Otherwise a trailing
            // `PRINTFORM "[58] ..."` is rendered as clickable but its token remains
            // outside the active INPUT/INPUTS validator.
            let count = self.presentation.pending_auto_button_values().len();
            let tokens = (0..count)
                .map(|_| self.allocate_interaction())
                .collect::<Vec<_>>();
            for (token, value) in self.presentation.bind_pending_auto_buttons(&tokens) {
                self.command_intents.insert(token, VmValue::Integer(value));
            }
            if matches!(
                pending.wait.kind,
                WaitKind::IntegerValue
                    | WaitKind::StringValue
                    | WaitKind::IntegerButton
                    | WaitKind::StringButton
            ) {
                pending.choices = std::mem::take(&mut self.command_intents);
            }
            if pending.wait.stop_message_skip {
                self.message_skip = false;
            }
            let timed_value_input = matches!(
                name.as_str(),
                "TINPUT" | "TONEINPUT" | "TINPUTS" | "TONEINPUTS"
            );
            let untimed_value_input = matches!(
                name.as_str(),
                "INPUT"
                    | "ONEINPUT"
                    | "INPUTS"
                    | "ONEINPUTS"
                    | "BINPUT"
                    | "ONEBINPUT"
                    | "BINPUTS"
                    | "ONEBINPUTS"
            );
            let (mouse_index, can_skip_index) = if timed_value_input { (4, 5) } else { (1, 2) };
            if self.message_skip
                && (timed_value_input || untimed_value_input)
                && request.arguments.get(can_skip_index).is_some()
            {
                let mouse = matches!(
                    request.arguments.get(mouse_index),
                    Some(VmValue::Integer(value)) if *value != 0
                );
                let target = pending.result_name.as_deref().and_then(|result| {
                    if mouse {
                        global_place_at(vm, result, 1)
                    } else {
                        global_place(vm, result)
                    }
                });
                let value = pending
                    .wait
                    .default_value
                    .as_ref()
                    .map_or(VmValue::Integer(0), protocol_to_vm);
                let writes = target
                    .map(|target| vec![HostWrite { target, value }])
                    .unwrap_or_default();
                return commit_completion(
                    vm,
                    request.id,
                    VmHostCompletion::Ready(HostReady {
                        value: None,
                        writes,
                    }),
                );
            }
            let stability = match pending.wait.stability {
                WaitStability::StableInput => HostWaitStability::StableInput,
                WaitStability::Transient => HostWaitStability::Transient,
            };
            commit_completion(
                vm,
                request.id,
                VmHostCompletion::Pending {
                    stability,
                    rebind_payload: encode_canonical(&pending.wait)?,
                },
            )?;
            return self.open_wait(pending, false);
        }
        if name == "GETLINESTR" {
            let Some(VmValue::String(pattern)) = request.arguments.first() else {
                return self.fault(
                    FaultCode::VmFault,
                    "GETLINESTR expects a string pattern",
                    Some(request.origin.clone()),
                );
            };
            let value = match logical_line_string(
                pattern,
                usize::try_from(self.line_columns).unwrap_or(usize::MAX),
            ) {
                Ok(value) => value,
                Err(message) => {
                    return self.fault(FaultCode::VmFault, message, Some(request.origin.clone()));
                }
            };
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::String(value)),
                    writes: Vec::new(),
                }),
            );
        }
        if matches!(
            name.as_str(),
            "CLIENTWIDTH" | "CLIENTHEIGHT" | "PRINTCPERLINE" | "PRINTCLENGTH"
        ) {
            let project = self.project_snapshot.as_ref().ok_or_else(|| {
                RuntimeError::Internal("layout query has no loaded project".into())
            })?;
            let value = match name.as_str() {
                "CLIENTWIDTH" => self.client_width,
                "CLIENTHEIGHT" => self.client_height,
                "PRINTCPERLINE" => project.print_c_per_line,
                _ => project.print_c_length,
            };
            let result = VmValue::Integer(i64::from(value));
            let writes = request
                .arguments
                .first()
                .and_then(vm_place)
                .map(|target| {
                    vec![HostWrite {
                        target,
                        value: result.clone(),
                    }]
                })
                .unwrap_or_default();
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: writes.is_empty().then_some(result),
                    writes,
                }),
            );
        }
        if name == "HTML_POPPRINTINGSTR" {
            let count = self.presentation.pending_auto_button_values().len();
            let tokens = (0..count)
                .map(|_| self.allocate_interaction())
                .collect::<Vec<_>>();
            for (token, value) in self.presentation.bind_pending_auto_buttons(&tokens) {
                self.command_intents.insert(token, VmValue::Integer(value));
            }
            let value = self.presentation.pop_printing_html();
            commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::String(value)),
                    writes: Vec::new(),
                }),
            )?;
            return self.emit_presentation();
        }
        if matches!(name.as_str(), "GETDISPLAYLINE" | "HTML_GETPRINTEDSTR") {
            let context = self.projection_query_context();
            let index = request
                .arguments
                .first()
                .and_then(|value| match value {
                    VmValue::Integer(value) => Some(*value),
                    _ => None,
                })
                .unwrap_or_default();
            if index < 0 {
                if name == "GETDISPLAYLINE" {
                    return commit_completion(
                        vm,
                        request.id,
                        VmHostCompletion::Ready(HostReady {
                            value: Some(VmValue::String(String::new())),
                            writes: Vec::new(),
                        }),
                    );
                }
                return self.fault(
                    FaultCode::VmFault,
                    "HTML_GETPRINTEDSTR line number must be non-negative",
                    Some(request.origin.clone()),
                );
            }
            let (operation, version, completion) = if name == "GETDISPLAYLINE" {
                (
                    GET_DISPLAY_LINE_OPERATION,
                    GET_DISPLAY_LINE_OPERATION_VERSION,
                    ProjectionStringOperation::DisplayLine,
                )
            } else {
                (
                    HTML_GET_PRINTED_STR_OPERATION,
                    HTML_GET_PRINTED_STR_OPERATION_VERSION,
                    ProjectionStringOperation::PrintedHtml,
                )
            };
            return self.issue_host_service(
                vm,
                request,
                ExternalCompletion::ProjectionString {
                    request: request.id,
                    operation: completion,
                    context,
                },
                ServiceKind::PresentationQuery,
                operation,
                version,
                &ProjectionStringIndexRequest { context, index },
            );
        }
        if matches!(
            name.as_str(),
            "HTML_STRINGLEN" | "HTML_SUBSTRING" | "HTML_STRINGLINES"
        ) {
            let context = self.projection_query_context();
            let markup = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let argument = request
                .arguments
                .get(1)
                .and_then(|value| match value {
                    VmValue::Integer(value) => Some(*value),
                    _ => None,
                })
                .unwrap_or_default();
            let (operation, version) = match name.as_str() {
                "HTML_STRINGLEN" => (HTML_STRING_LEN_OPERATION, HTML_STRING_LEN_OPERATION_VERSION),
                "HTML_SUBSTRING" => (HTML_SUBSTRING_OPERATION, HTML_SUBSTRING_OPERATION_VERSION),
                _ => (
                    HTML_STRING_LINES_OPERATION,
                    HTML_STRING_LINES_OPERATION_VERSION,
                ),
            };
            let completion = if name == "HTML_SUBSTRING" {
                ExternalCompletion::HtmlSubstring {
                    request: request.id,
                    context,
                }
            } else {
                ExternalCompletion::ProjectionInteger {
                    request: request.id,
                    context,
                }
            };
            return self.issue_host_service(
                vm,
                request,
                completion,
                ServiceKind::PresentationQuery,
                operation,
                version,
                &HtmlMeasureRequest {
                    context,
                    markup,
                    argument,
                },
            );
        }
        if matches!(
            name.as_str(),
            "DRAWLINE" | "CUSTOMDRAWLINE" | "DRAWLINEFORM"
        ) {
            let pattern = request
                .arguments
                .first()
                .map_or_else(|| "-".into(), display_value);
            self.presentation.append_separator(pattern);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "CLEARLINE" {
            let count = request
                .arguments
                .first()
                .and_then(|value| match value {
                    VmValue::Integer(value) => usize::try_from(*value).ok(),
                    _ => None,
                })
                .unwrap_or(1);
            self.presentation.delete_last_lines(count);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "HTML_PRINT" {
            let markup = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let mut document = match erabasic_html::parse_document(&markup) {
                Ok(document) => document,
                Err(error) => {
                    return self.fault(
                        FaultCode::VmFault,
                        &format!(
                            "HTML_PRINT {:?} at UTF-8 bytes {}..{}",
                            error.kind, error.start, error.end
                        ),
                        Some(request.origin.clone()),
                    );
                }
            };
            bind_html_document(self, &mut document)?;
            if request.arguments.get(1).map_or(0, integer_value_or_zero) != 0 {
                self.presentation.append_html_inline(document);
            } else {
                self.presentation.append_html(document);
            }
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "HTML_PRINT_ISLAND" {
            let markup = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let mut document = match erabasic_html::parse_document(&markup) {
                Ok(document) => document,
                Err(error) => {
                    return self.fault(
                        FaultCode::VmFault,
                        &format!(
                            "HTML_PRINT_ISLAND {:?} at UTF-8 bytes {}..{}",
                            error.kind, error.start, error.end
                        ),
                        Some(request.origin.clone()),
                    );
                }
            };
            bind_html_document(self, &mut document)?;
            self.presentation.append_html_island(document);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "HTML_PRINT_ISLAND_CLEAR" {
            self.presentation.clear_html_island();
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if matches!(name.as_str(), "BAR" | "BARL") {
            let value = integer_argument_value(&request.arguments, 0)?;
            let maximum = integer_argument_value(&request.arguments, 1)?;
            let length = integer_argument_value(&request.arguments, 2)?;
            let replace = &vm.vm().artifact().project_data.static_data.replace;
            let bar = match make_bar(
                value,
                maximum,
                length,
                replace.bar_char_1,
                replace.bar_char_2,
            ) {
                Ok(value) => value,
                Err(message) => {
                    return self.fault(FaultCode::VmFault, message, Some(request.origin.clone()));
                }
            };
            self.presentation
                .append_print_text(bar, false, name == "BARL");
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if matches!(
            name.as_str(),
            "DEBUGPRINT" | "DEBUGPRINTL" | "DEBUGPRINTFORM" | "DEBUGPRINTFORML"
        ) {
            let mut appended = String::new();
            for value in &request.arguments {
                appended.push_str(&display_value(value));
            }
            if name.ends_with('L') {
                appended.push_str("\r\n");
            }
            let cursor = self
                .debug_output_base
                .saturating_add(u64::try_from(self.debug_output.len()).unwrap_or(u64::MAX));
            self.debug_output.push_str(&appended);
            if self.debug_output.len() > 1_048_576 {
                let remove = self.debug_output.len() - 1_048_576;
                let boundary = self
                    .debug_output
                    .char_indices()
                    .find_map(|(index, _)| (index >= remove).then_some(index))
                    .unwrap_or(remove);
                self.debug_output.drain(..boundary);
                self.debug_output_base = self
                    .debug_output_base
                    .saturating_add(u64::try_from(boundary).unwrap_or(u64::MAX));
            }
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            if self.debug_output_subscribed {
                self.emit_debug(
                    DebugMessage::Response(DebugResponse::ScriptOutput(ScriptOutputChunk {
                        cursor,
                        next_cursor: cursor
                            .saturating_add(u64::try_from(appended.len()).unwrap_or(u64::MAX)),
                        text: appended,
                        truncated: false,
                    })),
                    None,
                )?;
            }
            return Ok(());
        }
        if name == "DEBUGCLEAR" {
            self.debug_output_base = self
                .debug_output_base
                .saturating_add(u64::try_from(self.debug_output.len()).unwrap_or(u64::MAX));
            self.debug_output.clear();
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "ALIGNMENT" {
            let alignment = match request.arguments.first() {
                Some(VmValue::String(value)) if value.eq_ignore_ascii_case("CENTER") => {
                    LineAlignment::Center
                }
                Some(VmValue::String(value)) if value.eq_ignore_ascii_case("RIGHT") => {
                    LineAlignment::Right
                }
                Some(VmValue::String(value)) if value.eq_ignore_ascii_case("LEFT") => {
                    LineAlignment::Left
                }
                _ => {
                    return self.fault(
                        FaultCode::VmFault,
                        "ALIGNMENT expects LEFT, CENTER, or RIGHT",
                        Some(request.origin.clone()),
                    );
                }
            };
            self.presentation.set_alignment(alignment);
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "FONTSTYLE" {
            let bits = integer_argument_value(&request.arguments, 0)?;
            self.presentation.set_font_style(bits);
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if matches!(name.as_str(), "FONTBOLD" | "FONTITALIC" | "FONTREGULAR") {
            match name.as_str() {
                "FONTBOLD" => self.presentation.set_bold(true),
                "FONTITALIC" => self.presentation.set_italic(true),
                _ => self.presentation.clear_font_style(),
            }
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "SETFONT" {
            let family = request.arguments.first().map(display_value);
            self.presentation.set_font(family);
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "SETCOLOR" {
            let color = match color_argument_value(&request.arguments) {
                Ok(color) => color,
                Err(error) => {
                    return self.fault(FaultCode::VmFault, error, Some(request.origin.clone()));
                }
            };
            self.presentation.set_foreground(color);
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if matches!(name.as_str(), "SETCOLORBYNAME" | "SETBGCOLORBYNAME") {
            let color_name = string_argument_value(&request.arguments, 0, &name)?;
            let Some(color) = named_color(color_name) else {
                return self.fault(
                    FaultCode::VmFault,
                    "unknown or transparent color name",
                    Some(request.origin.clone()),
                );
            };
            if name == "SETCOLORBYNAME" {
                self.presentation.set_foreground(color);
            } else {
                self.presentation.set_background(color);
            }
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return if name == "SETBGCOLORBYNAME" {
                self.emit_presentation()
            } else {
                Ok(())
            };
        }
        if name == "RESETCOLOR" {
            self.presentation.reset_foreground();
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "RESETBGCOLOR" {
            self.presentation.reset_background();
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "REDRAW" {
            let flags = integer_argument_value(&request.arguments, 0)?;
            self.presentation.set_redraw(flags & 1 != 0);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            self.emit_presentation()?;
            return if flags & 2 != 0 {
                self.emit_effect(EffectKind::PresentNow {
                    presentation_revision: self.presentation.revision(),
                })
            } else {
                Ok(())
            };
        }
        if matches!(name.as_str(), "CURRENTALIGN" | "GETFONT") {
            let value = if name == "GETFONT" {
                self.presentation.font()
            } else {
                match self.presentation.alignment() {
                    LineAlignment::Left => "LEFT",
                    LineAlignment::Center => "CENTER",
                    LineAlignment::Right => "RIGHT",
                }
                .into()
            };
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::String(value)),
                    writes: Vec::new(),
                }),
            );
        }
        if matches!(
            name.as_str(),
            "CURRENTREDRAW"
                | "GETBGCOLOR"
                | "GETCOLOR"
                | "GETDEFBGCOLOR"
                | "GETDEFCOLOR"
                | "GETFOCUSCOLOR"
                | "GETSTYLE"
        ) {
            let value = match name.as_str() {
                "CURRENTREDRAW" => i64::from(self.presentation.redraw_enabled()),
                "GETBGCOLOR" => self.presentation.background_rgb(),
                "GETCOLOR" => self.presentation.foreground_rgb(),
                "GETDEFBGCOLOR" => self.presentation.default_background_rgb(),
                "GETDEFCOLOR" => self.presentation.default_foreground_rgb(),
                "GETFOCUSCOLOR" => self.presentation.focus_rgb(),
                _ => self.presentation.style_bits(),
            };
            return commit_integer_result(vm, request.id, value);
        }
        if name == "SETBGCOLOR" {
            let color = match color_argument_value(&request.arguments) {
                Ok(color) => color,
                Err(error) => {
                    return self.fault(FaultCode::VmFault, error, Some(request.origin.clone()));
                }
            };
            self.presentation.set_background(color);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "SETBGIMAGE" {
            let resource = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let depth = request.arguments.get(1).map_or(0, integer_value_or_zero);
            let opacity = request.arguments.get(2).map_or(255, integer_value_or_zero);
            let exists = self
                .project_snapshot
                .as_ref()
                .and_then(|project| project.resource_graph.sprite(&resource))
                .is_some();
            if exists {
                self.presentation.add_background(resource, depth, opacity);
            }
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "REMOVEBGIMAGE" {
            let resource = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            if !self.presentation.remove_background(&resource) {
                return self.fault(
                    FaultCode::VmFault,
                    "REMOVEBGIMAGE did not find the requested background",
                    Some(request.origin.clone()),
                );
            }
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "CLEARBGIMAGE" {
            self.presentation.clear_backgrounds();
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "CBGCLEAR" {
            self.presentation.clear_client_backgrounds();
            commit_integer_result(vm, request.id, 1)?;
            return self.emit_presentation();
        }
        if name.starts_with("TOOLTIP_") {
            let result = match name.as_str() {
                "TOOLTIP_SETCOLOR" => {
                    let foreground = integer_argument_value(&request.arguments, 0)?;
                    let background = integer_argument_value(&request.arguments, 1)?;
                    if !(0..=0xff_ffff).contains(&foreground)
                        || !(0..=0xff_ffff).contains(&background)
                    {
                        Err("tooltip color is out of range")
                    } else {
                        self.presentation.set_tooltip_colors(foreground, background);
                        Ok(())
                    }
                }
                "TOOLTIP_SETDELAY" => self
                    .presentation
                    .set_tooltip_delay(integer_argument_value(&request.arguments, 0)?),
                "TOOLTIP_SETDURATION" => self
                    .presentation
                    .set_tooltip_duration(integer_argument_value(&request.arguments, 0)?),
                "TOOLTIP_SETFONT" => {
                    self.presentation.set_tooltip_font(
                        request
                            .arguments
                            .first()
                            .map_or_else(String::new, display_value),
                    );
                    Ok(())
                }
                "TOOLTIP_SETFONTSIZE" => self
                    .presentation
                    .set_tooltip_font_size(integer_argument_value(&request.arguments, 0)?),
                "TOOLTIP_CUSTOM" => {
                    self.presentation
                        .set_tooltip_custom(integer_argument_value(&request.arguments, 0)? != 0);
                    Ok(())
                }
                "TOOLTIP_FORMAT" => {
                    self.presentation
                        .set_tooltip_format(integer_argument_value(&request.arguments, 0)?);
                    Ok(())
                }
                "TOOLTIP_IMG" => {
                    self.presentation
                        .set_tooltip_images(integer_argument_value(&request.arguments, 0)? != 0);
                    Ok(())
                }
                _ => Err("unsupported tooltip operation"),
            };
            if let Err(message) = result {
                return self.fault(FaultCode::VmFault, message, Some(request.origin.clone()));
            }
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "PRINT_IMG" {
            let resource = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let mut cursor = 1;
            let hover = request.arguments.get(cursor).and_then(|value| match value {
                VmValue::String(value) => Some(value.clone()).filter(|value| !value.is_empty()),
                _ => None,
            });
            if request
                .arguments
                .get(cursor)
                .is_some_and(|value| matches!(value, VmValue::String(_)))
            {
                cursor += 1;
            }
            let mask = request.arguments.get(cursor).and_then(|value| match value {
                VmValue::String(value) => Some(value.clone()).filter(|value| !value.is_empty()),
                _ => None,
            });
            if request
                .arguments
                .get(cursor)
                .is_some_and(|value| matches!(value, VmValue::String(_)))
            {
                cursor += 1;
            }
            let lengths = mixed_lengths(&request.arguments[cursor..])?;
            let exists = self
                .project_snapshot
                .as_ref()
                .and_then(|project| project.resource_graph.sprite(&resource))
                .is_some();
            if exists {
                self.presentation.append_image_with_options(
                    resource,
                    hover,
                    mask,
                    lengths.first().copied(),
                    lengths.get(1).copied(),
                    lengths.get(2).copied(),
                    None,
                );
            } else {
                let mut fallback = format!("<img src='{}'", erabasic_html::escape(&resource));
                if let Some(value) = &hover {
                    let _ = write!(fallback, " srcb='{}'", erabasic_html::escape(value));
                }
                if let Some(value) = &mask {
                    let _ = write!(fallback, " srcm='{}'", erabasic_html::escape(value));
                }
                let line_height = self.presentation.line_height();
                append_mixed_html_attribute(&mut fallback, "height", lengths.get(1), line_height);
                append_mixed_html_attribute(&mut fallback, "width", lengths.first(), line_height);
                append_mixed_html_attribute(&mut fallback, "ypos", lengths.get(2), line_height);
                fallback.push('>');
                self.presentation.append_image_with_options(
                    resource,
                    hover,
                    mask,
                    lengths.first().copied(),
                    lengths.get(1).copied(),
                    lengths.get(2).copied(),
                    Some(fallback),
                );
            }
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "PRINT_RECT" {
            let parameters = mixed_lengths(&request.arguments)?;
            self.presentation.append_shape("rect", parameters);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "PRINT_SPACE" {
            let widths = mixed_lengths(&request.arguments)?;
            let Some(width) = widths.into_iter().next() else {
                return self.fault(
                    FaultCode::VmFault,
                    "PRINT_SPACE requires one length",
                    Some(request.origin.clone()),
                );
            };
            self.presentation.append_space(width);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "EXISTSOUND" {
            let resource = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let exists = self
                .project_snapshot
                .as_ref()
                .is_some_and(|project| project.resource_graph.contains_audio(&resource));
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::Integer(i64::from(exists))),
                    writes: Vec::new(),
                }),
            );
        }
        if matches!(
            name.as_str(),
            "SPRITECREATED" | "SPRITEWIDTH" | "SPRITEHEIGHT" | "SPRITEPOSX" | "SPRITEPOSY"
        ) {
            let resource = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let value = self
                .project_snapshot
                .as_ref()
                .and_then(|project| project.resource_graph.sprite(&resource))
                .map_or(0, |sprite| match name.as_str() {
                    "SPRITECREATED" => 1,
                    "SPRITEWIDTH" => i64::from(sprite.width),
                    "SPRITEHEIGHT" => i64::from(sprite.height),
                    "SPRITEPOSX" => i64::from(sprite.position_x),
                    _ => i64::from(sprite.position_y),
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
        if name == "SPRITEGETCOLOR" {
            let resource = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let x = integer_argument_value(&request.arguments, 1)?;
            let y = integer_argument_value(&request.arguments, 2)?;
            let Some((resource_id, digest, x, y)) = self
                .project_snapshot
                .as_ref()
                .and_then(|project| project.resource_graph.sprite_pixel_request(&resource, x, y))
            else {
                return commit_completion(
                    vm,
                    request.id,
                    VmHostCompletion::Ready(HostReady {
                        value: Some(VmValue::Integer(-1)),
                        writes: Vec::new(),
                    }),
                );
            };
            return self.issue_host_service(
                vm,
                request,
                ExternalCompletion::SpritePixel {
                    request: request.id,
                },
                ServiceKind::Image,
                IMAGE_PIXEL_OPERATION,
                IMAGE_PIXEL_OPERATION_VERSION,
                &ImagePixelRequest {
                    resource_id,
                    content_digest: ProtocolBytes::new(digest),
                    x,
                    y,
                },
            );
        }
        if matches!(name.as_str(), "SPRITEMOVE" | "SPRITESETPOS") {
            let resource = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let x = i32::try_from(integer_argument_value(&request.arguments, 1)?).unwrap_or(0);
            let y = i32::try_from(integer_argument_value(&request.arguments, 2)?).unwrap_or(0);
            let changed = self.project_snapshot.as_mut().is_some_and(|project| {
                project
                    .resource_graph
                    .move_sprite(&resource, x, y, name == "SPRITEMOVE")
            });
            return self.complete_graphics_result(vm, request.id, i64::from(changed));
        }
        if name == "GCREATEFROMFILE" {
            let id = integer_argument_value(&request.arguments, 0)?;
            if self
                .project_snapshot
                .as_ref()
                .and_then(|project| project.resource_graph.canvas_state(id))
                .is_some()
            {
                return self.complete_graphics_result(vm, request.id, 0);
            }
            let filename = string_argument_value(&request.arguments, 1, &name)?;
            // Emuera treats a missing or unusable image filename as an ordinary
            // creation failure. Keep unsafe paths away from the frontend without
            // exposing portable path validation as a runtime-internal fault.
            let Ok(path) = safe_relative_path(filename) else {
                return self.complete_graphics_result(vm, request.id, 0);
            };
            let relative = request
                .arguments
                .get(2)
                .is_some_and(|value| integer_value_or_zero(value) != 0);
            if !relative {
                let created = self.project_snapshot.as_mut().is_some_and(|project| {
                    project
                        .resource_graph
                        .create_canvas_from_resource(id, &path)
                });
                return self.complete_graphics_result(vm, request.id, i64::from(created));
            }
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::GraphicsImageRead {
                    request: request.id,
                    canvas_id: id,
                },
                StorageNamespace::Data,
                StorageOperation::Read,
                path,
            );
        }
        if name == "GLOAD" {
            let id = integer_argument_value(&request.arguments, 0)?;
            let file_no = integer_argument_value(&request.arguments, 1)?;
            if !(0..=i64::from(i32::MAX)).contains(&file_no)
                || self
                    .project_snapshot
                    .as_ref()
                    .and_then(|project| project.resource_graph.canvas_state(id))
                    .is_some()
            {
                return commit_integer_result(vm, request.id, 0);
            }
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::GraphicsImageRead {
                    request: request.id,
                    canvas_id: id,
                },
                StorageNamespace::Save,
                StorageOperation::Read,
                format!("img{file_no:04}.png"),
            );
        }
        if name == "GSAVE" {
            let id = integer_argument_value(&request.arguments, 0)?;
            let file_no = integer_argument_value(&request.arguments, 1)?;
            let observation = self
                .project_snapshot
                .as_ref()
                .and_then(|project| project.resource_graph.canvas_observation(id));
            let Some((_, _, canvas_revision)) = observation else {
                return commit_integer_result(vm, request.id, 0);
            };
            if !(0..=i64::from(i32::MAX)).contains(&file_no) {
                return commit_integer_result(vm, request.id, 0);
            }
            // PNG encoding is an optional frontend raster capability. Emuera reports
            // GSAVE failure to the script when the image cannot be encoded; a text-only
            // client must not fault the whole session merely because it lacks a renderer.
            if self
                .service_capabilities
                .get(&(ServiceKind::Canvas, ENCODE_CANVAS_PNG_OPERATION.to_owned()))
                != Some(&ENCODE_CANVAS_PNG_OPERATION_VERSION)
            {
                return commit_integer_result(vm, request.id, 0);
            }
            return self.issue_host_service(
                vm,
                request,
                ExternalCompletion::EncodeCanvasPng {
                    request: request.id,
                    relative_path: format!("img{file_no:04}.png"),
                },
                ServiceKind::Canvas,
                ENCODE_CANVAS_PNG_OPERATION,
                ENCODE_CANVAS_PNG_OPERATION_VERSION,
                &EncodeCanvasPngRequest {
                    canvas_id: id,
                    canvas_revision,
                },
            );
        }
        if name == "GCREATE" {
            let id = integer_argument_value(&request.arguments, 0)?;
            let width = integer_argument_value(&request.arguments, 1)?;
            let height = integer_argument_value(&request.arguments, 2)?;
            let result = self
                .project_snapshot
                .as_mut()
                .ok_or_else(|| RuntimeError::Internal("GCREATE has no loaded project".into()))?
                .resource_graph
                .create_canvas(id, width, height);
            let created = match result {
                Ok(value) => value,
                Err(message) => {
                    return self.fault(FaultCode::VmFault, message, Some(request.origin.clone()));
                }
            };
            return self.complete_graphics_result(vm, request.id, i64::from(created));
        }
        if name == "GDISPOSE" {
            let id = integer_argument_value(&request.arguments, 0)?;
            let disposed = self
                .project_snapshot
                .as_mut()
                .is_some_and(|project| project.resource_graph.dispose_canvas(id));
            return self.complete_graphics_result(vm, request.id, i64::from(disposed));
        }
        if matches!(name.as_str(), "GCREATED" | "GWIDTH" | "GHEIGHT") {
            let id = integer_argument_value(&request.arguments, 0)?;
            let state = self
                .project_snapshot
                .as_ref()
                .and_then(|project| project.resource_graph.canvas_state(id));
            let value = match (name.as_str(), state) {
                ("GCREATED", Some(_)) => 1,
                ("GWIDTH", Some((width, _))) => i64::from(width),
                ("GHEIGHT", Some((_, height))) => i64::from(height),
                _ => 0,
            };
            return commit_integer_result(vm, request.id, value);
        }
        if matches!(
            name.as_str(),
            "GGETBRUSH" | "GGETPEN" | "GGETPENWIDTH" | "GGETFONTSIZE" | "GGETFONTSTYLE"
        ) {
            let id = integer_argument_value(&request.arguments, 0)?;
            let value = self
                .project_snapshot
                .as_ref()
                .and_then(|project| project.resource_graph.canvas_style(id))
                .map_or(
                    0,
                    |(brush, pen, width, _, font_size, font_style)| match name.as_str() {
                        "GGETBRUSH" => i64::from(brush),
                        "GGETPEN" => i64::from(pen),
                        "GGETPENWIDTH" => width,
                        "GGETFONTSIZE" => font_size,
                        _ => i64::from(font_style),
                    },
                );
            return commit_integer_result(vm, request.id, value);
        }
        if name == "GGETFONT" {
            let id = integer_argument_value(&request.arguments, 0)?;
            let value = self
                .project_snapshot
                .as_ref()
                .and_then(|project| project.resource_graph.canvas_style(id))
                .map_or_else(String::new, |(_, _, _, family, _, _)| family.to_owned());
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::String(value)),
                    writes: Vec::new(),
                }),
            );
        }
        if matches!(
            name.as_str(),
            "GSETBRUSH" | "GSETPEN" | "GDASHSTYLE" | "GSETFONT"
        ) {
            let id = integer_argument_value(&request.arguments, 0)?;
            let changed = match name.as_str() {
                "GSETBRUSH" => {
                    let color = checked_argb(integer_argument_value(&request.arguments, 1)?)?;
                    self.project_snapshot
                        .as_mut()
                        .is_some_and(|project| project.resource_graph.set_canvas_brush(id, color))
                }
                "GSETPEN" => {
                    let color = checked_argb(integer_argument_value(&request.arguments, 1)?)?;
                    let width = integer_argument_value(&request.arguments, 2)?;
                    self.project_snapshot.as_mut().is_some_and(|project| {
                        project.resource_graph.set_canvas_pen(id, color, width)
                    })
                }
                "GDASHSTYLE" => {
                    let style = integer_argument_value(&request.arguments, 1)?;
                    let offset = integer_argument_value(&request.arguments, 2)?;
                    self.project_snapshot.as_mut().is_some_and(|project| {
                        project.resource_graph.set_canvas_dash(id, style, offset)
                    })
                }
                "GSETFONT" => {
                    let family = string_argument_value(&request.arguments, 1, &name)?.to_owned();
                    let size = integer_argument_value(&request.arguments, 2)?;
                    let style = request
                        .arguments
                        .get(3)
                        .and_then(|value| match value {
                            VmValue::Integer(value) => Some(*value),
                            _ => None,
                        })
                        .unwrap_or_default();
                    self.project_snapshot.as_mut().is_some_and(|project| {
                        project.resource_graph.set_canvas_font(
                            id,
                            family,
                            size,
                            u8::try_from(style & 0x0f).expect("masked font style fits u8"),
                        )
                    })
                }
                _ => unreachable!(),
            };
            return self.complete_graphics_result(vm, request.id, i64::from(changed));
        }
        if name == "GGETTEXTSIZE" {
            let context = self.projection_query_context();
            let text = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let font_family = request
                .arguments
                .get(1)
                .map_or_else(String::new, display_value);
            let font_size = integer_argument_value(&request.arguments, 2)?;
            let style = request
                .arguments
                .get(3)
                .and_then(|value| match value {
                    VmValue::Integer(value) => Some(*value),
                    _ => None,
                })
                .unwrap_or_default();
            return self.issue_host_service(
                vm,
                request,
                ExternalCompletion::TextExtent {
                    request: request.id,
                    context,
                },
                ServiceKind::FontMetrics,
                GGET_TEXT_SIZE_OPERATION,
                GGET_TEXT_SIZE_OPERATION_VERSION,
                &TextExtentRequest {
                    context,
                    text,
                    font_family,
                    font_size,
                    style_bits: u8::try_from(style & 0x0f).expect("masked style fits u8"),
                },
            );
        }
        if name == "GGETCOLOR" {
            let canvas_id = integer_argument_value(&request.arguments, 0)?;
            let x = integer_argument_value(&request.arguments, 1)?;
            let y = integer_argument_value(&request.arguments, 2)?;
            let observation = self
                .project_snapshot
                .as_ref()
                .and_then(|project| project.resource_graph.canvas_observation(canvas_id));
            let Some((width, height, canvas_revision)) = observation else {
                return commit_integer_result(vm, request.id, -1);
            };
            let (Ok(x), Ok(y)) = (i32::try_from(x), i32::try_from(y)) else {
                return commit_integer_result(vm, request.id, -1);
            };
            if x < 0
                || y < 0
                || u32::try_from(x).map_or(true, |x| x >= width)
                || u32::try_from(y).map_or(true, |y| y >= height)
            {
                return commit_integer_result(vm, request.id, -1);
            }
            let context = self.projection_query_context();
            return self.issue_host_service(
                vm,
                request,
                ExternalCompletion::CanvasPixel {
                    request: request.id,
                    context,
                    canvas_revision,
                },
                ServiceKind::Canvas,
                SAMPLE_CANVAS_PIXEL_OPERATION,
                SAMPLE_CANVAS_PIXEL_OPERATION_VERSION,
                &CanvasPixelRequest {
                    context,
                    canvas_id,
                    canvas_revision,
                    point: CanvasPoint { x, y },
                },
            );
        }
        if name == "GCLEAR" {
            let id = integer_argument_value(&request.arguments, 0)?;
            let color = integer_argument_value(&request.arguments, 1)?;
            let rectangle = if request.arguments.len() == 6 {
                Some([
                    i32_argument_value(&request.arguments, 2)?,
                    i32_argument_value(&request.arguments, 3)?,
                    i32_argument_value(&request.arguments, 4)?,
                    i32_argument_value(&request.arguments, 5)?,
                ])
            } else {
                None
            };
            let changed = self
                .project_snapshot
                .as_mut()
                .is_some_and(|project| project.resource_graph.clear_canvas(id, color, rectangle));
            return self.complete_graphics_result(vm, request.id, i64::from(changed));
        }
        if name == "GSETCOLOR" {
            let id = integer_argument_value(&request.arguments, 0)?;
            let color = checked_argb(integer_argument_value(&request.arguments, 1)?)?;
            let point = [
                i32_argument_value(&request.arguments, 2)?,
                i32_argument_value(&request.arguments, 3)?,
            ];
            let changed = self
                .project_snapshot
                .as_mut()
                .is_some_and(|project| project.resource_graph.set_canvas_pixel(id, color, point));
            return self.complete_graphics_result(vm, request.id, i64::from(changed));
        }
        if name == "GFILLRECTANGLE" {
            let id = integer_argument_value(&request.arguments, 0)?;
            let rectangle = [
                i32_argument_value(&request.arguments, 1)?,
                i32_argument_value(&request.arguments, 2)?,
                i32_argument_value(&request.arguments, 3)?,
                i32_argument_value(&request.arguments, 4)?,
            ];
            let changed = self
                .project_snapshot
                .as_mut()
                .is_some_and(|project| project.resource_graph.fill_canvas_rectangle(id, rectangle));
            return self.complete_graphics_result(vm, request.id, i64::from(changed));
        }
        if name == "GDRAWLINE" {
            let id = integer_argument_value(&request.arguments, 0)?;
            let start = [
                i32_argument_value(&request.arguments, 1)?,
                i32_argument_value(&request.arguments, 2)?,
            ];
            let end = [
                i32_argument_value(&request.arguments, 3)?,
                i32_argument_value(&request.arguments, 4)?,
            ];
            let changed = self
                .project_snapshot
                .as_mut()
                .is_some_and(|project| project.resource_graph.draw_canvas_line(id, start, end));
            return self.complete_graphics_result(vm, request.id, i64::from(changed));
        }
        if name == "GDRAWTEXT" {
            let id = integer_argument_value(&request.arguments, 0)?;
            let text = string_argument_value(&request.arguments, 1, &name)?.to_owned();
            let point = if request.arguments.len() == 4 {
                [
                    i32_argument_value(&request.arguments, 2)?,
                    i32_argument_value(&request.arguments, 3)?,
                ]
            } else {
                [0, 0]
            };
            let style = self
                .project_snapshot
                .as_ref()
                .and_then(|project| project.resource_graph.canvas_style(id));
            if style.is_none() {
                return commit_integer_result(vm, request.id, 0);
            }
            let (font_family, font_size, style_bits) = self
                .project_snapshot
                .as_ref()
                .and_then(|project| project.resource_graph.canvas_style(id))
                .map_or_else(
                    || {
                        (
                            self.presentation.font(),
                            100,
                            u8::try_from(self.presentation.style_bits()).unwrap_or_default(),
                        )
                    },
                    |(_, _, _, family, size, style)| {
                        if family.is_empty() {
                            (
                                self.presentation.font(),
                                100,
                                u8::try_from(self.presentation.style_bits()).unwrap_or_default(),
                            )
                        } else {
                            (family.to_owned(), size, style)
                        }
                    },
                );
            self.sync_resource_replay();
            let context = self.projection_query_context();
            return self.issue_host_service(
                vm,
                request,
                ExternalCompletion::DrawTextExtent {
                    request: request.id,
                    context,
                    canvas_id: id,
                    text: text.clone(),
                    point,
                },
                ServiceKind::FontMetrics,
                GGET_TEXT_SIZE_OPERATION,
                GGET_TEXT_SIZE_OPERATION_VERSION,
                &TextExtentRequest {
                    context,
                    text: string_argument_value(&request.arguments, 1, &name)?.to_owned(),
                    font_family,
                    font_size,
                    style_bits,
                },
            );
        }
        if matches!(
            name.as_str(),
            "GDRAWG" | "GDRAWGWITHMASK" | "GDRAWGWITHROTATE"
        ) {
            let id = integer_argument_value(&request.arguments, 0)?;
            let source_id = integer_argument_value(&request.arguments, 1)?;
            let (source, destination, mask, rotation, rotation_center) = match name.as_str() {
                "GDRAWG" => (
                    Some([
                        i32_argument_value(&request.arguments, 6)?,
                        i32_argument_value(&request.arguments, 7)?,
                        i32_argument_value(&request.arguments, 8)?,
                        i32_argument_value(&request.arguments, 9)?,
                    ]),
                    Some([
                        i32_argument_value(&request.arguments, 2)?,
                        i32_argument_value(&request.arguments, 3)?,
                        i32_argument_value(&request.arguments, 4)?,
                        i32_argument_value(&request.arguments, 5)?,
                    ]),
                    None,
                    0,
                    None,
                ),
                "GDRAWGWITHMASK" => {
                    let source_size = self
                        .project_snapshot
                        .as_ref()
                        .and_then(|project| project.resource_graph.canvas_state(source_id));
                    let Some((width, height)) = source_size else {
                        return commit_integer_result(vm, request.id, 0);
                    };
                    let mask_id = integer_argument_value(&request.arguments, 2)?;
                    let destination_id = id;
                    let destination_point = [
                        i32_argument_value(&request.arguments, 3)?,
                        i32_argument_value(&request.arguments, 4)?,
                    ];
                    let graph = self
                        .project_snapshot
                        .as_ref()
                        .map(|project| &project.resource_graph);
                    let mask_matches = graph
                        .and_then(|graph| graph.canvas_state(mask_id))
                        .is_some_and(|size| size == (width, height));
                    let destination_fits = graph
                        .and_then(|graph| graph.canvas_state(destination_id))
                        .is_some_and(|(destination_width, destination_height)| {
                            i64::from(destination_point[0]) + i64::from(width)
                                <= i64::from(destination_width)
                                && i64::from(destination_point[1]) + i64::from(height)
                                    <= i64::from(destination_height)
                        });
                    if !mask_matches || !destination_fits {
                        return commit_integer_result(vm, request.id, 0);
                    }
                    let rectangle = [
                        destination_point[0],
                        destination_point[1],
                        i32::try_from(width).unwrap_or(i32::MAX),
                        i32::try_from(height).unwrap_or(i32::MAX),
                    ];
                    (None, Some(rectangle), Some(mask_id), 0, None)
                }
                _ => {
                    let angle = integer_argument_value(&request.arguments, 2)?;
                    let source_size = self
                        .project_snapshot
                        .as_ref()
                        .and_then(|project| project.resource_graph.canvas_state(source_id));
                    let Some((source_width, source_height)) = source_size else {
                        return commit_integer_result(vm, request.id, 0);
                    };
                    let center = if request.arguments.len() == 5 {
                        [
                            i32_argument_value(&request.arguments, 3)?,
                            i32_argument_value(&request.arguments, 4)?,
                        ]
                    } else {
                        [
                            i32::try_from(source_width / 2).unwrap_or(i32::MAX),
                            i32::try_from(source_height / 2).unwrap_or(i32::MAX),
                        ]
                    };
                    (None, None, None, angle.saturating_mul(1_000), Some(center))
                }
            };
            let color_matrix = if name == "GDRAWG" {
                request
                    .arguments
                    .get(10)
                    .map(|value| read_color_matrix(vm, request.fiber, value))
                    .transpose()?
            } else {
                None
            };
            let changed = self.project_snapshot.as_mut().is_some_and(|project| {
                project.resource_graph.draw_canvas(
                    id,
                    source_id,
                    source,
                    destination,
                    color_matrix,
                    mask,
                    rotation,
                    rotation_center,
                )
            });
            return self.complete_graphics_result(vm, request.id, i64::from(changed));
        }
        if name == "GDRAWSPRITE" {
            let id = integer_argument_value(&request.arguments, 0)?;
            let sprite = request
                .arguments
                .get(1)
                .map_or_else(String::new, display_value);
            let destination = match request.arguments.len() {
                2 => None,
                4 => Some([
                    i32_argument_value(&request.arguments, 2)?,
                    i32_argument_value(&request.arguments, 3)?,
                    self.project_snapshot
                        .as_ref()
                        .and_then(|project| project.resource_graph.sprite(&sprite))
                        .map_or(0, |value| i32::try_from(value.width).unwrap_or(i32::MAX)),
                    self.project_snapshot
                        .as_ref()
                        .and_then(|project| project.resource_graph.sprite(&sprite))
                        .map_or(0, |value| i32::try_from(value.height).unwrap_or(i32::MAX)),
                ]),
                _ => Some([
                    i32_argument_value(&request.arguments, 2)?,
                    i32_argument_value(&request.arguments, 3)?,
                    i32_argument_value(&request.arguments, 4)?,
                    i32_argument_value(&request.arguments, 5)?,
                ]),
            };
            let color_matrix = request
                .arguments
                .get(6)
                .map(|value| read_color_matrix(vm, request.fiber, value))
                .transpose()?;
            let changed = self.project_snapshot.as_mut().is_some_and(|project| {
                project
                    .resource_graph
                    .draw_sprite(id, &sprite, destination, color_matrix)
            });
            return self.complete_graphics_result(vm, request.id, i64::from(changed));
        }
        if name == "SPRITEANIMECREATE" {
            let sprite = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let width = integer_argument_value(&request.arguments, 1)?;
            let height = integer_argument_value(&request.arguments, 2)?;
            if !(1..=8_192).contains(&width) || !(1..=8_192).contains(&height) {
                return self.fault(
                    FaultCode::VmFault,
                    "animation sprite dimensions are out of range",
                    Some(request.origin.clone()),
                );
            }
            let created = self.project_snapshot.as_mut().is_some_and(|project| {
                project
                    .resource_graph
                    .create_animation_sprite(&sprite, width, height)
            });
            return self.complete_graphics_result(vm, request.id, i64::from(created));
        }
        if name == "SPRITEANIMEADDFRAME" {
            let sprite = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let canvas_id = integer_argument_value(&request.arguments, 1)?;
            let rectangle = [
                i32_argument_value(&request.arguments, 2)?,
                i32_argument_value(&request.arguments, 3)?,
                i32_argument_value(&request.arguments, 4)?,
                i32_argument_value(&request.arguments, 5)?,
            ];
            let offset = [
                i32_argument_value(&request.arguments, 6)?,
                i32_argument_value(&request.arguments, 7)?,
            ];
            let delay = integer_argument_value(&request.arguments, 8)?;
            let added = self.project_snapshot.as_mut().is_some_and(|project| {
                project
                    .resource_graph
                    .add_animation_frame(&sprite, canvas_id, rectangle, offset, delay)
            });
            return self.complete_graphics_result(vm, request.id, i64::from(added));
        }
        if name == "SPRITECREATE" {
            let sprite = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let id = integer_argument_value(&request.arguments, 1)?;
            let rectangle = if request.arguments.len() == 6 {
                Some([
                    i32_argument_value(&request.arguments, 2)?,
                    i32_argument_value(&request.arguments, 3)?,
                    i32_argument_value(&request.arguments, 4)?,
                    i32_argument_value(&request.arguments, 5)?,
                ])
            } else {
                None
            };
            let created = self.project_snapshot.as_mut().is_some_and(|project| {
                project
                    .resource_graph
                    .create_canvas_sprite(&sprite, id, rectangle)
            });
            return self.complete_graphics_result(vm, request.id, i64::from(created));
        }
        if name == "SPRITEDISPOSE" {
            let sprite = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let disposed = self
                .project_snapshot
                .as_mut()
                .is_some_and(|project| project.resource_graph.dispose_sprite(&sprite));
            return self.complete_graphics_result(vm, request.id, i64::from(disposed));
        }
        if name == "SPRITEDISPOSEALL" {
            let include_static = integer_argument_value(&request.arguments, 0)? != 0;
            let count = self.project_snapshot.as_mut().map_or(0, |project| {
                project.resource_graph.dispose_sprites(include_static)
            });
            return self.complete_graphics_result(
                vm,
                request.id,
                i64::try_from(count).unwrap_or(i64::MAX),
            );
        }
        if matches!(name.as_str(), "PLAYBGM" | "PLAYSOUND") {
            let resource = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let bgm = name == "PLAYBGM";
            let resolved_resource = self
                .project_snapshot
                .as_ref()
                .and_then(|project| project.resource_graph.audio_path(&resource))
                .map(str::to_owned);
            let Some(resource) = resolved_resource else {
                return commit_completion(
                    vm,
                    request.id,
                    VmHostCompletion::Ready(HostReady::empty()),
                );
            };
            self.presentation.set_audio(resource.clone(), bgm, true);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            self.emit_presentation()?;
            if self.presentation.projects_audio() && self.client_audio_available {
                return self.emit_effect(EffectKind::Audio(AudioEffect {
                    channel_id: u64::from(bgm),
                    action: AudioEffectAction::Play,
                    resource_id: Some(resource),
                    repeat_count: if bgm { -1 } else { 1 },
                    volume_millionths: 1_000_000,
                }));
            }
            if self.presentation.projects_audio() {
                return self.emit_audio_unavailable();
            }
            return Ok(());
        }
        if matches!(name.as_str(), "STOPBGM" | "STOPSOUND") {
            let bgm = name == "STOPBGM";
            self.presentation.set_audio(String::new(), bgm, false);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            self.emit_presentation()?;
            if self.presentation.projects_audio() && self.client_audio_available {
                return self.emit_effect(EffectKind::Audio(AudioEffect {
                    channel_id: u64::from(bgm),
                    action: AudioEffectAction::Stop,
                    resource_id: None,
                    repeat_count: 0,
                    volume_millionths: 0,
                }));
            }
            if self.presentation.projects_audio() {
                return self.emit_audio_unavailable();
            }
            return Ok(());
        }
        if matches!(name.as_str(), "SETBGMVOLUME" | "SETSOUNDVOLUME") {
            let bgm = name == "SETBGMVOLUME";
            let volume = integer_argument_value(&request.arguments, 0)?;
            self.presentation.set_audio_volume(bgm, volume);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            self.emit_presentation()?;
            if self.presentation.projects_audio() && self.client_audio_available {
                return self.emit_effect(EffectKind::Audio(AudioEffect {
                    channel_id: u64::from(bgm),
                    action: AudioEffectAction::SetVolume,
                    resource_id: None,
                    repeat_count: 0,
                    volume_millionths: u32::try_from(volume.clamp(0, 100)).unwrap_or_default()
                        * 10_000,
                }));
            }
            if self.presentation.projects_audio() {
                return self.emit_audio_unavailable();
            }
            return Ok(());
        }
        if matches!(
            name.as_str(),
            "PRINTBUTTON" | "PRINTBUTTONC" | "PRINTBUTTONLC"
        ) {
            let text = request
                .arguments
                .first()
                .map_or_else(String::new, display_value)
                .replace('\n', "");
            let value = request
                .arguments
                .get(1)
                .cloned()
                .ok_or_else(|| RuntimeError::Internal("PRINTBUTTON value is missing".into()))?;
            let token = self.allocate_interaction();
            let alignment = match name.as_str() {
                "PRINTBUTTONC" => Some(CellAlignment::Right),
                "PRINTBUTTONLC" => Some(CellAlignment::Left),
                _ => None,
            };
            let protocol_value = match &value {
                VmValue::Integer(value) => era_runtime_protocol::ProtocolValue::Integer(*value),
                VmValue::String(value) => {
                    era_runtime_protocol::ProtocolValue::String(value.clone())
                }
                VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => {
                    return Err(RuntimeError::Internal(
                        "PRINTBUTTON value was not materialized".into(),
                    ));
                }
            };
            self.presentation
                .append_button(text, protocol_value, token, alignment);
            self.command_intents.insert(token, value);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if matches!(
            name.as_str(),
            "PRINT_ABL" | "PRINT_TALENT" | "PRINT_MARK" | "PRINT_EXP"
        ) {
            let target = u64::try_from(integer_argument_value(&request.arguments, 0)?)
                .map_err(|_| RuntimeError::Internal("character index is negative".into()))?;
            let (variable, table, format) = match name.as_str() {
                "PRINT_ABL" => ("ABL", erabasic_data::NameTableKind::Abl, 0),
                "PRINT_TALENT" => ("TALENT", erabasic_data::NameTableKind::Talent, 1),
                "PRINT_MARK" => ("MARK", erabasic_data::NameTableKind::Mark, 0),
                "PRINT_EXP" => ("EXP", erabasic_data::NameTableKind::Exp, 2),
                _ => unreachable!(),
            };
            let text = format_named_character_values(vm, variable, table, target, format)?;
            self.presentation.append_print_text(text, false, true);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "PRINT_ITEM" {
            let text = format_having_items(vm)?;
            self.presentation.append_print_text(text, false, true);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "PRINT_PALAM" {
            let target = u64::try_from(integer_argument_value(&request.arguments, 0)?)
                .map_err(|_| RuntimeError::Internal("character index is negative".into()))?;
            let per_line = self
                .project_snapshot
                .as_ref()
                .map_or(3, |project| project.print_c_per_line.max(1));
            for (index, text) in format_character_palam(vm, target)?.into_iter().enumerate() {
                self.presentation
                    .append_column_cell(text, CellAlignment::Right);
                if (index + 1) % usize::try_from(per_line).unwrap_or(usize::MAX) == 0 {
                    self.presentation.flush_pending_line();
                }
            }
            self.presentation.flush_pending_line();
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "PRINT_SHOPITEM" {
            let project = self.project_snapshot.as_ref().ok_or_else(|| {
                RuntimeError::Internal("PRINT_SHOPITEM has no loaded project".into())
            })?;
            let per_line = project.print_c_per_line.max(1);
            let entries = format_shop_items(vm, project)?;
            for (index, (text, value)) in entries.into_iter().enumerate() {
                let token = self.allocate_interaction();
                self.presentation.append_button(
                    text,
                    era_runtime_protocol::ProtocolValue::Integer(value),
                    token,
                    Some(CellAlignment::Left),
                );
                self.command_intents.insert(token, VmValue::Integer(value));
                if (index + 1) % usize::try_from(per_line).unwrap_or(usize::MAX) == 0 {
                    self.presentation.flush_pending_line();
                }
            }
            self.presentation.flush_pending_line();
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if is_print(&name) {
            let mut text = request
                .arguments
                .iter()
                .map(display_value)
                .collect::<String>();
            if print_uses_kana_conversion(&name) {
                text = convert_kana_mode(&text, self.force_kana_mode);
            }
            if name == "REUSELASTLINE" {
                self.presentation.print_temporary_line(text);
            } else if let Some(alignment) = column_print_alignment(&name) {
                // EmueraConsole.PrintC ignores empty strings entirely.
                if !text.is_empty() {
                    if print_uses_default_color(&name) {
                        self.presentation
                            .append_default_color_column_cell(text, alignment);
                    } else {
                        self.presentation.append_column_cell(text, alignment);
                    }
                    let values = self.presentation.last_column_auto_button_values();
                    let tokens = values
                        .iter()
                        .map(|_| self.allocate_interaction())
                        .collect::<Vec<_>>();
                    for (token, value) in self.presentation.bind_last_column_auto_buttons(&tokens) {
                        self.command_intents.insert(token, VmValue::Integer(value));
                    }
                }
            } else {
                let default_color = print_uses_default_color(&name);
                let plain = name.starts_with("PRINTPLAIN");
                let commit_at_end = print_commits_line(&name);
                let mut fragments = text.split('\n').peekable();
                while let Some(fragment) = fragments.next() {
                    let line_break = fragments.peek().is_some();
                    if default_color {
                        self.presentation.append_default_color_text(
                            fragment.to_owned(),
                            false,
                            false,
                        );
                    } else if plain {
                        self.presentation.append_plain_print_text(
                            fragment.to_owned(),
                            false,
                            false,
                        );
                    } else {
                        self.presentation
                            .append_print_text(fragment.to_owned(), false, false);
                    }
                    if line_break || commit_at_end {
                        if default_color {
                            self.presentation.force_default_color_new_line();
                        } else {
                            self.presentation.force_new_line();
                        }
                        if !plain {
                            bind_last_output_buttons(self);
                        }
                    }
                }
            }
            if name.ends_with('W') {
                let wait = InputWait {
                    wait_id: self.allocate_wait(),
                    kind: WaitKind::EnterKey,
                    stability: WaitStability::StableInput,
                    one_input: false,
                    stop_message_skip: false,
                    system_input: false,
                    mouse_input: false,
                    default_value: None,
                    deadline_ns: None,
                    display_time: false,
                    timeout_message: None,
                    submission_token: self.allocate_interaction(),
                    countdown_remaining_ms: None,
                };
                let pending = PendingInput {
                    host_request: Some(request.id),
                    wait,
                    result_name: None,
                    choices: BTreeMap::new(),
                    timeout_duration_ns: None,
                    post_input: None,
                };
                commit_completion(
                    vm,
                    request.id,
                    VmHostCompletion::Pending {
                        stability: HostWaitStability::StableInput,
                        rebind_payload: encode_canonical(&pending.wait)?,
                    },
                )?;
                return self.open_wait(pending, false);
            }
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "UPDATECHECK" {
            let game_base = &vm.vm().artifact().project_data.static_data.game_base;
            if game_base.update_url.is_empty() {
                return commit_host_result_write(vm, request.id, 3);
            }
            return self.issue_host_service(
                vm,
                request,
                ExternalCompletion::UpdateCheck {
                    request: request.id,
                },
                ServiceKind::Network,
                UPDATE_CHECK_OPERATION,
                UPDATE_CHECK_OPERATION_VERSION,
                &UpdateCheckRequest {
                    url: game_base.update_url.clone(),
                },
            );
        }
        if matches!(name.as_str(), "MOUSEX" | "MOUSEY" | "MOUSEB") {
            let coordinate = match name.as_str() {
                "MOUSEX" => PointerCoordinate::X,
                "MOUSEY" => PointerCoordinate::Y,
                _ => PointerCoordinate::Button,
            };
            let presentation_revision = self.presentation.revision();
            let environment_revision = self.projection_environment_revision;
            let projection_space_revision = self.projection_space_revision;
            self.issue_host_service(
                vm,
                request,
                ExternalCompletion::PointerState {
                    request: request.id,
                    coordinate,
                    presentation_revision,
                    environment_revision,
                    projection_space_revision,
                },
                ServiceKind::InputState,
                POINTER_STATE_OPERATION,
                POINTER_STATE_OPERATION_VERSION,
                &PointerStateRequest {
                    presentation_revision,
                    environment_revision,
                    projection_space_revision,
                },
            )
        } else if matches!(name.as_str(), "GETKEY" | "GETKEYTRIGGERED") {
            let key = match request.arguments.first() {
                Some(VmValue::Integer(value)) => match u8::try_from(*value) {
                    Ok(value) => value,
                    Err(_) => {
                        return commit_completion(
                            vm,
                            request.id,
                            VmHostCompletion::Ready(HostReady {
                                value: Some(VmValue::Integer(0)),
                                writes: Vec::new(),
                            }),
                        );
                    }
                },
                _ => {
                    return commit_completion(
                        vm,
                        request.id,
                        VmHostCompletion::Ready(HostReady {
                            value: Some(VmValue::Integer(0)),
                            writes: Vec::new(),
                        }),
                    );
                }
            };
            self.issue_host_service(
                vm,
                request,
                ExternalCompletion::GetKey {
                    request: request.id,
                    key_code: key,
                    triggered: name == "GETKEYTRIGGERED",
                },
                ServiceKind::InputState,
                GET_KEY_STATE_OPERATION,
                GET_KEY_STATE_OPERATION_VERSION,
                &GetKeyStateRequest { key_code: key },
            )
        } else if matches!(
            name.as_str(),
            "GETTIME" | "GETTIMES" | "GETMILLISECOND" | "GETSECOND"
        ) {
            let operation = match name.as_str() {
                "GETTIMES" => ClockOperation::Times,
                "GETMILLISECOND" => ClockOperation::Millisecond,
                "GETSECOND" => ClockOperation::Second,
                _ => ClockOperation::Time,
            };
            self.issue_host_service(
                vm,
                request,
                ExternalCompletion::LocalDateTime {
                    request: request.id,
                    operation,
                    result: request.import.import.result,
                },
                ServiceKind::Clock,
                LOCAL_DATE_TIME_OPERATION,
                LOCAL_DATE_TIME_OPERATION_VERSION,
                &LocalDateTimeRequest {},
            )
        } else {
            self.fault(
                FaultCode::UnsupportedRuntimeFeature,
                &format!("unsupported host import: {}", request.import.import.name),
                Some(request.origin.clone()),
            )
        }
    }

    // The typed operation tuple is deliberately explicit at this single protocol edge.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn issue_host_service<T: minicbor::Encode<()>>(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        completion: ExternalCompletion,
        kind: ServiceKind,
        operation: &str,
        operation_version: ProtocolVersion,
        payload: &T,
    ) -> Result<(), RuntimeError> {
        if self.service_capabilities.get(&(kind, operation.to_owned())) != Some(&operation_version)
        {
            return self.fault(
                FaultCode::UnsupportedRuntimeFeature,
                &format!(
                    "frontend did not negotiate service {kind:?}/{operation} {operation_version:?}"
                ),
                Some(request.origin.clone()),
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
        let request_id = self.allocate_request()?;
        self.operations
            .insert_service(request_id, PendingService::Host(completion));
        self.emit(
            RuntimeMessage::ServiceRequest(ServiceRequest {
                request_id,
                kind,
                operation: operation.into(),
                operation_version,
                payload: ProtocolBytes::new(encode_canonical(payload)?),
                deadline_ns: None,
            }),
            None,
        )
    }

    fn projection_query_context(&self) -> ProjectionQueryContext {
        ProjectionQueryContext {
            presentation_revision: self.presentation.revision(),
            environment_revision: self.projection_environment_revision,
            projection_space_revision: self.projection_space_revision,
        }
    }

    fn issue_extension(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
    ) -> Result<(), RuntimeError> {
        let operation = request.import.import.name.to_ascii_lowercase();
        let declaration = self
            .project_snapshot
            .as_ref()
            .and_then(|project| project.extensions.get(&operation))
            .cloned()
            .ok_or_else(|| {
                RuntimeError::Internal(format!("extension import {operation} has no declaration"))
            })?;
        let mut arguments = Vec::with_capacity(request.arguments.len());
        let mut mutable_places = Vec::with_capacity(request.arguments.len());
        for (ordinal, argument) in request.arguments.iter().enumerate() {
            let (value, place) = match argument {
                VmValue::Integer(value) => {
                    (era_runtime_protocol::ProtocolValue::Integer(*value), None)
                }
                VmValue::String(value) => (
                    era_runtime_protocol::ProtocolValue::String(value.clone()),
                    None,
                ),
                VmValue::IntegerPlace(place) | VmValue::StringPlace(place) => {
                    let value = vm
                        .read_host_place(request.fiber, place)
                        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                    let value = match value {
                        VmValue::Integer(value) => {
                            era_runtime_protocol::ProtocolValue::Integer(value)
                        }
                        VmValue::String(value) => {
                            era_runtime_protocol::ProtocolValue::String(value)
                        }
                        VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => {
                            return Err(RuntimeError::Internal(
                                "reading an extension place returned another place".into(),
                            ));
                        }
                    };
                    let declared_type = declaration
                        .arguments
                        .get(ordinal)
                        .map_or(era_runtime_protocol::ExtensionValueType::Any, |argument| {
                            argument.value_type
                        });
                    (value, Some((place.as_ref().clone(), declared_type)))
                }
            };
            arguments.push(value);
            mutable_places.push(place);
        }
        let invocation = era_runtime_protocol::ExtensionInvocation {
            extension_id: declaration.id,
            arguments,
        };
        self.issue_host_service(
            vm,
            request,
            ExternalCompletion::Extension {
                request: request.id,
                return_type: declaration.return_type,
                mutable_places,
            },
            ServiceKind::Extension,
            &declaration.operation,
            declaration.operation_version,
            &invocation,
        )
    }

    pub(super) fn issue_platform_effect<T: minicbor::Encode<()>>(
        &mut self,
        kind: ServiceKind,
        operation: &str,
        operation_version: ProtocolVersion,
        payload: &T,
    ) -> Result<(), RuntimeError> {
        if self.service_capabilities.get(&(kind, operation.to_owned())) != Some(&operation_version)
        {
            return self.emit(
                RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                    code: "runtime.platform_capability_unavailable".into(),
                    level: RuntimeLogLevel::Warning,
                    message: format!("frontend did not negotiate service {kind:?}/{operation}"),
                    source: None,
                }),
                None,
            );
        }
        let request_id = self.allocate_request()?;
        self.operations.insert_service(
            request_id,
            PendingService::PlatformEffect {
                operation: operation.into(),
            },
        );
        self.emit(
            RuntimeMessage::ServiceRequest(ServiceRequest {
                request_id,
                kind,
                operation: operation.into(),
                operation_version,
                payload: ProtocolBytes::new(encode_canonical(payload)?),
                deadline_ns: None,
            }),
            None,
        )
    }

    pub(super) fn issue_host_storage(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        pending: PendingStorage,
        namespace: StorageNamespace,
        operation: StorageOperation,
        relative_path: String,
    ) -> Result<(), RuntimeError> {
        commit_completion(
            vm,
            request.id,
            VmHostCompletion::Pending {
                stability: HostWaitStability::Transient,
                rebind_payload: Vec::new(),
            },
        )?;
        self.issue_storage(pending, namespace, operation, relative_path)
    }
}

fn bind_html_document(
    session: &mut RuntimeSession,
    document: &mut erabasic_html::HtmlDocument,
) -> Result<(), RuntimeError> {
    fn visit(
        session: &mut RuntimeSession,
        nodes: &mut [erabasic_html::HtmlNode],
        buttons_suppressed: bool,
    ) -> Result<(), RuntimeError> {
        for node in nodes {
            let erabasic_html::HtmlNode::Element {
                kind,
                attributes,
                children,
                interaction,
                ..
            } = node
            else {
                continue;
            };
            match kind {
                erabasic_html::HtmlElementKind::Button if !buttons_suppressed => {
                    let Some(value) = attributes
                        .iter()
                        .find(|attribute| attribute.name == "value")
                        .map(|attribute| attribute.value.clone())
                    else {
                        visit(session, children, buttons_suppressed)?;
                        continue;
                    };
                    let token = session.allocate_interaction();
                    let vm_value = value
                        .parse::<i64>()
                        .map_or_else(|_| VmValue::String(value.clone()), VmValue::Integer);
                    let (integer_value, string_value) = match &vm_value {
                        VmValue::Integer(value) => (Some(*value), None),
                        VmValue::String(value) => (None, Some(value.clone())),
                        VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => unreachable!(),
                    };
                    *interaction = Some(erabasic_html::HtmlInteraction {
                        epoch: token.epoch,
                        id: token.id,
                        integer_value,
                        string_value,
                        generation: session.button_generation,
                        enabled: true,
                    });
                    session.command_intents.insert(token, vm_value);
                }
                erabasic_html::HtmlElementKind::ClearButton => {
                    // clearbutton suppresses buttonization only for its subtree;
                    // it never invalidates interactions already printed.
                    visit(session, children, true)?;
                    continue;
                }
                _ => {}
            }
            visit(session, children, buttons_suppressed)?;
        }
        Ok(())
    }
    visit(session, &mut document.nodes, false)
}

fn bind_last_output_buttons(session: &mut RuntimeSession) {
    let count = session.presentation.last_line_auto_button_values().len();
    let tokens = (0..count)
        .map(|_| session.allocate_interaction())
        .collect::<Vec<_>>();
    for (token, value) in session.presentation.bind_last_line_auto_buttons(&tokens) {
        session
            .command_intents
            .insert(token, VmValue::Integer(value));
    }
}

fn mixed_lengths(arguments: &[VmValue]) -> Result<Vec<PresentationLength>, RuntimeError> {
    if !arguments.len().is_multiple_of(2) {
        return Err(RuntimeError::Internal(
            "mixed-number host arguments are not value/unit pairs".into(),
        ));
    }
    arguments
        .chunks_exact(2)
        .map(|pair| {
            let VmValue::Integer(value) = pair[0] else {
                return Err(RuntimeError::Internal(
                    "mixed-number value is not an integer".into(),
                ));
            };
            let VmValue::Integer(unit) = pair[1] else {
                return Err(RuntimeError::Internal(
                    "mixed-number unit is not an integer".into(),
                ));
            };
            let is_px = unit != 0;
            Ok(if is_px {
                PresentationLength::Logical(era_runtime_protocol::LogicalLength(
                    value.saturating_mul(1_000),
                ))
            } else {
                PresentationLength::FontHeightHundredths(value)
            })
        })
        .collect()
}

fn append_mixed_html_attribute(
    output: &mut String,
    name: &str,
    value: Option<&PresentationLength>,
    line_height: era_runtime_protocol::LogicalLength,
) {
    let Some(value) = value else {
        return;
    };
    let (number, suffix) = match value {
        PresentationLength::Logical(value) => (value.0 / 1_000, "px"),
        PresentationLength::FontHeightHundredths(value) => {
            (value.saturating_mul(line_height.0) / 100_000, "")
        }
    };
    if number != 0 {
        let _ = write!(output, " {name}='{number}{suffix}'");
    }
}
