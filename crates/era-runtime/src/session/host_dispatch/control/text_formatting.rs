#[allow(clippy::needless_borrow, clippy::single_match_else)]
impl RuntimeSession {
    fn dispatch_control_text_formatting(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        Self::dispatch_control_tags_and_bars(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_control_numeric_strings(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        Self::dispatch_control_width_conversion(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        Ok(())
    }

    fn dispatch_control_tags_and_bars(
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "HTML_TAGSPLIT" {
            *status = HostDispatchStatus::Handled;
            let source = string_argument_value(request, 0, name)?;
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
        Ok(())
    }

    fn dispatch_control_numeric_strings(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if matches!(name, "MONEYSTR" | "TOSTR") {
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
        Ok(())
    }

    fn dispatch_control_width_conversion(
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if matches!(name, "TOFULL" | "TOHALF") {
            *status = HostDispatchStatus::Handled;
            let value = string_argument_value(request, 0, name)?;
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
        Ok(())
    }
}
