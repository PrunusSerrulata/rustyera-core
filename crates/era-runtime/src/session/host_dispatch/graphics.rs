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
        if matches!(
            name.as_str(),
            "GDRAWG" | "GDRAWGWITHMASK" | "GDRAWGWITHROTATE"
        ) {
            *status = HostDispatchStatus::Handled;
            let id = integer_argument_value(request, 0)?;
            let source_id = integer_argument_value(request, 1)?;
            let (source, destination, mask, rotation, rotation_center) = match name.as_str() {
                "GDRAWG" => (
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
                ),
                "GDRAWGWITHMASK" => {
                    let source_size = self
                        .project_snapshot
                        .as_ref()
                        .and_then(|project| project.resource_graph.canvas_state(source_id));
                    let Some((width, height)) = source_size else {
                        return commit_integer_result(vm, request.id, 0);
                    };
                    let mask_id = integer_argument_value(request, 2)?;
                    let destination_id = id;
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
                    let angle = integer_argument_value(request, 2)?;
                    let source_size = self
                        .project_snapshot
                        .as_ref()
                        .and_then(|project| project.resource_graph.canvas_state(source_id));
                    let Some((source_width, source_height)) = source_size else {
                        return commit_integer_result(vm, request.id, 0);
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
                    (None, None, None, angle.saturating_mul(1_000), Some(center))
                }
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

        self.dispatch_audio(vm, request, name, status)?;

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn dispatch_scene_graphics(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
    ) -> Result<(), RuntimeError> {
        match name {
            "CBGCLEAR" => {
                let tokens = self.presentation.clear_client_backgrounds();
                self.remove_scene_interactions(tokens);
                commit_integer_result(vm, request.id, 1)?;
            }
            "CBGCLEARBUTTON" => {
                let tokens = self.presentation.clear_client_background_buttons();
                self.remove_scene_interactions(tokens);
                commit_integer_result(vm, request.id, 1)?;
            }
            "CBGREMOVEBMAP" => {
                self.presentation.clear_client_background_button_map();
                commit_integer_result(vm, request.id, 1)?;
            }
            "CBGREMOVERANGE" => {
                let minimum = integer_argument_value(request, 0)?;
                let maximum = integer_argument_value(request, 1)?;
                let tokens = self
                    .presentation
                    .clear_client_background_range(minimum, maximum);
                self.remove_scene_interactions(tokens);
                commit_integer_result(vm, request.id, 1)?;
            }
            "CBGSETBMAPG" => {
                let canvas_id = integer_argument_value(request, 0)?;
                let Some((_, _, revision)) = self
                    .project_snapshot
                    .as_ref()
                    .and_then(|project| project.resource_graph.canvas_observation(canvas_id))
                else {
                    return commit_integer_result(vm, request.id, 0);
                };
                let source = era_runtime_protocol::SceneSourceV1::Canvas {
                    canvas_id,
                    resource_revision: revision,
                };
                if !self
                    .project_snapshot
                    .as_mut()
                    .is_some_and(|project| project.resource_graph.retain_scene_source(&source))
                {
                    return commit_integer_result(vm, request.id, 0);
                }
                self.presentation.set_client_background_button_map(source);
                commit_integer_result(vm, request.id, 1)?;
            }
            "CBGSETG" => {
                let canvas_id = integer_argument_value(request, 0)?;
                let Some((_, _, revision)) = self
                    .project_snapshot
                    .as_ref()
                    .and_then(|project| project.resource_graph.canvas_observation(canvas_id))
                else {
                    return commit_integer_result(vm, request.id, 0);
                };
                let x = i32_argument_value(request, 1)?;
                let y = i32_argument_value(request, 2)?;
                let depth = cbg_depth(request, 3)?;
                let source = era_runtime_protocol::SceneSourceV1::Canvas {
                    canvas_id,
                    resource_revision: revision,
                };
                if !self
                    .project_snapshot
                    .as_mut()
                    .is_some_and(|project| project.resource_graph.retain_scene_source(&source))
                {
                    return commit_integer_result(vm, request.id, 0);
                }
                self.presentation.add_client_background(
                    source,
                    depth,
                    x,
                    y,
                    0,
                    0,
                    u8::MAX,
                    None,
                    None,
                );
                commit_integer_result(vm, request.id, 1)?;
            }
            "CBGSETSPRITE" => {
                let sprite_name = string_argument_value(request, 0, name)?.to_owned();
                let Some(revision) = self
                    .project_snapshot
                    .as_ref()
                    .and_then(|project| project.resource_graph.sprite_revision(&sprite_name))
                else {
                    return commit_integer_result(vm, request.id, 0);
                };
                let x = optional_i32_argument(request, 1, 0)?;
                let y = optional_i32_argument(request, 2, 0)?;
                let depth = optional_cbg_depth(request, 3, 1)?;
                let width = optional_i32_argument(request, 4, 0)?;
                let height = optional_i32_argument(request, 5, 0)?;
                let opacity = optional_opacity(request, 6, u8::MAX);
                let color_matrix = optional_color_matrix(vm, request, 7)?;
                let source = era_runtime_protocol::SceneSourceV1::Sprite {
                    sprite_name,
                    resource_revision: revision,
                };
                if !self
                    .project_snapshot
                    .as_mut()
                    .is_some_and(|project| project.resource_graph.retain_scene_source(&source))
                {
                    return commit_integer_result(vm, request.id, 0);
                }
                self.presentation.add_client_background(
                    source,
                    depth,
                    x,
                    y,
                    width,
                    height,
                    opacity,
                    color_matrix,
                    None,
                );
                commit_integer_result(vm, request.id, 1)?;
            }
            "CBGSETBUTTONSPRITE" => {
                let value = integer_argument_value(request, 0)?;
                if !(0..=0xff_ffff).contains(&value) {
                    return commit_integer_result(vm, request.id, 0);
                }
                let sprite_name = string_argument_value(request, 1, name)?.to_owned();
                let hover_name = string_argument_value(request, 2, name)?.to_owned();
                let Some(sprite_revision) = self
                    .project_snapshot
                    .as_ref()
                    .and_then(|project| project.resource_graph.sprite_revision(&sprite_name))
                else {
                    return commit_integer_result(vm, request.id, 1);
                };
                let hover_source = self
                    .project_snapshot
                    .as_ref()
                    .and_then(|project| project.resource_graph.sprite_revision(&hover_name))
                    .map(
                        |resource_revision| era_runtime_protocol::SceneSourceV1::Sprite {
                            sprite_name: hover_name,
                            resource_revision,
                        },
                    );
                let source = era_runtime_protocol::SceneSourceV1::Sprite {
                    sprite_name,
                    resource_revision: sprite_revision,
                };
                let x = i32_argument_value(request, 3)?;
                let y = i32_argument_value(request, 4)?;
                let depth = cbg_depth(request, 5)?;
                let title = request.argument(6).map(display_value);
                let mut roots = vec![source.clone()];
                roots.extend(hover_source.iter().cloned());
                let retained = self
                    .project_snapshot
                    .as_mut()
                    .is_some_and(|project| project.resource_graph.retain_scene_sources(&roots));
                if !retained {
                    return commit_integer_result(vm, request.id, 0);
                }
                let token = self.allocate_interaction();
                self.presentation.add_client_background(
                    source,
                    depth,
                    x,
                    y,
                    0,
                    0,
                    u8::MAX,
                    None,
                    Some((token, value, hover_source, title)),
                );
                self.command_intents.insert(token, VmValue::Integer(value));
                commit_integer_result(vm, request.id, 1)?;
            }
            "SETIMAGELAYER" | "SETIMAGELAYERL" => {
                let sprite_name = string_argument_value(request, 0, name)?.to_owned();
                let Some(resource_revision) = self
                    .project_snapshot
                    .as_ref()
                    .and_then(|project| project.resource_graph.sprite_revision(&sprite_name))
                else {
                    commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
                    return Ok(());
                };
                let depth = integer_argument_value(request, 1)?;
                let x = optional_i32_argument(request, 2, 0)?;
                let y = optional_i32_argument(request, 3, 0)?;
                let width = optional_i32_argument(request, 4, 0)?;
                let height = optional_i32_argument(request, 5, 0)?;
                let opacity = optional_opacity(request, 6, u8::MAX);
                let color_matrix = optional_color_matrix(vm, request, 7)?;
                let line_relative = name == "SETIMAGELAYERL";
                let follow_content = line_relative
                    || request
                        .argument(8)
                        .is_some_and(|value| integer_value_or_zero(value) != 0);
                let anchor = if line_relative {
                    era_runtime_protocol::SceneAnchorV1::DisplayLine {
                        line_id: self.presentation.current_line_id(),
                    }
                } else {
                    era_runtime_protocol::SceneAnchorV1::Viewport
                };
                let source = era_runtime_protocol::SceneSourceV1::Sprite {
                    sprite_name,
                    resource_revision,
                };
                if !self
                    .project_snapshot
                    .as_mut()
                    .is_some_and(|project| project.resource_graph.retain_scene_source(&source))
                {
                    commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
                    return Ok(());
                }
                self.presentation.add_image_layer(
                    source,
                    depth,
                    anchor,
                    x,
                    y,
                    width,
                    height,
                    opacity,
                    color_matrix,
                    follow_content,
                );
                commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            }
            "CLEARIMAGELAYER" => {
                self.presentation
                    .clear_image_layer(integer_argument_value(request, 0)?);
                commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            }
            "CLEARIMAGELAYER_ALL" => {
                self.presentation.clear_image_layers();
                commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            }
            "EXISTSIMAGELAYER" => {
                let exists = self
                    .presentation
                    .image_layer_exists(integer_argument_value(request, 0)?);
                return commit_integer_result(vm, request.id, i64::from(exists));
            }
            _ => unreachable!("scene graphics dispatch is exhaustive"),
        }
        self.emit_presentation()
    }

    fn remove_scene_interactions(&mut self, tokens: Vec<InteractionToken>) {
        for token in tokens {
            self.command_intents.remove(&token);
        }
    }
}

fn optional_i32_argument(
    request: &VmHostRequest,
    index: usize,
    default: i32,
) -> Result<i32, RuntimeError> {
    request.argument(index).map_or(Ok(default), |value| {
        i32::try_from(integer_value_or_zero(value)).map_err(|_| RuntimeError::Script {
            kind: erabasic_vm::ScriptFaultKind::Bounds,
            message: format!(
                "host argument {} must fit a signed 32-bit drawing coordinate",
                index + 1
            ),
        })
    })
}

fn optional_opacity(request: &VmHostRequest, index: usize, default: u8) -> u8 {
    let value = request
        .argument(index)
        .map_or(i64::from(default), integer_value_or_zero);
    u8::try_from(value.clamp(0, 255)).expect("clamped opacity fits u8")
}

fn cbg_depth(request: &VmHostRequest, index: usize) -> Result<i64, RuntimeError> {
    optional_cbg_depth(request, index, 0)
}

fn optional_cbg_depth(
    request: &VmHostRequest,
    index: usize,
    default: i64,
) -> Result<i64, RuntimeError> {
    let depth = request
        .argument(index)
        .map_or(default, integer_value_or_zero);
    if depth == 0 || i32::try_from(depth).is_err() {
        return Err(RuntimeError::Script {
            kind: erabasic_vm::ScriptFaultKind::Bounds,
            message: format!(
                "CBG depth argument {} must fit i32 and cannot be zero",
                index + 1
            ),
        });
    }
    Ok(depth)
}

fn optional_color_matrix(
    vm: &RuntimeVm,
    request: &VmHostRequest,
    index: usize,
) -> Result<Option<[i64; 25]>, RuntimeError> {
    request
        .argument(index)
        .map(|value| {
            read_color_matrix(vm, request.fiber, value).and_then(|matrix| {
                matrix.try_into().map_err(|_| {
                    RuntimeError::Internal("graphics color matrix did not contain 25 values".into())
                })
            })
        })
        .transpose()
}
