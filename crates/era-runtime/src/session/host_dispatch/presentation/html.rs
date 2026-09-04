#[allow(clippy::needless_borrow)]
impl RuntimeSession {
    fn dispatch_presentation_html(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        self.dispatch_presentation_html_history(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_presentation_html_cells(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_presentation_html_print(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_presentation_html_island(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_presentation_html_island_clear(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        Ok(())
    }

    fn dispatch_presentation_html_history(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
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
        Ok(())
    }

    fn dispatch_presentation_html_cells(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if matches!(name.as_str(), "HTML_PRINTC" | "HTML_PRINTLC") {
            *status = HostDispatchStatus::Handled;
            if !vm
                .vm()
                .artifact()
                .manifest
                .compatibility
                .supports_snake_display_state()
            {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Operation,
                    format!("{name} is unavailable for this compatibility profile"),
                );
            }
            let mut prepared = match PreparedHtmlColumnPrint::prepare(name, &request.arguments) {
                Ok(prepared) => prepared,
                Err(error) => {
                    emit_html_profile_error(
                        self,
                        name,
                        &error,
                        &request.origin,
                        &vm.vm().artifact().manifest.compatibility,
                    )?;
                    return complete_script_fault(
                        vm,
                        request,
                        erabasic_vm::ScriptFaultKind::Parse,
                        format!(
                            "{name} {:?} at UTF-8 bytes {}..{}",
                            error.kind, error.start, error.end
                        ),
                    );
                }
            };
            resolve_document_color_matrices(vm, request.fiber, &mut prepared.document);
            emit_html_warnings(self, name, &prepared.warnings, &request.origin)?;
            {
                let mut bindings = HtmlInteractionBindings {
                    epoch: self.epoch.0,
                    next_interaction_id: &mut self.next_interaction_id,
                    button_generation: self.button_generation,
                    command_intents: &mut self.command_intents,
                };
                bind_html_document(&mut bindings, &mut prepared.document);
            }
            let changed = prepared.apply(&mut self.presentation);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return if changed {
                self.emit_presentation()
            } else {
                Ok(())
            };
        }
        Ok(())
    }

    fn dispatch_presentation_html_print(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "HTML_PRINT" {
            *status = HostDispatchStatus::Handled;
            let mut prepared = match PreparedHtmlPrint::prepare(&request.arguments) {
                Ok(prepared) => prepared,
                Err(error) => {
                    emit_html_profile_error(
                        self,
                        "HTML_PRINT",
                        &error,
                        &request.origin,
                        &vm.vm().artifact().manifest.compatibility,
                    )?;
                    if error.origin() != erabasic_html::HtmlQueryErrorOrigin::ScriptInput {
                        return self.fault(
                            FaultCode::VmFault,
                            &format!(
                                "HTML_PRINT {:?} at UTF-8 bytes {}..{}",
                                error.kind, error.start, error.end
                            ),
                            Some(request.origin.clone()),
                        );
                    }
                    return complete_script_fault(
                        vm,
                        request,
                        erabasic_vm::ScriptFaultKind::Parse,
                        format!(
                            "HTML_PRINT {:?} at UTF-8 bytes {}..{}",
                            error.kind, error.start, error.end
                        ),
                    );
                }
            };
            if !vm
                .vm()
                .artifact()
                .manifest
                .compatibility
                .supports_snake_display_state()
                && let Some(range) = erabasic_html::snake_extension_range(&prepared.document)
            {
                let (start, end) = (range.start, range.end);
                let error = erabasic_html::HtmlError::new(
                    erabasic_html::HtmlErrorKind::InvalidAttribute,
                    start,
                    end,
                );
                emit_html_profile_error(
                    self,
                    "HTML_PRINT",
                    &error,
                    &request.origin,
                    &vm.vm().artifact().manifest.compatibility,
                )?;
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Parse,
                    format!("HTML_PRINT profile attribute at UTF-8 bytes {start}..{end}"),
                );
            }
            resolve_document_color_matrices(vm, request.fiber, &mut prepared.document);
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
        Ok(())
    }

    fn dispatch_presentation_html_island(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "HTML_PRINT_ISLAND" {
            *status = HostDispatchStatus::Handled;
            let markup = request.argument(0).map_or_else(String::new, display_value);
            let (mut document, warnings) =
                match erabasic_html::parse_document_with_warnings(&markup) {
                    Ok(parsed) => parsed,
                    Err(error) => {
                        emit_html_profile_error(
                            self,
                            "HTML_PRINT_ISLAND",
                            &error,
                            &request.origin,
                            &vm.vm().artifact().manifest.compatibility,
                        )?;
                        if error.origin() != erabasic_html::HtmlQueryErrorOrigin::ScriptInput {
                            return self.fault(
                                FaultCode::VmFault,
                                &format!(
                                    "HTML_PRINT_ISLAND {:?} at UTF-8 bytes {}..{}",
                                    error.kind, error.start, error.end
                                ),
                                Some(request.origin.clone()),
                            );
                        }
                        return complete_script_fault(
                            vm,
                            request,
                            erabasic_vm::ScriptFaultKind::Parse,
                            format!(
                                "HTML_PRINT_ISLAND {:?} at UTF-8 bytes {}..{}",
                                error.kind, error.start, error.end
                            ),
                        );
                    }
                };
            if !vm
                .vm()
                .artifact()
                .manifest
                .compatibility
                .supports_snake_display_state()
                && let Some(range) = erabasic_html::snake_extension_range(&document)
            {
                let (start, end) = (range.start, range.end);
                let error = erabasic_html::HtmlError::new(
                    erabasic_html::HtmlErrorKind::InvalidAttribute,
                    start,
                    end,
                );
                emit_html_profile_error(
                    self,
                    "HTML_PRINT_ISLAND",
                    &error,
                    &request.origin,
                    &vm.vm().artifact().manifest.compatibility,
                )?;
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Parse,
                    format!("HTML_PRINT_ISLAND profile attribute at UTF-8 bytes {start}..{end}"),
                );
            }
            resolve_document_color_matrices(vm, request.fiber, &mut document);
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
        Ok(())
    }

    fn dispatch_presentation_html_island_clear(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "HTML_PRINT_ISLAND_CLEAR" {
            *status = HostDispatchStatus::Handled;
            self.presentation.clear_html_island();
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        Ok(())
    }
}
