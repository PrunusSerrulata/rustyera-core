#[allow(clippy::needless_borrow)]
impl RuntimeSession {
    fn dispatch_graphics_queries_and_state(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        self.dispatch_graphics_sound_queries(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_graphics_sprite_state(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_graphics_canvas_file_loading(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_graphics_canvas_persistence(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_graphics_canvas_queries(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_graphics_canvas_font_state(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        Ok(())
    }

    fn dispatch_graphics_sound_queries(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if matches!(
            name.as_str(),
            "CBGCLEAR"
                | "CBGCLEARBUTTON"
                | "CBGREMOVEBMAP"
                | "CBGREMOVERANGE"
                | "CBGSETBMAPG"
                | "CBGSETBUTTONSPRITE"
                | "CBGSETG"
                | "CBGSETSPRITE"
                | "SETIMAGELAYER"
                | "SETIMAGELAYERL"
                | "CLEARIMAGELAYER"
                | "CLEARIMAGELAYER_ALL"
                | "EXISTSIMAGELAYER"
        ) {
            *status = HostDispatchStatus::Handled;
            return self.dispatch_scene_graphics(vm, request, name);
        }
        if name == "EXISTSOUND" {
            *status = HostDispatchStatus::Handled;
            let resource = request.argument(0).map_or_else(String::new, display_value);
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
            *status = HostDispatchStatus::Handled;
            let resource = request.argument(0).map_or_else(String::new, display_value);
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
        Ok(())
    }

    fn dispatch_graphics_sprite_state(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "SPRITEGETCOLOR" {
            *status = HostDispatchStatus::Handled;
            let resource = request.argument(0).map_or_else(String::new, display_value);
            let x = integer_argument_value(request, 1)?;
            let y = integer_argument_value(request, 2)?;
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
            *status = HostDispatchStatus::Handled;
            let resource = request.argument(0).map_or_else(String::new, display_value);
            let x = i32::try_from(integer_argument_value(request, 1)?).unwrap_or(0);
            let y = i32::try_from(integer_argument_value(request, 2)?).unwrap_or(0);
            let changed = self.project_snapshot.as_mut().is_some_and(|project| {
                project
                    .resource_graph
                    .move_sprite(&resource, x, y, name == "SPRITEMOVE")
            });
            return self.complete_graphics_result(vm, request.id, i64::from(changed));
        }
        Ok(())
    }

    fn dispatch_graphics_canvas_file_loading(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "GCREATEFROMFILE" {
            *status = HostDispatchStatus::Handled;
            let id = integer_argument_value(request, 0)?;
            if self
                .project_snapshot
                .as_ref()
                .and_then(|project| project.resource_graph.canvas_state(id))
                .is_some()
            {
                return self.complete_graphics_result(vm, request.id, 0);
            }
            let filename = string_argument_value(request, 1, &name)?;
            let relative = if request.arguments.len() > 2 {
                integer_argument_value(request, 2)? != 0
            } else {
                false
            };
            // Emuera treats a missing or unusable image filename as an ordinary
            // creation failure. Keep unsafe paths away from the frontend without
            // exposing portable path validation as a runtime-internal fault.
            let Ok(path) = safe_relative_path(filename) else {
                return self.complete_graphics_result(vm, request.id, 0);
            };
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
            *status = HostDispatchStatus::Handled;
            let id = integer_argument_value(request, 0)?;
            let file_no = integer_argument_value(request, 1)?;
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
        Ok(())
    }

    fn dispatch_graphics_canvas_persistence(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "GSAVE" {
            *status = HostDispatchStatus::Handled;
            let id = integer_argument_value(request, 0)?;
            let file_no = integer_argument_value(request, 1)?;
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
            *status = HostDispatchStatus::Handled;
            let id = integer_argument_value(request, 0)?;
            let width = integer_argument_value(request, 1)?;
            let height = integer_argument_value(request, 2)?;
            let result = self
                .project_snapshot
                .as_mut()
                .ok_or_else(|| RuntimeError::Internal("GCREATE has no loaded project".into()))?
                .resource_graph
                .create_canvas(id, width, height);
            let created = match result {
                Ok(value) => value,
                Err(message) => {
                    // Preserve create_canvas's conversion order and duplicate/full sentinel.
                    let script_dimension_error = match (u32::try_from(width), u32::try_from(height))
                    {
                        (Err(_), _) => width < 0,
                        (Ok(_), Err(_)) => height < 0,
                        (Ok(width), Ok(height)) => width == 0 || height == 0,
                    };
                    if script_dimension_error {
                        return complete_script_fault(
                            vm,
                            request,
                            erabasic_vm::ScriptFaultKind::Argument,
                            message,
                        );
                    }
                    // Positive dimensions above the canvas safety limit remain noncatch.
                    return self.fault(FaultCode::VmFault, message, Some(request.origin.clone()));
                }
            };
            return self.complete_graphics_result(vm, request.id, i64::from(created));
        }
        if name == "GDISPOSE" {
            *status = HostDispatchStatus::Handled;
            let id = integer_argument_value(request, 0)?;
            let disposed = self
                .project_snapshot
                .as_mut()
                .is_some_and(|project| project.resource_graph.dispose_canvas(id));
            return self.complete_graphics_result(vm, request.id, i64::from(disposed));
        }
        Ok(())
    }

    fn dispatch_graphics_canvas_queries(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if matches!(name.as_str(), "GCREATED" | "GWIDTH" | "GHEIGHT") {
            *status = HostDispatchStatus::Handled;
            let id = integer_argument_value(request, 0)?;
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
            *status = HostDispatchStatus::Handled;
            let id = integer_argument_value(request, 0)?;
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
            *status = HostDispatchStatus::Handled;
            let id = integer_argument_value(request, 0)?;
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
        Ok(())
    }

    fn dispatch_graphics_canvas_font_state(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if matches!(
            name.as_str(),
            "GSETBRUSH" | "GSETPEN" | "GDASHSTYLE" | "GSETFONT"
        ) {
            *status = HostDispatchStatus::Handled;
            let id = integer_argument_value(request, 0)?;
            let changed = match name.as_str() {
                "GSETBRUSH" => {
                    let color = checked_argb(integer_argument_value(request, 1)?)?;
                    self.project_snapshot
                        .as_mut()
                        .is_some_and(|project| project.resource_graph.set_canvas_brush(id, color))
                }
                "GSETPEN" => {
                    let color = checked_argb(integer_argument_value(request, 1)?)?;
                    let width = integer_argument_value(request, 2)?;
                    self.project_snapshot.as_mut().is_some_and(|project| {
                        project.resource_graph.set_canvas_pen(id, color, width)
                    })
                }
                "GDASHSTYLE" => {
                    let style = integer_argument_value(request, 1)?;
                    let offset = integer_argument_value(request, 2)?;
                    self.project_snapshot.as_mut().is_some_and(|project| {
                        project.resource_graph.set_canvas_dash(id, style, offset)
                    })
                }
                "GSETFONT" => {
                    if !request.omitted_arguments.is_empty()
                        && !self.project_snapshot.as_ref().is_some_and(|project| {
                            project.resource_graph.canvas_style(id).is_some()
                        })
                    {
                        return commit_integer_result(vm, request.id, 0);
                    }
                    if request.arguments.len() < 3 {
                        let exists = self.project_snapshot.as_ref().is_some_and(|project| {
                            project.resource_graph.canvas_style(id).is_some()
                        });
                        if !exists {
                            return commit_integer_result(vm, request.id, 0);
                        }
                        return complete_script_fault(
                            vm,
                            request,
                            erabasic_vm::ScriptFaultKind::Operation,
                            "GSETFONT dereferenced its absent font-size argument",
                        );
                    }
                    let family = string_argument_value(request, 1, &name)?.to_owned();
                    let size = integer_argument_value(request, 2)?;
                    let style = if request.arguments.len() > 3 {
                        integer_argument_value(request, 3)?
                    } else {
                        0
                    };
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
        Ok(())
    }
}
