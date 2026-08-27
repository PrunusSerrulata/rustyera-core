#[allow(clippy::wildcard_imports)]
use super::*;

#[allow(clippy::needless_borrow)]
impl RuntimeSession {
    #[allow(clippy::too_many_lines)]
    pub(super) fn dispatch_presentation(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if let Some(mut pending) = input_wait(
            request,
            self.allocate_wait(),
            self.allocate_interaction(),
            self.logical_time_ns,
        ) {
            *status = HostDispatchStatus::Handled;
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
                && matches!(
                    request.arguments.get(can_skip_index),
                    Some(VmValue::Integer(value)) if *value != i64::MIN
                )
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
            *status = HostDispatchStatus::Handled;
            let Some(VmValue::String(pattern)) = request.arguments.first() else {
                return self.fault(
                    FaultCode::VmFault,
                    "GETLINESTR expects a string pattern",
                    Some(request.origin.clone()),
                );
            };
            let value = match erabasic_vm::logical_line_string_with_mode(
                pattern,
                usize::try_from(self.line_columns).unwrap_or(usize::MAX),
                vm.character_width_mode(),
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
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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
        if matches!(
            name.as_str(),
            "HTML_STRINGLEN"
                | "HTML_SUBSTRING"
                | "HTML_STRINGLINES"
                | "HTML__MEASURE_LENGTH"
                | "HTML__LENGTH_UNIT"
                | "HTML__LINES_BEGIN"
                | "HTML__LINES_MORE"
                | "HTML__LINES_STEP"
                | "HTML__LINES_END"
        ) {
            *status = HostDispatchStatus::Handled;
            return self.dispatch_html_query(vm, request, name);
        }
        if let Some(prepared) = PreparedLineEdit::prepare(name, &request.arguments) {
            *status = HostDispatchStatus::Handled;
            prepared.apply(&mut self.presentation);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "HTML_PRINT" {
            *status = HostDispatchStatus::Handled;
            let mut prepared = match PreparedHtmlPrint::prepare(&request.arguments) {
                Ok(prepared) => prepared,
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
            emit_html_warnings(self, "HTML_PRINT", &prepared.warnings, &request.origin)?;
            {
                let mut bindings = HtmlInteractionBindings {
                    epoch: self.epoch.0,
                    next_interaction_id: &mut self.next_interaction_id,
                    button_generation: self.button_generation,
                    command_intents: &mut self.command_intents,
                };
                bind_html_document(&mut bindings, &mut prepared.document);
            }
            prepared.apply(&mut self.presentation);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "HTML_PRINT_ISLAND" {
            *status = HostDispatchStatus::Handled;
            let markup = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let (mut document, warnings) =
                match erabasic_html::parse_document_with_warnings(&markup) {
                    Ok(parsed) => parsed,
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
            emit_html_warnings(self, "HTML_PRINT_ISLAND", &warnings, &request.origin)?;
            let mut bindings = HtmlInteractionBindings {
                epoch: self.epoch.0,
                next_interaction_id: &mut self.next_interaction_id,
                button_generation: self.button_generation,
                command_intents: &mut self.command_intents,
            };
            bind_html_document(&mut bindings, &mut document);
            self.presentation.append_html_island(document);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "HTML_PRINT_ISLAND_CLEAR" {
            *status = HostDispatchStatus::Handled;
            self.presentation.clear_html_island();
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if matches!(name.as_str(), "BAR" | "BARL") {
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
            self.debug_output_base = self
                .debug_output_base
                .saturating_add(u64::try_from(self.debug_output.len()).unwrap_or(u64::MAX));
            self.debug_output.clear();
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        match PreparedPresentationState::prepare(name, &request.arguments) {
            Ok(Some(prepared)) => {
                *status = HostDispatchStatus::Handled;
                prepared.apply(&mut self.presentation);
                return commit_completion(
                    vm,
                    request.id,
                    VmHostCompletion::Ready(HostReady::empty()),
                );
            }
            Ok(None) => {}
            Err(PresentationStatePreparationError::Alignment) => {
                *status = HostDispatchStatus::Handled;
                return self.fault(
                    FaultCode::VmFault,
                    "ALIGNMENT expects LEFT, CENTER, or RIGHT",
                    Some(request.origin.clone()),
                );
            }
            Err(PresentationStatePreparationError::FontStyle(error)) => {
                *status = HostDispatchStatus::Handled;
                return Err(error);
            }
            Err(PresentationStatePreparationError::Color(error)) => {
                *status = HostDispatchStatus::Handled;
                return self.fault(FaultCode::VmFault, error, Some(request.origin.clone()));
            }
        }
        if matches!(name.as_str(), "SETCOLORBYNAME" | "SETBGCOLORBYNAME") {
            *status = HostDispatchStatus::Handled;
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
        if name == "RESETBGCOLOR" {
            *status = HostDispatchStatus::Handled;
            self.presentation.reset_background();
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "REDRAW" {
            *status = HostDispatchStatus::Handled;
            let flags = integer_argument_value(&request.arguments, 0)?;
            self.presentation.set_redraw(flags & 1 != 0);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            self.emit_presentation()?;
            return if flags & 2 != 0 {
                self.flush_presentation_for_observation()?;
                self.emit_effect(EffectKind::PresentNow {
                    presentation_revision: self.presentation.revision(),
                })
            } else {
                Ok(())
            };
        }
        if name == "SETBGCOLOR" {
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
            self.presentation.clear_backgrounds();
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "CBGCLEAR" {
            *status = HostDispatchStatus::Handled;
            self.presentation.clear_client_backgrounds();
            commit_integer_result(vm, request.id, 1)?;
            return self.emit_presentation();
        }
        if name.starts_with("TOOLTIP_") {
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
            let parameters = mixed_lengths(&request.arguments)?;
            self.presentation.append_shape("rect", parameters);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "PRINT_SPACE" {
            *status = HostDispatchStatus::Handled;
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

        Ok(())
    }
}
