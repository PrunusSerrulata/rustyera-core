#[allow(clippy::wildcard_imports)]
use super::*;

mod audio;

#[allow(clippy::needless_borrow)]
impl RuntimeSession {
    #[allow(clippy::too_many_lines)]
    pub(super) fn dispatch_graphics(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "EXISTSOUND" {
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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
            let id = integer_argument_value(&request.arguments, 0)?;
            let disposed = self
                .project_snapshot
                .as_mut()
                .is_some_and(|project| project.resource_graph.dispose_canvas(id));
            return self.complete_graphics_result(vm, request.id, i64::from(disposed));
        }
        if matches!(name.as_str(), "GCREATED" | "GWIDTH" | "GHEIGHT") {
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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
        if name == "GCLEAR" {
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
            let sprite = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let width = integer_argument_value(&request.arguments, 1)?;
            let height = integer_argument_value(&request.arguments, 2)?;
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
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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
            *status = HostDispatchStatus::Handled;
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

        self.dispatch_audio(vm, request, name, status)?;

        Ok(())
    }
}
