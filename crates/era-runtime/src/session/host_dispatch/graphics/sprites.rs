#[allow(clippy::needless_borrow)]
impl RuntimeSession {
    fn dispatch_graphics_sprites(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        self.dispatch_graphics_sprite_animation(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_graphics_sprite_creation(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_graphics_polygon(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_graphics_sprite_disposal(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        Ok(())
    }

    fn dispatch_graphics_sprite_animation(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "SPRITEANIMECREATE" {
            *status = HostDispatchStatus::Handled;
            let sprite = request.argument(0).map_or_else(String::new, display_value);
            let width = integer_argument_value(request, 1)?;
            let height = integer_argument_value(request, 2)?;
            if !(1..=8_192).contains(&width) || !(1..=8_192).contains(&height) {
                if width <= 8_192 && height <= 8_192 && (width <= 0 || height <= 0) {
                    return complete_script_fault(
                        vm,
                        request,
                        erabasic_vm::ScriptFaultKind::Argument,
                        "animation sprite dimensions are out of range",
                    );
                }
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
            *status = HostDispatchStatus::Handled;
            let sprite = request.argument(0).map_or_else(String::new, display_value);
            let canvas_id = integer_argument_value(request, 1)?;
            let rectangle = [
                i32_argument_value(request, 2)?,
                i32_argument_value(request, 3)?,
                i32_argument_value(request, 4)?,
                i32_argument_value(request, 5)?,
            ];
            let offset = [
                i32_argument_value(request, 6)?,
                i32_argument_value(request, 7)?,
            ];
            let delay = integer_argument_value(request, 8)?;
            let added = self.project_snapshot.as_mut().is_some_and(|project| {
                project
                    .resource_graph
                    .add_animation_frame(&sprite, canvas_id, rectangle, offset, delay)
            });
            return self.complete_graphics_result(vm, request.id, i64::from(added));
        }
        if name == "SPRITECREATEFROMFILE" {
            *status = HostDispatchStatus::Handled;
            let sprite = request.argument(0).map_or_else(String::new, display_value);
            let path = string_argument_value(request, 1, name)?;
            let relative_to_source = request
                .argument(2)
                .is_some_and(|value| integer_value_or_zero(value) != 0);
            let declaring_source = request
                .origin
                .source
                .as_ref()
                .map(|source| source.relative_path.as_str());
            let created = self.project_snapshot.as_mut().is_some_and(|project| {
                project.resource_graph.create_file_sprite(
                    &sprite,
                    &path,
                    declaring_source,
                    relative_to_source,
                )
            });
            return self.complete_graphics_result(vm, request.id, i64::from(created));
        }
        Ok(())
    }

    fn dispatch_graphics_sprite_creation(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "SPRITECREATE" {
            *status = HostDispatchStatus::Handled;
            let snake_graphics = self.project_snapshot.as_ref().is_some_and(|project| {
                project
                    .manifest
                    .compatibility
                    .supports_snake_display_state()
            });
            if matches!(request.arguments.len(), 8 | 10) && !snake_graphics {
                return self.fault(
                    FaultCode::UnsupportedRuntimeFeature,
                    "SPRITECREATE 8/10-argument forms require the snake profile",
                    Some(request.origin.clone()),
                );
            }
            if !matches!(request.arguments.len(), 2 | 6 | 8 | 10) {
                return Err(RuntimeError::Internal(
                    "SPRITECREATE physical source shape is invalid".into(),
                ));
            }
            let sprite = request.argument(0).map_or_else(String::new, display_value);
            let id = integer_argument_value(request, 1)?;
            let rectangle = if request.arguments.len() >= 6 {
                Some([
                    i32_argument_value(request, 2)?,
                    i32_argument_value(request, 3)?,
                    i32_argument_value(request, 4)?,
                    i32_argument_value(request, 5)?,
                ])
            } else {
                None
            };
            let position = if request.arguments.len() >= 8 {
                [
                    i32_argument_value(request, 6)?,
                    i32_argument_value(request, 7)?,
                ]
            } else {
                [0, 0]
            };
            let destination_size = if request.arguments.len() == 10 {
                Some([
                    i32_argument_value(request, 8)?,
                    i32_argument_value(request, 9)?,
                ])
            } else {
                None
            };
            let created = self.project_snapshot.as_mut().is_some_and(|project| {
                project.resource_graph.create_canvas_sprite(
                    &sprite,
                    id,
                    rectangle,
                    position,
                    destination_size,
                )
            });
            return self.complete_graphics_result(vm, request.id, i64::from(created));
        }
        Ok(())
    }

    fn dispatch_graphics_polygon(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "G_POLYGON_POINT_ADD" {
            *status = HostDispatchStatus::Handled;
            let canvas_id = integer_argument_value(request, 0)?;
            let point = [
                i32_argument_value(request, 1)?,
                i32_argument_value(request, 2)?,
            ];
            let added = self.project_snapshot.as_mut().is_some_and(|project| {
                project
                    .resource_graph
                    .add_canvas_polygon_point(canvas_id, point)
            });
            return self.complete_graphics_result(vm, request.id, i64::from(added));
        }
        if name == "G_POLYGON_POINT_CLEAR" {
            *status = HostDispatchStatus::Handled;
            let canvas_id = integer_argument_value(request, 0)?;
            let cleared = self.project_snapshot.as_mut().is_some_and(|project| {
                project
                    .resource_graph
                    .clear_canvas_polygon_points(canvas_id)
            });
            return self.complete_graphics_result(vm, request.id, i64::from(cleared));
        }
        if matches!(name.as_str(), "G_POLYGON_DRAW" | "G_POLYGON_FILL") {
            *status = HostDispatchStatus::Handled;
            let canvas_id = integer_argument_value(request, 0)?;
            let result = self.project_snapshot.as_mut().map_or(Ok(false), |project| {
                project
                    .resource_graph
                    .draw_canvas_polygon(canvas_id, name == "G_POLYGON_FILL")
            });
            let drawn = match result {
                Ok(drawn) => drawn,
                Err(message) => {
                    return complete_script_fault(
                        vm,
                        request,
                        erabasic_vm::ScriptFaultKind::Operation,
                        format!("{name}: {message}"),
                    );
                }
            };
            return self.complete_graphics_result(vm, request.id, i64::from(drawn));
        }
        Ok(())
    }

    fn dispatch_graphics_sprite_disposal(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "SPRITEDISPOSE" {
            *status = HostDispatchStatus::Handled;
            let sprite = request.argument(0).map_or_else(String::new, display_value);
            let disposed = self
                .project_snapshot
                .as_mut()
                .is_some_and(|project| project.resource_graph.dispose_sprite(&sprite));
            return self.complete_graphics_result(vm, request.id, i64::from(disposed));
        }
        if name == "SPRITEDISPOSEALL" {
            *status = HostDispatchStatus::Handled;
            let include_static = integer_argument_value(request, 0)? != 0;
            let count = self.project_snapshot.as_mut().map_or(0, |project| {
                project.resource_graph.dispose_sprites(include_static)
            });
            return self.complete_graphics_result(
                vm,
                request.id,
                i64::try_from(count).unwrap_or(i64::MAX),
            );
        }
        Ok(())
    }
}
