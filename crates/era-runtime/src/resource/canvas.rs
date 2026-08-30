use std::mem::size_of;

use era_runtime_protocol::CanvasRect;

use super::{CanvasCommand, CanvasSurface, ResourceGraph, SpriteDefinition, SpriteFrame};

pub(super) const MAXIMUM_CANVAS_COMMAND_BYTES: usize = 64 * 1024 * 1024;
const MAXIMUM_CANVASES: usize = 65_536;

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
            self.retained_canvas_command_bytes,
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
            self.retained_canvas_command_bytes,
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
            if !retained_canvas_bytes_fit(
                self.retained_canvas_command_bytes
                    .saturating_sub(previous_retained),
                0,
                retained,
                MAXIMUM_CANVAS_COMMAND_BYTES,
            ) {
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
        let Some(sprite) = self.sprite(name) else {
            return false;
        };
        let destination = destination.unwrap_or([
            0,
            0,
            i32::try_from(sprite.width).unwrap_or(i32::MAX),
            i32::try_from(sprite.height).unwrap_or(i32::MAX),
        ]);
        if !self.push_canvas_command(
            id,
            CanvasCommand::DrawSprite {
                name: name.to_ascii_uppercase(),
                destination,
                color_matrix,
            },
        ) {
            return false;
        }
        let canvas = self.canvases.get_mut(&id).expect("canvas was checked");
        canvas.revision = canvas.revision.saturating_add(1);
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
        if mask_canvas_id.is_some_and(|mask| !self.canvases.contains_key(&mask)) {
            return false;
        }
        if !self.push_canvas_command(
            id,
            CanvasCommand::DrawCanvas {
                source_canvas_id: source_id,
                source_revision,
                source: source.unwrap_or(full),
                destination: destination.unwrap_or(full),
                color_matrix,
                mask_canvas_id,
                rotation_millidegrees,
                rotation_center,
            },
        ) {
            return false;
        }
        let canvas = self.canvases.get_mut(&id).expect("canvas was checked");
        bump_canvas(canvas);
        true
    }

    pub(crate) fn create_canvas_sprite(
        &mut self,
        name: &str,
        canvas_id: i64,
        rectangle: Option<[i32; 4]>,
        position: [i32; 2],
        destination_size: Option<[i32; 2]>,
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
        if !rectangle_axis_intersects(rectangle[0], rectangle[2], canvas.width)
            || !rectangle_axis_intersects(rectangle[1], rectangle[3], canvas.height)
        {
            return false;
        }
        let size = destination_size.map_or(
            [rectangle[2].unsigned_abs(), rectangle[3].unsigned_abs()],
            |size| [size[0].unsigned_abs(), size[1].unsigned_abs()],
        );
        let revision = self.allocate_sprite_revision();
        self.sprites.insert(
            key.clone(),
            SpriteDefinition {
                name: key,
                revision,
                width: size[0],
                height: size[1],
                frames: Vec::new(),
                dynamic: true,
                position_x: position[0],
                position_y: position[1],
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
        let revision = self.allocate_sprite_revision();
        self.sprites.insert(
            key.clone(),
            SpriteDefinition {
                name: key,
                revision,
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
        let key = name.to_ascii_uppercase();
        let Some(sprite) = self.sprites.get(&key) else {
            return false;
        };
        if !sprite.dynamic || sprite.canvas_id.is_some() {
            return false;
        }
        let revision = self.allocate_sprite_revision();
        let sprite = self.sprites.get_mut(&key).expect("sprite was checked");
        sprite.frames.push(SpriteFrame {
            image_path: String::new(),
            content_digest: None,
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
        sprite.revision = revision;
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
        self.next_sprite_revision = self.next_sprite_revision.max(previous.next_sprite_revision);
        self.canvases.clone_from(&previous.canvases);
        self.retained_canvas_command_bytes = previous.retained_canvas_command_bytes;
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
            let resources_still_match = sprite.frames.iter().all(|frame| {
                frame.content_digest.map_or_else(
                    || frame.image_path.is_empty(),
                    |digest| {
                        self.images
                            .get(&frame.image_path.to_ascii_lowercase())
                            .is_some_and(|image| image.digest == digest)
                    },
                )
            });
            if sprite.dynamic && resources_still_match {
                self.sprites.insert(name.clone(), sprite.clone());
            }
        }
    }

    pub(crate) fn reset_runtime_graph(&mut self) {
        self.canvases = std::collections::BTreeMap::default();
        self.retained_canvas_command_bytes = 0;
        self.animation_timer_ms = 0;
        self.sprites.retain(|_, sprite| !sprite.dynamic);
        let moved = self
            .sprites
            .iter()
            .filter_map(|(name, sprite)| {
                ((sprite.position_x, sprite.position_y) != (0, 0)).then_some(name.clone())
            })
            .collect::<Vec<_>>();
        for name in moved {
            let revision = self.allocate_sprite_revision();
            let sprite = self.sprites.get_mut(&name).expect("sprite was retained");
            sprite.position_x = 0;
            sprite.position_y = 0;
            sprite.revision = revision;
        }
    }

    fn ensure_canvas_retained_bytes(&mut self) {
        let mut rebuilt_surface = false;
        for canvas in self.canvases.values_mut() {
            if canvas.retained_command_bytes == 0 && !canvas.commands.is_empty() {
                canvas.retained_command_bytes = canvas
                    .commands
                    .iter()
                    .map(CanvasCommand::retained_bytes)
                    .fold(0, usize::saturating_add);
                rebuilt_surface = true;
            }
        }
        if rebuilt_surface || self.retained_canvas_command_bytes == 0 {
            self.retained_canvas_command_bytes = self
                .canvases
                .values()
                .map(|canvas| canvas.retained_command_bytes)
                .fold(0, usize::saturating_add);
        }
    }

    fn push_canvas_command(&mut self, id: i64, command: CanvasCommand) -> bool {
        self.ensure_canvas_retained_bytes();
        let Some(canvas) = self.canvases.get_mut(&id) else {
            return false;
        };
        push_canvas_command(&mut self.retained_canvas_command_bytes, canvas, command)
    }

    #[cfg(test)]
    pub(super) fn push_canvas_command_with_limit(
        &mut self,
        id: i64,
        command: CanvasCommand,
        maximum: usize,
    ) -> bool {
        self.ensure_canvas_retained_bytes();
        let Some(canvas) = self.canvases.get_mut(&id) else {
            return false;
        };
        push_canvas_command_with_limit(
            &mut self.retained_canvas_command_bytes,
            canvas,
            command,
            maximum,
        )
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

fn rectangle_axis_intersects(origin: i32, extent: i32, limit: u32) -> bool {
    if extent == 0 {
        return false;
    }
    let origin = i64::from(origin);
    let end = origin.saturating_add(i64::from(extent));
    origin.min(end) < i64::from(limit) && origin.max(end) > 0
}

fn push_canvas_command(
    total_retained: &mut usize,
    canvas: &mut CanvasSurface,
    command: CanvasCommand,
) -> bool {
    push_canvas_command_with_limit(
        total_retained,
        canvas,
        command,
        MAXIMUM_CANVAS_COMMAND_BYTES,
    )
}

fn push_canvas_command_with_limit(
    total_retained: &mut usize,
    canvas: &mut CanvasSurface,
    command: CanvasCommand,
    maximum: usize,
) -> bool {
    if canvas.retained_command_bytes == 0 && !canvas.commands.is_empty() {
        // Snapshots written before the retained-byte counter was introduced decode
        // it as zero. Rebuild it lazily before accepting another command.
        canvas.retained_command_bytes = canvas
            .commands
            .iter()
            .map(CanvasCommand::retained_bytes)
            .fold(0, usize::saturating_add);
    }
    let retained = command.retained_bytes();
    if !retained_canvas_bytes_fit(
        *total_retained,
        canvas.retained_command_bytes,
        retained,
        maximum,
    ) {
        return false;
    }
    canvas.retained_command_bytes = canvas.retained_command_bytes.saturating_add(retained);
    *total_retained = total_retained.saturating_add(retained);
    canvas.commands.push(command);
    true
}

pub(super) fn retained_canvas_bytes_fit(
    total_retained: usize,
    surface_retained: usize,
    incoming: usize,
    maximum: usize,
) -> bool {
    surface_retained.saturating_add(incoming) <= maximum
        && total_retained.saturating_add(incoming) <= maximum
}
