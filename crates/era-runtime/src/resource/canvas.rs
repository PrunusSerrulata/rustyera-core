use era_runtime_protocol::CanvasRect;

use super::{CanvasCommand, CanvasSurface, ResourceGraph, SpriteDefinition, SpriteFrame};

impl ResourceGraph {
    pub(crate) fn create_canvas(
        &mut self,
        id: i64,
        width: i64,
        height: i64,
    ) -> Result<bool, &'static str> {
        if self.canvases.contains_key(&id) {
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
        if self.canvases.contains_key(&id) {
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
        self.canvases.insert(
            id,
            CanvasSurface {
                width: metadata.width,
                height: metadata.height,
                revision: 1,
                commands: vec![CanvasCommand::LoadEncodedImage {
                    content_digest: image.digest.to_vec(),
                    encoded: image.bytes.clone(),
                }],
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
            || width == 0
            || height == 0
            || width > 8_192
            || height > 8_192
        {
            return false;
        }
        let digest = blake3::hash(&encoded);
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
        true
    }

    pub(crate) fn dispose_canvas(&mut self, id: i64) -> bool {
        self.canvases.remove(&id).is_some()
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
        let Some(canvas) = self.canvases.get_mut(&id) else {
            return false;
        };
        canvas.commands.push(CanvasCommand::Clear {
            // System.Drawing treats the signed EraBasic value as a 32-bit ARGB
            // bit pattern. Mask explicitly so the narrowing rule is portable.
            argb: u32::try_from(argb & i64::from(u32::MAX)).expect("masked ARGB fits u32"),
            rectangle,
        });
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
        let Some(sprite) = self.sprite(name) else {
            return false;
        };
        let destination = destination.unwrap_or([
            0,
            0,
            i32::try_from(sprite.width).unwrap_or(i32::MAX),
            i32::try_from(sprite.height).unwrap_or(i32::MAX),
        ]);
        let Some(canvas) = self.canvases.get_mut(&id) else {
            return false;
        };
        canvas.commands.push(CanvasCommand::DrawSprite {
            name: name.to_ascii_uppercase(),
            destination,
            color_matrix,
        });
        canvas.revision = canvas.revision.saturating_add(1);
        true
    }

    pub(crate) fn set_canvas_pixel(&mut self, id: i64, argb: i64, point: [i32; 2]) -> bool {
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
        canvas.commands.push(CanvasCommand::SetPixel {
            point,
            argb: argb_bits(argb),
        });
        bump_canvas(canvas);
        true
    }

    pub(crate) fn fill_canvas_rectangle(&mut self, id: i64, rectangle: [i32; 4]) -> bool {
        let Some(canvas) = self.canvases.get_mut(&id) else {
            return false;
        };
        canvas.commands.push(CanvasCommand::FillRectangle {
            rectangle,
            brush_argb: canvas.brush_argb,
        });
        bump_canvas(canvas);
        true
    }

    pub(crate) fn set_canvas_brush(&mut self, id: i64, argb: i64) -> bool {
        let Some(canvas) = self.canvases.get_mut(&id) else {
            return false;
        };
        canvas.brush_argb = argb_bits(argb);
        canvas.commands.push(CanvasCommand::SetBrush {
            argb: canvas.brush_argb,
        });
        bump_canvas(canvas);
        true
    }

    pub(crate) fn set_canvas_pen(&mut self, id: i64, argb: i64, width: i64) -> bool {
        let Some(canvas) = self.canvases.get_mut(&id) else {
            return false;
        };
        canvas.pen_argb = argb_bits(argb);
        canvas.pen_width = width;
        canvas.commands.push(CanvasCommand::SetPen {
            argb: canvas.pen_argb,
            width,
        });
        bump_canvas(canvas);
        true
    }

    pub(crate) fn set_canvas_dash(&mut self, id: i64, style: i64, cap: i64) -> bool {
        let Some(canvas) = self.canvases.get_mut(&id) else {
            return false;
        };
        canvas.dash_style = style;
        canvas.dash_cap = cap;
        canvas
            .commands
            .push(CanvasCommand::SetDashStyle { style, cap });
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
        let Some(canvas) = self.canvases.get_mut(&id) else {
            return false;
        };
        if family.is_empty() || size <= 0 {
            return false;
        }
        canvas.font_family.clone_from(&family);
        canvas.font_size = size;
        canvas.font_style = style_bits;
        canvas.commands.push(CanvasCommand::SetFont {
            family,
            size,
            style_bits,
        });
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

    pub(crate) fn draw_canvas_line(&mut self, id: i64, start: [i32; 2], end: [i32; 2]) -> bool {
        let Some(canvas) = self.canvases.get_mut(&id) else {
            return false;
        };
        canvas.commands.push(CanvasCommand::DrawLine { start, end });
        bump_canvas(canvas);
        true
    }

    pub(crate) fn draw_canvas_text(&mut self, id: i64, text: String, point: [i32; 2]) -> bool {
        let Some(canvas) = self.canvases.get_mut(&id) else {
            return false;
        };
        canvas
            .commands
            .push(CanvasCommand::DrawText { text, point });
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
        if mask_canvas_id.is_some_and(|mask| !self.canvases.contains_key(&mask)) {
            return false;
        }
        let Some(canvas) = self.canvases.get_mut(&id) else {
            return false;
        };
        canvas.commands.push(CanvasCommand::DrawCanvas {
            source_canvas_id: source_id,
            source_revision,
            source: source.unwrap_or(full),
            destination: destination.unwrap_or(full),
            color_matrix,
            mask_canvas_id,
            rotation_millidegrees,
            rotation_center,
        });
        bump_canvas(canvas);
        true
    }

    pub(crate) fn create_canvas_sprite(
        &mut self,
        name: &str,
        canvas_id: i64,
        rectangle: Option<[i32; 4]>,
    ) -> bool {
        let key = name.to_ascii_uppercase();
        if name.is_empty() || self.sprites.contains_key(&key) {
            return false;
        }
        let Some(canvas) = self.canvases.get(&canvas_id) else {
            return false;
        };
        let rectangle = rectangle.unwrap_or([
            0,
            0,
            i32::try_from(canvas.width).unwrap_or(i32::MAX),
            i32::try_from(canvas.height).unwrap_or(i32::MAX),
        ]);
        if rectangle[2] == 0 || rectangle[3] == 0 {
            return false;
        }
        self.sprites.insert(
            key.clone(),
            SpriteDefinition {
                name: key,
                width: rectangle[2].unsigned_abs(),
                height: rectangle[3].unsigned_abs(),
                frames: Vec::new(),
                dynamic: true,
                position_x: 0,
                position_y: 0,
                canvas_id: Some(canvas_id),
                canvas_rectangle: Some(rectangle),
            },
        );
        true
    }

    pub(crate) fn create_animation_sprite(&mut self, name: &str, width: i64, height: i64) -> bool {
        let key = name.to_ascii_uppercase();
        let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
            return false;
        };
        if name.is_empty()
            || width == 0
            || height == 0
            || width > 8_192
            || height > 8_192
            || self.sprites.contains_key(&key)
        {
            return false;
        }
        self.sprites.insert(
            key.clone(),
            SpriteDefinition {
                name: key,
                width,
                height,
                frames: Vec::new(),
                dynamic: true,
                position_x: 0,
                position_y: 0,
                canvas_id: None,
                canvas_rectangle: None,
            },
        );
        true
    }

    pub(crate) fn add_animation_frame(
        &mut self,
        name: &str,
        canvas_id: i64,
        rectangle: [i32; 4],
        offset: [i32; 2],
        delay_ms: i64,
    ) -> bool {
        let Some(canvas) = self.canvases.get(&canvas_id) else {
            return false;
        };
        if rectangle[0] < 0
            || rectangle[1] < 0
            || rectangle[2] <= 0
            || rectangle[3] <= 0
            || i64::from(rectangle[0]) + i64::from(rectangle[2]) > i64::from(canvas.width)
            || i64::from(rectangle[1]) + i64::from(rectangle[3]) > i64::from(canvas.height)
            || !(1..=i64::from(i32::MAX)).contains(&delay_ms)
        {
            return false;
        }
        let Some(sprite) = self.sprites.get_mut(&name.to_ascii_uppercase()) else {
            return false;
        };
        if !sprite.dynamic || sprite.canvas_id.is_some() {
            return false;
        }
        sprite.frames.push(SpriteFrame {
            image_path: String::new(),
            canvas_id: Some(canvas_id),
            source_x: rectangle[0],
            source_y: rectangle[1],
            source_width: Some(rectangle[2].cast_unsigned()),
            source_height: Some(rectangle[3].cast_unsigned()),
            offset_x: offset[0],
            offset_y: offset[1],
            delay_ms: u32::try_from(delay_ms).expect("validated animation delay"),
            destination_width: None,
            destination_height: None,
        });
        true
    }

    pub(crate) fn dispose_sprite(&mut self, name: &str) -> bool {
        let key = name.to_ascii_uppercase();
        if self.sprites.get(&key).is_none_or(|sprite| !sprite.dynamic) {
            return false;
        }
        self.sprites.remove(&key).is_some()
    }

    pub(crate) fn dispose_sprites(&mut self, include_static: bool) -> usize {
        let before = self.sprites.len();
        self.sprites
            .retain(|_, sprite| !sprite.dynamic && !include_static);
        before.saturating_sub(self.sprites.len())
    }

    /// Preserve game-created replay objects while replacing submitted static resources.
    pub(crate) fn inherit_runtime_graph(&mut self, previous: &Self) {
        self.canvases.clone_from(&previous.canvases);
        self.animation_timer_ms = previous.animation_timer_ms;
        let mut inherited_metadata = Vec::new();
        for image in self.images.values_mut() {
            if let Some(previous_image) = previous
                .images
                .get(&image.relative_path.to_ascii_lowercase())
                && previous_image.digest == image.digest
            {
                image.metadata.clone_from(&previous_image.metadata);
                if image.metadata.is_some() {
                    inherited_metadata.push(image.relative_path.clone());
                }
            }
        }
        for path in inherited_metadata {
            let _ = self.validate_image_frames(&path);
        }
        for (name, sprite) in &previous.sprites {
            if sprite.dynamic {
                self.sprites.insert(name.clone(), sprite.clone());
            }
        }
    }

    pub(crate) fn reset_runtime_graph(&mut self) {
        self.canvases.clear();
        self.animation_timer_ms = 0;
        self.sprites.retain(|_, sprite| !sprite.dynamic);
        for sprite in self.sprites.values_mut() {
            sprite.position_x = 0;
            sprite.position_y = 0;
        }
    }
}

pub(super) fn canvas_rect(value: [i32; 4]) -> CanvasRect {
    CanvasRect {
        x: value[0],
        y: value[1],
        width: value[2],
        height: value[3],
    }
}

fn argb_bits(value: i64) -> u32 {
    u32::try_from(value & i64::from(u32::MAX)).expect("masked ARGB fits u32")
}

pub(super) fn opaque_rgb(value: i64) -> u32 {
    0xff00_0000 | (argb_bits(value) & 0x00ff_ffff)
}

fn bump_canvas(canvas: &mut CanvasSurface) {
    canvas.revision = canvas.revision.saturating_add(1);
}
