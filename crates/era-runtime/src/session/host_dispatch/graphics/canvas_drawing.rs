type CanvasCopyParameters = (
    Option<[i32; 4]>,
    Option<[i32; 4]>,
    Option<i64>,
    i64,
    Option<[i32; 2]>,
);

#[allow(clippy::needless_borrow)]
impl RuntimeSession {
    fn dispatch_graphics_canvas_drawing(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        self.dispatch_graphics_measurement_and_pixels(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_graphics_canvas_primitives(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_graphics_canvas_text(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_graphics_canvas_copy(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_graphics_canvas_sprite(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        Ok(())
    }

    fn dispatch_graphics_measurement_and_pixels(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "GGETTEXTSIZE" {
            *status = HostDispatchStatus::Handled;
            let context = self.projection_query_context();
            let text = request.argument(0).map_or_else(String::new, display_value);
            let font_family = request.argument(1).map_or_else(String::new, display_value);
            let font_size = integer_argument_value(request, 2)?;
            let style = if request.arguments.len() > 3 {
                integer_argument_value(request, 3)?
            } else {
                0
            };
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
            *status = HostDispatchStatus::Handled;
            let canvas_id = integer_argument_value(request, 0)?;
            let x = integer_argument_value(request, 1)?;
            let y = integer_argument_value(request, 2)?;
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
            let context = self.presentation_observation_context()?;
            return self.issue_host_service(
                vm,
                request,
                ExternalCompletion::CanvasPixel {
                    request: request.id,
                    context,
                    canvas_id,
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
        Ok(())
    }

    fn dispatch_graphics_canvas_primitives(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "GCLEAR" {
            *status = HostDispatchStatus::Handled;
            let id = integer_argument_value(request, 0)?;
            let color = integer_argument_value(request, 1)?;
            let rectangle = if request.arguments.len() == 6 {
                Some([
                    i32_argument_value(request, 2)?,
                    i32_argument_value(request, 3)?,
                    i32_argument_value(request, 4)?,
                    i32_argument_value(request, 5)?,
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
            *status = HostDispatchStatus::Handled;
            let id = integer_argument_value(request, 0)?;
            let color = checked_argb(integer_argument_value(request, 1)?)?;
            let point = [
                i32_argument_value(request, 2)?,
                i32_argument_value(request, 3)?,
            ];
            let changed = self
                .project_snapshot
                .as_mut()
                .is_some_and(|project| project.resource_graph.set_canvas_pixel(id, color, point));
            return self.complete_graphics_result(vm, request.id, i64::from(changed));
        }
        if name == "GFILLRECTANGLE" {
            *status = HostDispatchStatus::Handled;
            let id = integer_argument_value(request, 0)?;
            let rectangle = [
                i32_argument_value(request, 1)?,
                i32_argument_value(request, 2)?,
                i32_argument_value(request, 3)?,
                i32_argument_value(request, 4)?,
            ];
            let changed = self
                .project_snapshot
                .as_mut()
                .is_some_and(|project| project.resource_graph.fill_canvas_rectangle(id, rectangle));
            return self.complete_graphics_result(vm, request.id, i64::from(changed));
        }
        if name == "GDRAWLINE" {
            *status = HostDispatchStatus::Handled;
            let id = integer_argument_value(request, 0)?;
            let start = [
                i32_argument_value(request, 1)?,
                i32_argument_value(request, 2)?,
            ];
            let end = [
                i32_argument_value(request, 3)?,
                i32_argument_value(request, 4)?,
            ];
            let changed = self
                .project_snapshot
                .as_mut()
                .is_some_and(|project| project.resource_graph.draw_canvas_line(id, start, end));
            return self.complete_graphics_result(vm, request.id, i64::from(changed));
        }
        Ok(())
    }

    fn dispatch_graphics_canvas_text(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "GDRAWTEXT" {
            *status = HostDispatchStatus::Handled;
            let id = integer_argument_value(request, 0)?;
            let text = string_argument_value(request, 1, &name)?.to_owned();
            let point = if request.arguments.len() == 4 {
                [
                    i32_argument_value(request, 2)?,
                    i32_argument_value(request, 3)?,
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
                    text: string_argument_value(request, 1, &name)?.to_owned(),
                    font_family,
                    font_size,
                    style_bits,
                },
            );
        }
        Ok(())
    }

    fn dispatch_graphics_canvas_copy(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if matches!(
            name.as_str(),
            "GDRAWG" | "GDRAWGWITHMASK" | "GDRAWGWITHROTATE"
        ) {
            *status = HostDispatchStatus::Handled;
            let id = integer_argument_value(request, 0)?;
            let source_id = integer_argument_value(request, 1)?;
            let Some((source, destination, mask, rotation, rotation_center)) =
                self.canvas_copy_parameters(request, name, id, source_id)?
            else {
                return commit_integer_result(vm, request.id, 0);
            };
            let color_matrix = if name == "GDRAWG" {
                request
                    .argument(10)
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
        Ok(())
    }

    fn canvas_copy_parameters(
        &self,
        request: &VmHostRequest,
        name: &str,
        id: i64,
        source_id: i64,
    ) -> Result<Option<CanvasCopyParameters>, RuntimeError> {
        match name {
            "GDRAWG" => Ok(Some((
                Some([
                    i32_argument_value(request, 6)?,
                    i32_argument_value(request, 7)?,
                    i32_argument_value(request, 8)?,
                    i32_argument_value(request, 9)?,
                ]),
                Some([
                    i32_argument_value(request, 2)?,
                    i32_argument_value(request, 3)?,
                    i32_argument_value(request, 4)?,
                    i32_argument_value(request, 5)?,
                ]),
                None,
                0,
                None,
            ))),
            "GDRAWGWITHMASK" => self.masked_canvas_copy_parameters(request, id, source_id),
            _ => self.rotated_canvas_copy_parameters(request, source_id),
        }
    }

    fn masked_canvas_copy_parameters(
        &self,
        request: &VmHostRequest,
        destination_id: i64,
        source_id: i64,
    ) -> Result<Option<CanvasCopyParameters>, RuntimeError> {
        let source_size = self
            .project_snapshot
            .as_ref()
            .and_then(|project| project.resource_graph.canvas_state(source_id));
        let Some((width, height)) = source_size else {
            return Ok(None);
        };
        let mask_id = integer_argument_value(request, 2)?;
        let destination_point = [
            i32_argument_value(request, 3)?,
            i32_argument_value(request, 4)?,
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
            return Ok(None);
        }
        let rectangle = [
            destination_point[0],
            destination_point[1],
            i32::try_from(width).unwrap_or(i32::MAX),
            i32::try_from(height).unwrap_or(i32::MAX),
        ];
        Ok(Some((None, Some(rectangle), Some(mask_id), 0, None)))
    }

    fn rotated_canvas_copy_parameters(
        &self,
        request: &VmHostRequest,
        source_id: i64,
    ) -> Result<Option<CanvasCopyParameters>, RuntimeError> {
        let angle = integer_argument_value(request, 2)?;
        let source_size = self
            .project_snapshot
            .as_ref()
            .and_then(|project| project.resource_graph.canvas_state(source_id));
        let Some((source_width, source_height)) = source_size else {
            return Ok(None);
        };
        let center = if request.arguments.len() == 5 {
            [
                i32_argument_value(request, 3)?,
                i32_argument_value(request, 4)?,
            ]
        } else {
            [
                i32::try_from(source_width / 2).unwrap_or(i32::MAX),
                i32::try_from(source_height / 2).unwrap_or(i32::MAX),
            ]
        };
        Ok(Some((
            None,
            None,
            None,
            angle.saturating_mul(1_000),
            Some(center),
        )))
    }

    fn dispatch_graphics_canvas_sprite(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "GDRAWSPRITE" {
            *status = HostDispatchStatus::Handled;
            let id = integer_argument_value(request, 0)?;
            let sprite = request.argument(1).map_or_else(String::new, display_value);
            if request.omitted_arguments.binary_search(&6).is_ok() {
                let exists = self.project_snapshot.as_ref().is_some_and(|project| {
                    project.resource_graph.canvas_state(id).is_some()
                        && project.resource_graph.sprite(&sprite).is_some()
                });
                if !exists {
                    return commit_integer_result(vm, request.id, 0);
                }
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Operation,
                    "GDRAWSPRITE dereferenced its omitted color-matrix argument",
                );
            }
            let destination = match request.arguments.len() {
                2 => None,
                4 => Some([
                    i32_argument_value(request, 2)?,
                    i32_argument_value(request, 3)?,
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
                    i32_argument_value(request, 2)?,
                    i32_argument_value(request, 3)?,
                    i32_argument_value(request, 4)?,
                    i32_argument_value(request, 5)?,
                ]),
            };
            let color_matrix = request
                .argument(6)
                .map(|value| read_color_matrix(vm, request.fiber, value))
                .transpose()?;
            let changed = self.project_snapshot.as_mut().is_some_and(|project| {
                project
                    .resource_graph
                    .draw_sprite(id, &sprite, destination, color_matrix)
            });
            return self.complete_graphics_result(vm, request.id, i64::from(changed));
        }
        Ok(())
    }
}
