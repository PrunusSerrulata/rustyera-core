impl ResourceGraph {
    pub(crate) fn create_canvas(
        &mut self,
        id: i64,
        width: i64,
        height: i64,
    ) -> Result<bool, &'static str> {
        if self.canvases.contains_key(&id) || self.canvases.len() >= MAXIMUM_CANVASES {
            return Ok(false);
        }
        let width = u32::try_from(width).map_err(|_| "canvas width must be positive")?;
        let height = u32::try_from(height).map_err(|_| "canvas height must be positive")?;
        if width == 0 || height == 0 || width > 8_192 || height > 8_192 {
            return Err("canvas dimensions are out of range");
        }
        self.canvases.insert(
            id,
            CanvasSurface {
                width,
                height,
                revision: 0,
                commands: Vec::new(),
                polygon_points: Vec::new(),
                retained_command_bytes: 0,
                brush_argb: self.canvas_defaults.brush_argb,
                pen_argb: self.canvas_defaults.pen_argb,
                pen_width: 1,
                dash_style: 0,
                dash_cap: 0,
                font_family: self.canvas_defaults.font_family.clone(),
                font_size: self.canvas_defaults.font_size,
                font_style: self.canvas_defaults.font_style,
            },
        );
        Ok(true)
    }

    pub(crate) fn create_canvas_from_resource(&mut self, id: i64, path: &str) -> bool {
        if self.canvases.contains_key(&id) || self.canvases.len() >= MAXIMUM_CANVASES {
            return false;
        }
        // With the default relative=false, GCREATEFROMFILE resolves filenames
        // against Emuera's ContentDir, the project's resources directory.
        let Some(image) = self.image_from_content_directory(path) else {
            return false;
        };
        let Some(metadata) = &image.metadata else {
            return false;
        };
        if metadata.width > 8_192 || metadata.height > 8_192 {
            return false;
        }
        let width = metadata.width;
        let height = metadata.height;
        let digest = image.digest.to_vec();
        let encoded = image.bytes.clone();
        let command = CanvasCommand::LoadEncodedImage {
            content_digest: digest,
            encoded,
        };
        let retained_command_bytes = command.retained_bytes();
        self.ensure_canvas_retained_bytes();
        if !retained_canvas_bytes_fit(
            self.total_canvas_bytes_with(&self.exact_revisions),
            0,
            retained_command_bytes,
            MAXIMUM_CANVAS_COMMAND_BYTES,
        ) {
            return false;
        }
        self.canvases.insert(
            id,
            CanvasSurface {
                width,
                height,
                revision: 1,
                commands: vec![command],
                polygon_points: Vec::new(),
                retained_command_bytes,
                brush_argb: self.canvas_defaults.brush_argb,
                pen_argb: self.canvas_defaults.pen_argb,
                pen_width: 1,
                dash_style: 0,
                dash_cap: 0,
                font_family: self.canvas_defaults.font_family.clone(),
                font_size: self.canvas_defaults.font_size,
                font_style: self.canvas_defaults.font_style,
            },
        );
        self.retained_canvas_command_bytes = self
            .retained_canvas_command_bytes
            .saturating_add(retained_command_bytes);
        true
    }

    pub(crate) fn create_canvas_from_encoded(
        &mut self,
        id: i64,
        width: u32,
        height: u32,
        encoded: Vec<u8>,
    ) -> bool {
        if self.canvases.contains_key(&id)
            || self.canvases.len() >= MAXIMUM_CANVASES
            || width == 0
            || height == 0
            || width > 8_192
            || height > 8_192
        {
            return false;
        }
        let digest = blake3::hash(&encoded);
        let retained_command_bytes = size_of::<CanvasCommand>()
            .saturating_add(digest.as_bytes().len())
            .saturating_add(encoded.len());
        self.ensure_canvas_retained_bytes();
        if !retained_canvas_bytes_fit(
            self.total_canvas_bytes_with(&self.exact_revisions),
            0,
            retained_command_bytes,
            MAXIMUM_CANVAS_COMMAND_BYTES,
        ) {
            return false;
        }
        self.canvases.insert(
            id,
            CanvasSurface {
                width,
                height,
                revision: 1,
                commands: vec![CanvasCommand::LoadEncodedImage {
                    content_digest: digest.as_bytes().to_vec(),
                    encoded,
                }],
                polygon_points: Vec::new(),
                retained_command_bytes,
                brush_argb: self.canvas_defaults.brush_argb,
                pen_argb: self.canvas_defaults.pen_argb,
                pen_width: 1,
                dash_style: 0,
                dash_cap: 0,
                font_family: self.canvas_defaults.font_family.clone(),
                font_size: self.canvas_defaults.font_size,
                font_style: self.canvas_defaults.font_style,
            },
        );
        self.retained_canvas_command_bytes = self
            .retained_canvas_command_bytes
            .saturating_add(retained_command_bytes);
        true
    }

    pub(crate) fn dispose_canvas(&mut self, id: i64) -> bool {
        self.ensure_canvas_retained_bytes();
        let Some(canvas) = self.canvases.remove(&id) else {
            return false;
        };
        self.retained_canvas_command_bytes = self
            .retained_canvas_command_bytes
            .saturating_sub(canvas.retained_command_bytes);
        true
    }

    pub(crate) fn canvas_state(&self, id: i64) -> Option<(u32, u32)> {
        self.canvases
            .get(&id)
            .map(|canvas| (canvas.width, canvas.height))
    }

    pub(crate) fn canvas_observation(&self, id: i64) -> Option<(u32, u32, u64)> {
        self.canvases
            .get(&id)
            .map(|canvas| (canvas.width, canvas.height, canvas.revision))
    }

    pub(crate) fn clear_canvas(&mut self, id: i64, argb: i64, rectangle: Option<[i32; 4]>) -> bool {
        let Some(canvas) = self.canvases.get(&id) else {
            return false;
        };
        let command = CanvasCommand::Clear {
            // System.Drawing treats the signed EraBasic value as a 32-bit ARGB
            // bit pattern. Mask explicitly so the narrowing rule is portable.
            argb: u32::try_from(argb & i64::from(u32::MAX)).expect("masked ARGB fits u32"),
            rectangle,
        };
        if rectangle.is_none() {
            // A full clear is a semantic checkpoint: no earlier drawing command can
            // affect the resulting pixels. Preserve the current state explicitly so
            // later drawing commands remain replayable without the discarded prefix.
            let mut checkpoint = vec![
                command,
                CanvasCommand::SetBrush {
                    argb: canvas.brush_argb,
                },
                CanvasCommand::SetPen {
                    argb: canvas.pen_argb,
                    width: canvas.pen_width,
                },
                CanvasCommand::SetDashStyle {
                    style: canvas.dash_style,
                    cap: canvas.dash_cap,
                },
                CanvasCommand::SetFont {
                    family: canvas.font_family.clone(),
                    size: canvas.font_size,
                    style_bits: canvas.font_style,
                },
            ];
            checkpoint.extend(
                canvas
                    .polygon_points
                    .iter()
                    .copied()
                    .map(|point| CanvasCommand::PolygonPointAdd { point }),
            );
            let retained = checkpoint
                .iter()
                .map(CanvasCommand::retained_bytes)
                .fold(0, usize::saturating_add);
            self.ensure_canvas_retained_bytes();
            let previous_retained = self.canvases[&id].retained_command_bytes;
            let target_is_exact = self
                .exact_revisions
                .canvases
                .get(&id)
                .is_some_and(|revisions| revisions.contains_key(&self.canvases[&id].revision));
            let historical_retained = if target_is_exact {
                previous_retained
            } else {
                0
            };
            let next_total = self
                .total_canvas_bytes_with(&self.exact_revisions)
                .saturating_sub(previous_retained)
                .saturating_add(retained)
                .saturating_add(historical_retained);
            if next_total > MAXIMUM_CANVAS_COMMAND_BYTES {
                return false;
            }
            let canvas = self.canvases.get_mut(&id).expect("canvas was checked");
            self.retained_canvas_command_bytes = self
                .retained_canvas_command_bytes
                .saturating_sub(canvas.retained_command_bytes)
                .saturating_add(retained);
            canvas.commands = checkpoint;
            canvas.retained_command_bytes = retained;
            canvas.revision = canvas.revision.saturating_add(1);
            return true;
        }
        if !self.push_canvas_command(id, command) {
            return false;
        }
        let canvas = self.canvases.get_mut(&id).expect("canvas was checked");
        canvas.revision = canvas.revision.saturating_add(1);
        true
    }

    pub(crate) fn draw_sprite(
        &mut self,
        id: i64,
        name: &str,
        destination: Option<[i32; 4]>,
        color_matrix: Option<Vec<i64>>,
    ) -> bool {
        if !self.canvases.contains_key(&id) {
            return false;
        }
        let Some(sprite) = self.sprite(name) else {
            return false;
        };
        let resource_revision = sprite.revision;
        let sprite_size = [sprite.width, sprite.height];
        let destination = destination.unwrap_or([
            0,
            0,
            i32::try_from(sprite_size[0]).unwrap_or(i32::MAX),
            i32::try_from(sprite_size[1]).unwrap_or(i32::MAX),
        ]);
        let source = era_runtime_protocol::SceneSourceV1::Sprite {
            sprite_name: name.to_ascii_uppercase(),
            resource_revision,
        };
        let command = CanvasCommand::DrawSprite {
            name: name.to_ascii_uppercase(),
            resource_revision,
            destination,
            color_matrix,
        };
        let Some(exact_revisions) = self.preflight_exact_command(id, &[source], &command) else {
            return false;
        };
        if !self.push_canvas_command(id, command) {
            return false;
        }
        let canvas = self.canvases.get_mut(&id).expect("canvas was checked");
        canvas.revision = canvas.revision.saturating_add(1);
        self.exact_revisions = exact_revisions;
        true
    }

    pub(crate) fn set_canvas_pixel(&mut self, id: i64, argb: i64, point: [i32; 2]) -> bool {
        self.ensure_canvas_retained_bytes();
        let Some(canvas) = self.canvases.get_mut(&id) else {
            return false;
        };
        if point[0] < 0
            || point[1] < 0
            || point[0].unsigned_abs() >= canvas.width
            || point[1].unsigned_abs() >= canvas.height
        {
            return false;
        }
        if !push_canvas_command(
            &mut self.retained_canvas_command_bytes,
            canvas,
            CanvasCommand::SetPixel {
                point,
                argb: argb_bits(argb),
            },
        ) {
            return false;
        }
        bump_canvas(canvas);
        true
    }

    pub(crate) fn fill_canvas_rectangle(&mut self, id: i64, rectangle: [i32; 4]) -> bool {
        self.ensure_canvas_retained_bytes();
        let Some(canvas) = self.canvases.get_mut(&id) else {
            return false;
        };
        if !push_canvas_command(
            &mut self.retained_canvas_command_bytes,
            canvas,
            CanvasCommand::FillRectangle {
                rectangle,
                brush_argb: canvas.brush_argb,
            },
        ) {
            return false;
        }
        bump_canvas(canvas);
        true
    }

    pub(crate) fn set_canvas_brush(&mut self, id: i64, argb: i64) -> bool {
        self.ensure_canvas_retained_bytes();
        let Some(canvas) = self.canvases.get_mut(&id) else {
            return false;
        };
        let brush_argb = argb_bits(argb);
        if !push_canvas_command(
            &mut self.retained_canvas_command_bytes,
            canvas,
            CanvasCommand::SetBrush { argb: brush_argb },
        ) {
            return false;
        }
        canvas.brush_argb = brush_argb;
        bump_canvas(canvas);
        true
    }

    pub(crate) fn set_canvas_pen(&mut self, id: i64, argb: i64, width: i64) -> bool {
        self.ensure_canvas_retained_bytes();
        let Some(canvas) = self.canvases.get_mut(&id) else {
            return false;
        };
        let pen_argb = argb_bits(argb);
        if !push_canvas_command(
            &mut self.retained_canvas_command_bytes,
            canvas,
            CanvasCommand::SetPen {
                argb: pen_argb,
                width,
            },
        ) {
            return false;
        }
        canvas.pen_argb = pen_argb;
        canvas.pen_width = width;
        bump_canvas(canvas);
        true
    }

    pub(crate) fn set_canvas_dash(&mut self, id: i64, style: i64, cap: i64) -> bool {
        self.ensure_canvas_retained_bytes();
        let Some(canvas) = self.canvases.get_mut(&id) else {
            return false;
        };
        if !push_canvas_command(
            &mut self.retained_canvas_command_bytes,
            canvas,
            CanvasCommand::SetDashStyle { style, cap },
        ) {
            return false;
        }
        canvas.dash_style = style;
        canvas.dash_cap = cap;
        bump_canvas(canvas);
        true
    }

    pub(crate) fn set_canvas_font(
        &mut self,
        id: i64,
        family: String,
        size: i64,
        style_bits: u8,
    ) -> bool {
        self.ensure_canvas_retained_bytes();
        let Some(canvas) = self.canvases.get_mut(&id) else {
            return false;
        };
        if family.is_empty() || size <= 0 {
            return false;
        }
        if !push_canvas_command(
            &mut self.retained_canvas_command_bytes,
            canvas,
            CanvasCommand::SetFont {
                family: family.clone(),
                size,
                style_bits,
            },
        ) {
            return false;
        }
        canvas.font_family = family;
        canvas.font_size = size;
        canvas.font_style = style_bits;
        bump_canvas(canvas);
        true
    }

    pub(crate) fn canvas_style(&self, id: i64) -> Option<(u32, u32, i64, &str, i64, u8)> {
        self.canvases.get(&id).map(|canvas| {
            (
                canvas.brush_argb,
                canvas.pen_argb,
                canvas.pen_width,
                canvas.font_family.as_str(),
                canvas.font_size,
                canvas.font_style,
            )
        })
    }

    pub(crate) fn add_canvas_polygon_point(&mut self, id: i64, point: [i32; 2]) -> bool {
        if !self.push_canvas_command(id, CanvasCommand::PolygonPointAdd { point }) {
            return false;
        }
        let canvas = self.canvases.get_mut(&id).expect("canvas was checked");
        canvas.polygon_points.push(point);
        bump_canvas(canvas);
        true
    }

    pub(crate) fn clear_canvas_polygon_points(&mut self, id: i64) -> bool {
        if !self.push_canvas_command(id, CanvasCommand::PolygonPointClear) {
            return false;
        }
        let canvas = self.canvases.get_mut(&id).expect("canvas was checked");
        canvas.polygon_points.clear();
        bump_canvas(canvas);
        true
    }

    pub(crate) fn draw_canvas_polygon(
        &mut self,
        id: i64,
        fill: bool,
    ) -> Result<bool, &'static str> {
        let Some(canvas) = self.canvases.get(&id) else {
            return Ok(false);
        };
        if canvas.polygon_points.is_empty() {
            return Err("polygon point set is empty");
        }
        let command = if fill {
            CanvasCommand::FillPolygon
        } else {
            CanvasCommand::DrawPolygon
        };
        if !self.push_canvas_command(id, command) {
            return Ok(false);
        }
        let canvas = self.canvases.get_mut(&id).expect("canvas was checked");
        bump_canvas(canvas);
        Ok(true)
    }

    pub(crate) fn draw_canvas_line(&mut self, id: i64, start: [i32; 2], end: [i32; 2]) -> bool {
        self.ensure_canvas_retained_bytes();
        let Some(canvas) = self.canvases.get_mut(&id) else {
            return false;
        };
        if !push_canvas_command(
            &mut self.retained_canvas_command_bytes,
            canvas,
            CanvasCommand::DrawLine { start, end },
        ) {
            return false;
        }
        bump_canvas(canvas);
        true
    }

    pub(crate) fn draw_canvas_text(&mut self, id: i64, text: String, point: [i32; 2]) -> bool {
        self.ensure_canvas_retained_bytes();
        let Some(canvas) = self.canvases.get_mut(&id) else {
            return false;
        };
        if !push_canvas_command(
            &mut self.retained_canvas_command_bytes,
            canvas,
            CanvasCommand::DrawText { text, point },
        ) {
            return false;
        }
        bump_canvas(canvas);
        true
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_canvas(
        &mut self,
        id: i64,
        source_id: i64,
        source: Option<[i32; 4]>,
        destination: Option<[i32; 4]>,
        color_matrix: Option<Vec<i64>>,
        mask_canvas_id: Option<i64>,
        rotation_millidegrees: i64,
        rotation_center: Option<[i32; 2]>,
    ) -> bool {
        let Some(source_canvas) = self.canvases.get(&source_id) else {
            return false;
        };
        let source_revision = source_canvas.revision;
        let full = [
            0,
            0,
            i32::try_from(source_canvas.width).unwrap_or(i32::MAX),
            i32::try_from(source_canvas.height).unwrap_or(i32::MAX),
        ];
        let mask_revision = match mask_canvas_id {
            Some(mask) => {
                let Some(mask) = self.canvases.get(&mask) else {
                    return false;
                };
                Some(mask.revision)
            }
            None => None,
        };
        if !self.canvases.contains_key(&id) {
            return false;
        }
        let mut roots = vec![era_runtime_protocol::SceneSourceV1::Canvas {
            canvas_id: source_id,
            resource_revision: source_revision,
        }];
        if let Some((canvas_id, resource_revision)) = mask_canvas_id.zip(mask_revision) {
            roots.push(era_runtime_protocol::SceneSourceV1::Canvas {
                canvas_id,
                resource_revision,
            });
        }
        let command = CanvasCommand::DrawCanvas {
            source_canvas_id: source_id,
            source_revision,
            source: source.unwrap_or(full),
            destination: destination.unwrap_or(full),
            color_matrix,
            mask_canvas_id,
            mask_revision,
            rotation_millidegrees,
            rotation_center,
        };
        let Some(exact_revisions) = self.preflight_exact_command(id, &roots, &command) else {
            return false;
        };
        if !self.push_canvas_command(id, command) {
            return false;
        }
        let canvas = self.canvases.get_mut(&id).expect("canvas was checked");
        bump_canvas(canvas);
        self.exact_revisions = exact_revisions;
        true
    }

}
