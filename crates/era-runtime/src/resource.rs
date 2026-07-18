use std::collections::{BTreeMap, BTreeSet};

use era_runtime_protocol::{
    CanvasRect, CanvasReplay, CanvasReplayCommand, CanvasSize, FileCategory, FilePayload,
    ImageMetadataResponse, ProjectManifest, ResourceReplay, SpriteFrameReplay, SpriteReplay,
    validate_relative_path,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct ResourceGraph {
    images: BTreeMap<String, ResourceImage>,
    sprites: BTreeMap<String, SpriteDefinition>,
    canvases: BTreeMap<i64, CanvasSurface>,
    animation_timer_ms: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CanvasSurface {
    width: u32,
    height: u32,
    revision: u64,
    commands: Vec<CanvasCommand>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum CanvasCommand {
    Clear {
        argb: u32,
        rectangle: Option<[i32; 4]>,
    },
    DrawSprite {
        name: String,
        destination: [i32; 4],
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ResourceImage {
    relative_path: String,
    digest: [u8; 32],
    metadata: Option<ImageMetadata>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ImageMetadata {
    width: u32,
    height: u32,
    format: String,
    animated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SpriteDefinition {
    pub(crate) name: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) frames: Vec<SpriteFrame>,
    pub(crate) dynamic: bool,
    pub(crate) position_x: i32,
    pub(crate) position_y: i32,
    pub(crate) canvas_id: Option<i64>,
    pub(crate) canvas_rectangle: Option<[i32; 4]>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SpriteFrame {
    pub(crate) image_path: String,
    pub(crate) canvas_id: Option<i64>,
    pub(crate) source_x: i32,
    pub(crate) source_y: i32,
    pub(crate) source_width: Option<u32>,
    pub(crate) source_height: Option<u32>,
    pub(crate) offset_x: i32,
    pub(crate) offset_y: i32,
    pub(crate) delay_ms: u32,
    pub(crate) destination_width: Option<u32>,
    pub(crate) destination_height: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResourceDiagnostic {
    pub(crate) code: &'static str,
    pub(crate) path: String,
    pub(crate) line: Option<u64>,
    pub(crate) message: String,
    pub(crate) error: bool,
}

impl ResourceGraph {
    pub(crate) fn from_manifest(manifest: &ProjectManifest) -> (Self, Vec<ResourceDiagnostic>) {
        let mut graph = Self::default();
        let mut diagnostics = Vec::new();
        for file in &manifest.files {
            if file.category != FileCategory::Resource {
                continue;
            }
            let Ok(path) = validate_relative_path(&file.relative_path) else {
                continue;
            };
            let bytes = match &file.payload {
                FilePayload::Utf8(value) => value.as_bytes(),
                FilePayload::Bytes(value) => value.as_slice(),
                FilePayload::IoError(_) => continue,
            };
            graph.images.insert(
                path.to_ascii_lowercase(),
                ResourceImage {
                    relative_path: path,
                    digest: *blake3::hash(bytes).as_bytes(),
                    metadata: None,
                },
            );
        }

        let mut manifests = manifest
            .files
            .iter()
            .filter(|file| file.category == FileCategory::ResourceManifest)
            .collect::<Vec<_>>();
        manifests.sort_by(|left, right| {
            left.relative_path
                .to_ascii_lowercase()
                .cmp(&right.relative_path.to_ascii_lowercase())
        });
        for manifest in manifests {
            let FilePayload::Utf8(text) = &manifest.payload else {
                continue;
            };
            parse_resource_manifest(&mut graph, &mut diagnostics, &manifest.relative_path, text);
        }
        (graph, diagnostics)
    }

    pub(crate) fn metadata_requests(&self) -> Vec<(String, [u8; 32])> {
        let referenced = self
            .sprites
            .values()
            .flat_map(|sprite| &sprite.frames)
            .map(|frame| frame.image_path.to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        referenced
            .into_iter()
            .filter_map(|key| {
                self.images.get(&key).and_then(|image| {
                    image
                        .metadata
                        .is_none()
                        .then(|| (image.relative_path.clone(), image.digest))
                })
            })
            .collect()
    }

    pub(crate) fn apply_metadata(
        &mut self,
        relative_path: &str,
        metadata: ImageMetadataResponse,
    ) -> Result<(), String> {
        if metadata.width == 0 || metadata.height == 0 {
            return Err("image metadata dimensions must be positive".into());
        }
        let image = self
            .images
            .get_mut(&relative_path.to_ascii_lowercase())
            .ok_or_else(|| "image metadata refers to an unknown resource".to_owned())?;
        image.metadata = Some(ImageMetadata {
            width: metadata.width,
            height: metadata.height,
            format: metadata.format,
            animated: metadata.animated,
        });
        self.validate_image_frames(relative_path)
    }

    fn validate_image_frames(&mut self, relative_path: &str) -> Result<(), String> {
        let image = self
            .images
            .get(&relative_path.to_ascii_lowercase())
            .and_then(|image| image.metadata.as_ref())
            .ok_or("image metadata is unavailable")?;
        for sprite in self.sprites.values_mut() {
            for frame in &mut sprite.frames {
                if !frame.image_path.eq_ignore_ascii_case(relative_path) {
                    continue;
                }
                let width = frame.source_width.unwrap_or(image.width);
                let height = frame.source_height.unwrap_or(image.height);
                let intersects = frame.source_x < i32::try_from(image.width).unwrap_or(i32::MAX)
                    && frame.source_y < i32::try_from(image.height).unwrap_or(i32::MAX)
                    && frame
                        .source_x
                        .saturating_add(i32::try_from(width).unwrap_or(i32::MAX))
                        > 0
                    && frame
                        .source_y
                        .saturating_add(i32::try_from(height).unwrap_or(i32::MAX))
                        > 0;
                if width == 0 || height == 0 || !intersects {
                    return Err(format!(
                        "sprite {} has a source rectangle outside image {relative_path}",
                        sprite.name
                    ));
                }
                frame.source_width = Some(width);
                frame.source_height = Some(height);
                if sprite.width == 0 {
                    sprite.width = frame.destination_width.unwrap_or(width);
                }
                if sprite.height == 0 {
                    sprite.height = frame.destination_height.unwrap_or(height);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn sprite(&self, name: &str) -> Option<&SpriteDefinition> {
        self.sprites.get(&name.to_ascii_uppercase())
    }

    pub(crate) fn move_sprite(&mut self, name: &str, x: i32, y: i32, relative: bool) -> bool {
        let Some(sprite) = self.sprites.get_mut(&name.to_ascii_uppercase()) else {
            return false;
        };
        if relative {
            sprite.position_x = sprite.position_x.saturating_add(x);
            sprite.position_y = sprite.position_y.saturating_add(y);
        } else {
            sprite.position_x = x;
            sprite.position_y = y;
        }
        true
    }

    pub(crate) fn sprite_pixel_request(
        &self,
        name: &str,
        x: i64,
        y: i64,
    ) -> Option<(String, [u8; 32], u32, u32)> {
        let sprite = self.sprite(name)?;
        if x < 0 || y < 0 || x >= i64::from(sprite.width) || y >= i64::from(sprite.height) {
            return None;
        }
        // Animated sprites expose the first frame at a stable VM instant. Animation
        // time belongs to presentation projection and never makes VM reads race a renderer.
        let frame = sprite.frames.first()?;
        let source_width = frame.source_width?;
        let source_height = frame.source_height?;
        let source_x = i64::from(frame.source_x)
            .saturating_add(x.saturating_mul(i64::from(source_width)) / i64::from(sprite.width));
        let source_y = i64::from(frame.source_y)
            .saturating_add(y.saturating_mul(i64::from(source_height)) / i64::from(sprite.height));
        let image = self.images.get(&frame.image_path.to_ascii_lowercase())?;
        Some((
            image.relative_path.clone(),
            image.digest,
            u32::try_from(source_x).ok()?,
            u32::try_from(source_y).ok()?,
        ))
    }

    pub(crate) fn contains_audio(&self, name: &str) -> bool {
        self.images.contains_key(&name.to_ascii_lowercase())
    }

    pub(crate) fn replay(&self) -> ResourceReplay {
        ResourceReplay {
            sprites: self
                .sprites
                .values()
                .map(|sprite| SpriteReplay {
                    name: sprite.name.clone(),
                    size: [sprite.width, sprite.height],
                    position: [sprite.position_x, sprite.position_y],
                    frames: sprite
                        .frames
                        .iter()
                        .map(|frame| SpriteFrameReplay {
                            resource_id: frame.image_path.clone(),
                            source_rectangle: [
                                frame.source_x,
                                frame.source_y,
                                i32::try_from(frame.source_width.unwrap_or_default())
                                    .unwrap_or(i32::MAX),
                                i32::try_from(frame.source_height.unwrap_or_default())
                                    .unwrap_or(i32::MAX),
                            ],
                            offset: [frame.offset_x, frame.offset_y],
                            delay_ms: frame.delay_ms,
                            destination_size: frame
                                .destination_width
                                .zip(frame.destination_height)
                                .map(|(width, height)| [width, height]),
                            canvas_id: frame.canvas_id,
                        })
                        .collect(),
                    canvas_id: sprite.canvas_id,
                    canvas_rectangle: sprite.canvas_rectangle.map(canvas_rect),
                })
                .collect(),
            canvases: self
                .canvases
                .iter()
                .map(|(canvas_id, canvas)| CanvasReplay {
                    canvas_id: *canvas_id,
                    size: CanvasSize {
                        width: canvas.width,
                        height: canvas.height,
                    },
                    commands: canvas
                        .commands
                        .iter()
                        .map(|command| match command {
                            CanvasCommand::Clear { argb, rectangle } => {
                                CanvasReplayCommand::Clear {
                                    argb: *argb,
                                    rectangle: rectangle.map(canvas_rect),
                                }
                            }
                            CanvasCommand::DrawSprite { name, destination } => {
                                CanvasReplayCommand::DrawSprite {
                                    name: name.clone(),
                                    destination: canvas_rect(*destination),
                                }
                            }
                        })
                        .collect(),
                    revision: canvas.revision,
                })
                .collect(),
            animation_timer_ms: self.animation_timer_ms,
        }
    }

    pub(crate) fn set_animation_timer(&mut self, milliseconds: i64) -> Result<(), &'static str> {
        self.animation_timer_ms = i32::try_from(milliseconds)
            .map_err(|_| "animation timer is outside the signed 32-bit range")?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) const fn animation_timer(&self) -> i32 {
        self.animation_timer_ms
    }

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
            },
        );
        Ok(true)
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
        });
        canvas.revision = canvas.revision.saturating_add(1);
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

fn canvas_rect(value: [i32; 4]) -> CanvasRect {
    CanvasRect {
        x: value[0],
        y: value[1],
        width: value[2],
        height: value[3],
    }
}

#[allow(clippy::too_many_lines)]
fn parse_resource_manifest(
    graph: &mut ResourceGraph,
    diagnostics: &mut Vec<ResourceDiagnostic>,
    path: &str,
    text: &str,
) {
    let directory = path.rsplit_once('/').map_or("", |(directory, _)| directory);
    let mut current_animation: Option<String> = None;
    for (line_index, raw) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        let tokens = line.split(',').map(str::trim).collect::<Vec<_>>();
        if tokens.len() < 2 || tokens[0].is_empty() || tokens[1].is_empty() {
            continue;
        }
        let name = tokens[0].to_ascii_uppercase();
        if tokens[1].eq_ignore_ascii_case("ANIME") {
            let Some((width, height)) = parse_pair(&tokens, 2).filter(|(w, h)| *w > 0 && *h > 0)
            else {
                diagnostics.push(resource_error(
                    path,
                    line_index,
                    "runtime.invalid_animation_sprite",
                    "animation sprite requires positive width and height",
                ));
                current_animation = None;
                continue;
            };
            if graph.sprites.contains_key(&name) {
                diagnostics.push(resource_warning(
                    path,
                    line_index,
                    "runtime.duplicate_sprite",
                    format!("duplicate sprite {name} was ignored"),
                ));
                current_animation = None;
                continue;
            }
            graph.sprites.insert(
                name.clone(),
                SpriteDefinition {
                    name: name.clone(),
                    width: width.cast_unsigned(),
                    height: height.cast_unsigned(),
                    frames: Vec::new(),
                    dynamic: false,
                    position_x: 0,
                    position_y: 0,
                    canvas_id: None,
                    canvas_rectangle: None,
                },
            );
            current_animation = Some(name);
            continue;
        }
        let image_path = if directory.is_empty() {
            tokens[1].to_owned()
        } else {
            format!("{directory}/{}", tokens[1])
        };
        let Ok(image_path) = validate_relative_path(&image_path) else {
            diagnostics.push(resource_error(
                path,
                line_index,
                "runtime.invalid_resource_path",
                "resource CSV image path is invalid",
            ));
            current_animation = None;
            continue;
        };
        if !graph.images.contains_key(&image_path.to_ascii_lowercase()) {
            diagnostics.push(resource_error(
                path,
                line_index,
                "runtime.missing_resource_image",
                format!("resource image {image_path} was not submitted by the frontend"),
            ));
            current_animation = None;
            continue;
        }
        let rect = parse_quad(&tokens, 2);
        let offset = parse_pair(&tokens, 6).unwrap_or((0, 0));
        let delay_ms = tokens
            .get(8)
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(1_000);
        let destination = parse_pair(&tokens, 9)
            .filter(|(width, height)| *width > 0 && *height > 0)
            .map(|(width, height)| (width.cast_unsigned(), height.cast_unsigned()));
        let frame = SpriteFrame {
            image_path,
            canvas_id: None,
            source_x: rect.map_or(0, |value| value.0),
            source_y: rect.map_or(0, |value| value.1),
            source_width: rect.and_then(|value| u32::try_from(value.2).ok()),
            source_height: rect.and_then(|value| u32::try_from(value.3).ok()),
            offset_x: offset.0,
            offset_y: offset.1,
            delay_ms,
            destination_width: destination.map(|value| value.0),
            destination_height: destination.map(|value| value.1),
        };
        if current_animation.as_deref() == Some(name.as_str()) {
            if let Some(animation) = graph.sprites.get_mut(&name) {
                animation.frames.push(frame);
            }
            continue;
        }
        current_animation = None;
        if graph.sprites.contains_key(&name) {
            diagnostics.push(resource_warning(
                path,
                line_index,
                "runtime.duplicate_sprite",
                format!("duplicate sprite {name} was ignored"),
            ));
            continue;
        }
        graph.sprites.insert(
            name.clone(),
            SpriteDefinition {
                name,
                width: destination.map_or(0, |value| value.0),
                height: destination.map_or(0, |value| value.1),
                frames: vec![frame],
                dynamic: false,
                position_x: 0,
                position_y: 0,
                canvas_id: None,
                canvas_rectangle: None,
            },
        );
    }
}

fn parse_pair(tokens: &[&str], start: usize) -> Option<(i32, i32)> {
    Some((
        tokens.get(start)?.parse().ok()?,
        tokens.get(start + 1)?.parse().ok()?,
    ))
}

fn parse_quad(tokens: &[&str], start: usize) -> Option<(i32, i32, i32, i32)> {
    Some((
        tokens.get(start)?.parse().ok()?,
        tokens.get(start + 1)?.parse().ok()?,
        tokens.get(start + 2)?.parse().ok()?,
        tokens.get(start + 3)?.parse().ok()?,
    ))
}

fn resource_error(
    path: &str,
    line: usize,
    code: &'static str,
    message: impl Into<String>,
) -> ResourceDiagnostic {
    ResourceDiagnostic {
        code,
        path: path.into(),
        line: Some(u64::try_from(line + 1).unwrap_or(u64::MAX)),
        message: message.into(),
        error: true,
    }
}

fn resource_warning(
    path: &str,
    line: usize,
    code: &'static str,
    message: impl Into<String>,
) -> ResourceDiagnostic {
    ResourceDiagnostic {
        error: false,
        ..resource_error(path, line, code, message)
    }
}

#[cfg(test)]
mod tests {
    use era_protocol::ProtocolBytes;
    use era_runtime_protocol::{ProjectManifest, SubmittedFile};

    use super::*;

    #[test]
    fn parses_static_and_animation_sprites_then_validates_metadata() {
        let manifest = ProjectManifest {
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "resources/sprites.csv".into(),
                    category: FileCategory::ResourceManifest,
                    payload: FilePayload::Utf8(
                        "FACE,face.png,0,0,32,16,1,2\nRUN,ANIME,10,20\nRUN,face.png,0,0,8,8,0,0,50"
                            .into(),
                    ),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "resources/face.png".into(),
                    category: FileCategory::Resource,
                    payload: FilePayload::Bytes(ProtocolBytes::new(vec![1, 2, 3])),
                    content_hash: None,
                },
            ],
        };
        let (mut graph, diagnostics) = ResourceGraph::from_manifest(&manifest);
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
        assert_eq!(graph.metadata_requests().len(), 1);
        graph
            .apply_metadata(
                "resources/face.png",
                ImageMetadataResponse {
                    width: 64,
                    height: 32,
                    format: "png".into(),
                    animated: false,
                },
            )
            .unwrap();
        assert_eq!(graph.sprite("face").unwrap().width, 32);
        assert_eq!(graph.sprite("run").unwrap().width, 10);
    }

    #[test]
    fn canvas_and_dynamic_sprite_mutations_form_a_deterministic_replay_graph() {
        let mut graph = ResourceGraph::default();
        assert_eq!(graph.create_canvas(3, 64, 32), Ok(true));
        assert_eq!(graph.create_canvas(3, 1, 1), Ok(false));
        assert!(graph.clear_canvas(3, 0x00ff_00ff, None));
        assert!(graph.create_canvas_sprite("generated", 3, None));
        assert!(graph.create_animation_sprite("animated", 16, 16));
        assert!(graph.add_animation_frame("animated", 3, [0, 0, 16, 16], [2, 3], 55,));
        assert_eq!(
            graph
                .sprite("GENERATED")
                .map(|sprite| (sprite.width, sprite.height)),
            Some((64, 32))
        );
        assert!(graph.move_sprite("generated", 4, 5, false));
        assert_eq!(graph.set_animation_timer(55), Ok(()));
        assert_eq!(graph.animation_timer(), 55);
        assert_eq!(
            graph
                .sprite("generated")
                .map(|sprite| (sprite.position_x, sprite.position_y)),
            Some((4, 5))
        );
        let replay = graph.replay();
        assert_eq!(replay.canvases.len(), 1);
        assert_eq!(replay.canvases[0].revision, 1);
        let animated = replay
            .sprites
            .iter()
            .find(|sprite| sprite.name == "ANIMATED")
            .unwrap();
        assert_eq!(animated.frames[0].canvas_id, Some(3));
        assert_eq!(animated.frames[0].delay_ms, 55);
        assert!(
            replay
                .sprites
                .iter()
                .any(|sprite| sprite.canvas_id == Some(3))
        );
        assert_eq!(replay.animation_timer_ms, 55);
        assert_eq!(graph.dispose_sprites(false), 2);
        assert!(graph.dispose_canvas(3));
    }
}
