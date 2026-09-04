#[allow(clippy::needless_borrow)]
impl RuntimeSession {
    fn dispatch_presentation_media(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        self.dispatch_presentation_tooltip(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_presentation_image(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_presentation_shapes(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        Ok(())
    }

    fn dispatch_presentation_tooltip(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name.starts_with("TOOLTIP_") {
            *status = HostDispatchStatus::Handled;
            let result = match name {
                "TOOLTIP_SETCOLOR" => {
                    let foreground = integer_argument_value(request, 0)?;
                    let background = integer_argument_value(request, 1)?;
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
                    .set_tooltip_delay(integer_argument_value(request, 0)?),
                "TOOLTIP_SETDURATION" => self
                    .presentation
                    .set_tooltip_duration(integer_argument_value(request, 0)?),
                "TOOLTIP_SETFONT" => {
                    self.presentation.set_tooltip_font(
                        request.argument(0).map_or_else(String::new, display_value),
                    );
                    Ok(())
                }
                "TOOLTIP_SETFONTSIZE" => self
                    .presentation
                    .set_tooltip_font_size(integer_argument_value(request, 0)?),
                "TOOLTIP_CUSTOM" => {
                    self.presentation
                        .set_tooltip_custom(integer_argument_value(request, 0)? != 0);
                    Ok(())
                }
                "TOOLTIP_FORMAT" => {
                    self.presentation
                        .set_tooltip_format(integer_argument_value(request, 0)?);
                    Ok(())
                }
                "TOOLTIP_IMG" => {
                    self.presentation
                        .set_tooltip_images(integer_argument_value(request, 0)? != 0);
                    Ok(())
                }
                _ => {
                    return self.fault(
                        FaultCode::VmFault,
                        "unsupported tooltip operation",
                        Some(request.origin.clone()),
                    );
                }
            };
            if let Err(message) = result {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Argument,
                    message,
                );
            }
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        Ok(())
    }

    fn dispatch_presentation_image(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "PRINT_IMG" {
            *status = HostDispatchStatus::Handled;
            let resource = request.argument(0).map_or_else(String::new, display_value);
            let mut cursor = 1;
            let hover = request.argument(cursor).and_then(|value| match value {
                VmValue::String(value) => Some(value.clone()).filter(|value| !value.is_empty()),
                _ => None,
            });
            if request
                .argument(cursor)
                .is_some_and(|value| matches!(value, VmValue::String(_)))
            {
                cursor += 1;
            }
            let mask = request.argument(cursor).and_then(|value| match value {
                VmValue::String(value) => Some(value.clone()).filter(|value| !value.is_empty()),
                _ => None,
            });
            if request
                .argument(cursor)
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
        Ok(())
    }

    fn dispatch_presentation_shapes(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
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
