#[allow(clippy::needless_borrow)]
impl RuntimeSession {
    fn dispatch_presentation_backgrounds(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        self.dispatch_presentation_named_backgrounds(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_presentation_redraw_and_color(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_presentation_background_images(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        Ok(())
    }

    fn dispatch_presentation_named_backgrounds(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if matches!(name.as_str(), "SETCOLORBYNAME" | "SETBGCOLORBYNAME") {
            *status = HostDispatchStatus::Handled;
            let color_name = string_argument_value(request, 0, &name)?;
            let Some(color) = named_color(color_name) else {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Argument,
                    "unknown or transparent color name",
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
        if matches!(name.as_str(), "TEXT_BGC_ON" | "TEXT_BGC_OFF") {
            *status = HostDispatchStatus::Handled;
            let color = if name == "TEXT_BGC_OFF" {
                None
            } else {
                let rgb = integer_argument_value(request, 0)?;
                let alpha_percent = integer_argument_value(request, 1)?;
                if !(0..=100).contains(&alpha_percent) {
                    return complete_script_fault(
                        vm,
                        request,
                        erabasic_vm::ScriptFaultKind::Bounds,
                        "TEXT_BGC_ON alpha must be between 0 and 100",
                    );
                }
                let rgb_channel = |shift| {
                    u8::try_from((rgb >> shift) & 0xff_i64)
                        .expect("masked TEXT_BGC_ON channel fits u8")
                };
                Some(era_runtime_protocol::Color {
                    red: rgb_channel(16),
                    green: rgb_channel(8),
                    blue: rgb_channel(0),
                    alpha: u8::try_from((alpha_percent * 255) / 100)
                        .expect("validated TEXT_BGC_ON alpha fits u8"),
                })
            };
            self.presentation.set_text_line_background(color);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        Ok(())
    }

    fn dispatch_presentation_redraw_and_color(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "REDRAW" {
            *status = HostDispatchStatus::Handled;
            let flags = integer_argument_value(request, 0)?;
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
                    if matches!(
                        request.arguments.as_slice(),
                        [
                            VmValue::Integer(_),
                            VmValue::Integer(_),
                            VmValue::Integer(_)
                        ]
                    ) {
                        return complete_script_fault(
                            vm,
                            request,
                            erabasic_vm::ScriptFaultKind::Argument,
                            error,
                        );
                    }
                    return self.fault(FaultCode::VmFault, error, Some(request.origin.clone()));
                }
            };
            self.presentation.set_background(color);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        Ok(())
    }

    fn dispatch_presentation_background_images(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "SETBGIMAGE" {
            *status = HostDispatchStatus::Handled;
            let resource = request.argument(0).map_or_else(String::new, display_value);
            let depth = request.argument(1).map_or(0, integer_value_or_zero);
            let opacity = request.argument(2).map_or(255, integer_value_or_zero);
            let resource_revision = self
                .project_snapshot
                .as_ref()
                .and_then(|project| project.resource_graph.sprite_revision(&resource));
            if let Some(resource_revision) = resource_revision {
                let source = era_runtime_protocol::SceneSourceV1::Sprite {
                    sprite_name: resource.clone(),
                    resource_revision,
                };
                if self
                    .project_snapshot
                    .as_mut()
                    .is_some_and(|project| project.resource_graph.retain_scene_source(&source))
                {
                    self.presentation
                        .add_background(resource, resource_revision, depth, opacity);
                }
            }
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "REMOVEBGIMAGE" {
            *status = HostDispatchStatus::Handled;
            let resource = request.argument(0).map_or_else(String::new, display_value);
            if !self.presentation.remove_background(&resource) {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Operation,
                    "REMOVEBGIMAGE did not find the requested background",
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
        Ok(())
    }
}
