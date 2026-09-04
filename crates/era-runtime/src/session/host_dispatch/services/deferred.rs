#[allow(clippy::needless_borrow)]
impl RuntimeSession {
    #[allow(clippy::too_many_lines)]
    pub(super) fn dispatch_services(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
    ) -> Result<(), RuntimeError> {
        if matches!(
            name.as_str(),
            "PRINTBUTTON" | "PRINTBUTTONC" | "PRINTBUTTONLC"
        ) {
            let button = PreparedButton::prepare(name, &request.arguments).map_err(|error| {
                RuntimeError::Internal(
                    match error {
                        ButtonPreparationError::MissingValue => "PRINTBUTTON value is missing",
                        ButtonPreparationError::UnmaterializedValue => {
                            "PRINTBUTTON value was not materialized"
                        }
                        ButtonPreparationError::Unsupported => "PRINTBUTTON command is unsupported",
                    }
                    .into(),
                )
            })?;
            let token = self.allocate_interaction();
            let value = button.apply(&mut self.presentation, token);
            self.command_intents.insert(token, value);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if matches!(
            name.as_str(),
            "PRINT_ABL" | "PRINT_TALENT" | "PRINT_MARK" | "PRINT_EXP"
        ) {
            let Ok(target) = u64::try_from(integer_argument_value(request, 0)?) else {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Bounds,
                    "character index is negative",
                );
            };
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
            let Ok(target) = u64::try_from(integer_argument_value(request, 0)?) else {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Bounds,
                    "character index is negative",
                );
            };
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
            let prepared =
                PreparedGenericPrint::prepare(&name, &request.arguments, self.force_kana_mode);
            let text = prepared.text;
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
                if is_immediate_text_print(&name) && !text.contains('\n') {
                    PreparedGenericPrint { text }.apply_uncommitted(&mut self.presentation, &name);
                    commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
                    return self.emit_presentation();
                }
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
                    viewport_policy: era_runtime_protocol::InputViewportPolicy::FollowOutput,
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
        if name == "GETLINEY" {
            let index = integer_argument_value(request, 0)?;
            let Some(line_id) = self.presentation.line_id_at_display_index(index) else {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Bounds,
                    "GETLINEY display index does not identify a retained line",
                );
            };
            let context = self.presentation_observation_context()?;
            self.issue_host_service(
                vm,
                request,
                ExternalCompletion::LineGeometry {
                    request: request.id,
                    context,
                    line_id,
                },
                ServiceKind::PresentationQuery,
                GET_LINE_GEOMETRY_OPERATION,
                GET_LINE_GEOMETRY_OPERATION_VERSION,
                &GetLineGeometryV1Request { context, line_id },
            )
        } else if matches!(name.as_str(), "MOUSEX" | "MOUSEY" | "MOUSEB") {
            let coordinate = match name.as_str() {
                "MOUSEX" => PointerCoordinate::X,
                "MOUSEY" => PointerCoordinate::Y,
                _ => PointerCoordinate::Button,
            };
            let context = self.presentation_observation_context()?;
            let presentation_revision = context.presentation_revision;
            let environment_revision = context.environment_revision;
            let projection_space_revision = context.projection_space_revision;
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
            let key = match request.argument(0) {
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
}
