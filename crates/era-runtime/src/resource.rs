use std::collections::BTreeMap;

use era_runtime_protocol::{
    CanvasReplay, CanvasReplayCommand, CanvasSize, FileCategory, FilePayload,
    ImageMetadataResponse, ProjectManifest, ResourceReplay, SpriteFrameReplay, SpriteReplay,
    validate_relative_path,
};
use serde::{Deserialize, Serialize};

mod sprite;

pub(crate) use sprite::{SpriteDefinition, SpriteFrame};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ResourceGraph {
    images: BTreeMap<String, ResourceImage>,
    sprites: BTreeMap<String, SpriteDefinition>,
    canvases: BTreeMap<i64, CanvasSurface>,
    animation_timer_ms: i32,
    canvas_defaults: CanvasDefaults,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CanvasDefaults {
    brush_argb: u32,
    pen_argb: u32,
    font_family: String,
    font_size: i64,
    font_style: u8,
}

impl Default for ResourceGraph {
    fn default() -> Self {
        Self {
            images: BTreeMap::new(),
            sprites: BTreeMap::new(),
            canvases: BTreeMap::new(),
            animation_timer_ms: 0,
            canvas_defaults: CanvasDefaults {
                brush_argb: 0xff00_0000,
                pen_argb: 0xffc0_c0c0,
                font_family: "sans-serif".into(),
                font_size: 100,
                font_style: 0,
            },
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CanvasSurface {
    width: u32,
    height: u32,
    revision: u64,
    commands: Vec<CanvasCommand>,
    brush_argb: u32,
    pen_argb: u32,
    pen_width: i64,
    dash_style: i64,
    dash_cap: i64,
    font_family: String,
    font_size: i64,
    font_style: u8,
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
        color_matrix: Option<Vec<i64>>,
    },
    SetPixel {
        point: [i32; 2],
        argb: u32,
    },
    FillRectangle {
        rectangle: [i32; 4],
        brush_argb: u32,
    },
    SetBrush {
        argb: u32,
    },
    SetPen {
        argb: u32,
        width: i64,
    },
    SetDashStyle {
        style: i64,
        cap: i64,
    },
    SetFont {
        family: String,
        size: i64,
        style_bits: u8,
    },
    DrawLine {
        start: [i32; 2],
        end: [i32; 2],
    },
    DrawText {
        text: String,
        point: [i32; 2],
    },
    DrawCanvas {
        source_canvas_id: i64,
        source_revision: u64,
        source: [i32; 4],
        destination: [i32; 4],
        color_matrix: Option<Vec<i64>>,
        mask_canvas_id: Option<i64>,
        rotation_millidegrees: i64,
        rotation_center: Option<[i32; 2]>,
    },
    LoadEncodedImage {
        content_digest: Vec<u8>,
        encoded: Vec<u8>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ResourceImage {
    relative_path: String,
    digest: [u8; 32],
    metadata: Option<ImageMetadata>,
    // Retained in the serialized snapshot schema for compatibility. Static file
    // payloads are frontend-owned, so new graphs always leave this empty.
    bytes: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ImageMetadata {
    width: u32,
    height: u32,
    format: String,
    animated: bool,
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
    /// Copy runtime resource state for a snapshot.
    ///
    /// Static resource payloads remain frontend-owned and are fetched lazily, so
    /// the graph contains only identities and mutable runtime metadata.
    pub(crate) fn compact_snapshot(&self) -> Self {
        self.clone()
    }

    /// Verify that a snapshot refers to the exact static resource set loaded by
    /// the current project.
    pub(crate) fn validate_project_resources(&mut self, project: &Self) -> Result<(), String> {
        if self.images.len() != project.images.len() {
            return Err("runtime snapshot resource list differs from the loaded project".into());
        }
        for (key, image) in &mut self.images {
            let source = project.images.get(key).ok_or_else(|| {
                format!("runtime snapshot resource {key} is absent from the loaded project")
            })?;
            if image.relative_path != source.relative_path || image.digest != source.digest {
                return Err(format!(
                    "runtime snapshot resource {} differs from the loaded project",
                    image.relative_path
                ));
            }
            image.bytes.clear();
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn embedded_project_bytes(&self) -> usize {
        self.images.values().map(|image| image.bytes.len()).sum()
    }

    #[cfg(test)]
    pub(crate) fn from_manifest(manifest: &ProjectManifest) -> (Self, Vec<ResourceDiagnostic>) {
        Self::from_manifest_with_progress(manifest, |_, _| {})
    }

    pub(crate) fn work_item_count(manifest: &ProjectManifest) -> usize {
        manifest
            .files
            .iter()
            .filter(|file| {
                matches!(
                    file.category,
                    FileCategory::Resource | FileCategory::ResourceManifest
                )
            })
            .count()
    }

    pub(crate) fn from_manifest_with_progress(
        manifest: &ProjectManifest,
        mut progress: impl FnMut(usize, usize),
    ) -> (Self, Vec<ResourceDiagnostic>) {
        let mut graph = Self::default();
        let mut diagnostics = Vec::new();
        let mut preloaded_metadata = Vec::new();
        let total = Self::work_item_count(manifest);
        let mut completed = 0;
        progress(completed, total);
        for file in manifest
            .files
            .iter()
            .filter(|file| file.category == FileCategory::Resource)
        {
            if let Ok(path) = validate_relative_path(&file.relative_path)
                && let Some(bytes) = match &file.payload {
                    FilePayload::Utf8(value) => Some(value.as_bytes()),
                    FilePayload::Bytes(value) => Some(value.as_slice()),
                    FilePayload::ExternalResource(_) => Some(&[][..]),
                    FilePayload::IoError(_) => None,
                }
            {
                let digest = file
                    .content_hash
                    .as_ref()
                    .and_then(|hash| <[u8; 32]>::try_from(hash.as_slice()).ok())
                    .unwrap_or_else(|| *blake3::hash(bytes).as_bytes());
                if let FilePayload::ExternalResource(resource) = &file.payload
                    && let Some(metadata) = resource.image_metadata.clone()
                {
                    preloaded_metadata.push((path.clone(), metadata));
                }
                graph.images.insert(
                    path.to_ascii_lowercase(),
                    ResourceImage {
                        relative_path: path,
                        digest,
                        metadata: None,
                        bytes: Vec::new(),
                    },
                );
            }
            completed += 1;
            progress(completed, total);
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
                completed += 1;
                progress(completed, total);
                continue;
            };
            parse_resource_manifest(&mut graph, &mut diagnostics, &manifest.relative_path, text);
            completed += 1;
            progress(completed, total);
        }
        for (path, metadata) in preloaded_metadata {
            if let Err(message) = graph.apply_metadata(&path, metadata) {
                diagnostics.push(ResourceDiagnostic {
                    code: "runtime.invalid_image_metadata",
                    path,
                    line: None,
                    message,
                    error: false,
                });
            }
        }
        (graph, diagnostics)
    }

    pub(crate) fn configure_canvas_defaults(
        &mut self,
        foreground_rgb: i64,
        background_rgb: i64,
        font_family: String,
        font_style: u8,
    ) {
        self.canvas_defaults = CanvasDefaults {
            brush_argb: opaque_rgb(background_rgb),
            pen_argb: opaque_rgb(foreground_rgb),
            font_family,
            // The reference canvas fallback font is deliberately 100 pixels,
            // independently of the normal console font size.
            font_size: 100,
            font_style,
        };
    }

    pub(crate) fn metadata_requests(&self) -> Vec<(String, [u8; 32])> {
        self.images
            .values()
            .filter(|image| is_image_path(&image.relative_path))
            .filter(|image| image.metadata.is_none())
            .map(|image| (image.relative_path.clone(), image.digest))
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
        if !matches!(
            metadata.format.as_str(),
            "png" | "bmp" | "gif" | "jpeg" | "webp"
        ) {
            return Err("image metadata format is unsupported".into());
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

    pub(crate) fn audio_path(&self, name: &str) -> Option<&str> {
        let normalized = name.replace('\\', "/");
        let key = normalized.to_ascii_lowercase();
        self.images
            .get(&key)
            .or_else(|| self.images.get(&format!("sound/{key}")))
            .map(|resource| resource.relative_path.as_str())
    }

    pub(crate) fn contains_audio(&self, name: &str) -> bool {
        self.audio_path(name).is_some()
    }

    fn image_from_content_directory(&self, path: &str) -> Option<&ResourceImage> {
        let key = path.to_ascii_lowercase();
        if key.starts_with("resources/") {
            return self.images.get(&key);
        }
        self.images
            .get(&format!("resources/{key}"))
            // Keep accepting explicit project-root resource paths used by older
            // RustyEra clients after applying Emuera's ContentDir precedence.
            .or_else(|| self.images.get(&key))
    }

    /// Static project images keep only their digest in runtime-owned canvas state.
    /// An empty encoded payload distinguishes that stable resource reference from
    /// GLOAD data, whose bytes must remain self-contained in the replay command.
    fn project_image_reference(
        &self,
        content_digest: &[u8],
        encoded: &[u8],
    ) -> Option<&ResourceImage> {
        if !encoded.is_empty() {
            return None;
        }
        self.images
            .values()
            .find(|image| image.digest.as_slice() == content_digest)
    }

    // This is deliberately one exhaustive translation table so adding an
    // internal command cannot silently omit its public replay equivalent.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn replay(&self) -> ResourceReplay {
        let mut sprites = self
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
            .collect::<Vec<_>>();
        let mut replay_resource_ordinal = 0_u64;
        let canvases = self
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
                    .map(|command| {
                        if let CanvasCommand::LoadEncodedImage {
                            content_digest,
                            encoded,
                        } = command
                            && let Some(image) =
                                self.project_image_reference(content_digest, encoded)
                            && let Some(metadata) = &image.metadata
                        {
                            let name = loop {
                                let candidate = format!(
                                    "__RUSTYERA_PROJECT_RESOURCE_{replay_resource_ordinal}"
                                );
                                replay_resource_ordinal = replay_resource_ordinal.saturating_add(1);
                                if !sprites
                                    .iter()
                                    .any(|sprite| sprite.name.eq_ignore_ascii_case(&candidate))
                                {
                                    break candidate;
                                }
                            };
                            sprites.push(SpriteReplay {
                                name: name.clone(),
                                size: [metadata.width, metadata.height],
                                position: [0, 0],
                                frames: vec![SpriteFrameReplay {
                                    resource_id: image.relative_path.clone(),
                                    source_rectangle: [
                                        0,
                                        0,
                                        i32::try_from(metadata.width).unwrap_or(i32::MAX),
                                        i32::try_from(metadata.height).unwrap_or(i32::MAX),
                                    ],
                                    offset: [0, 0],
                                    delay_ms: 1_000,
                                    destination_size: None,
                                    canvas_id: None,
                                }],
                                canvas_id: None,
                                canvas_rectangle: None,
                            });
                            return CanvasReplayCommand::DrawSprite {
                                name,
                                destination: canvas_rect([
                                    0,
                                    0,
                                    i32::try_from(canvas.width).unwrap_or(i32::MAX),
                                    i32::try_from(canvas.height).unwrap_or(i32::MAX),
                                ]),
                                color_matrix: None,
                            };
                        }
                        match command {
                            CanvasCommand::Clear { argb, rectangle } => {
                                CanvasReplayCommand::Clear {
                                    argb: *argb,
                                    rectangle: rectangle.map(canvas_rect),
                                }
                            }
                            CanvasCommand::DrawSprite {
                                name,
                                destination,
                                color_matrix,
                            } => CanvasReplayCommand::DrawSprite {
                                name: name.clone(),
                                destination: canvas_rect(*destination),
                                color_matrix: color_matrix.clone(),
                            },
                            CanvasCommand::SetPixel { point, argb } => {
                                CanvasReplayCommand::SetPixel {
                                    point: era_runtime_protocol::CanvasPoint {
                                        x: point[0],
                                        y: point[1],
                                    },
                                    argb: *argb,
                                }
                            }
                            CanvasCommand::FillRectangle {
                                rectangle,
                                brush_argb,
                            } => CanvasReplayCommand::FillRectangle {
                                rectangle: canvas_rect(*rectangle),
                                brush_argb: *brush_argb,
                            },
                            CanvasCommand::SetBrush { argb } => {
                                CanvasReplayCommand::SetBrush { argb: *argb }
                            }
                            CanvasCommand::SetPen { argb, width } => CanvasReplayCommand::SetPen {
                                argb: *argb,
                                width: *width,
                            },
                            CanvasCommand::SetDashStyle { style, cap } => {
                                CanvasReplayCommand::SetDashStyle {
                                    style: *style,
                                    cap: *cap,
                                }
                            }
                            CanvasCommand::SetFont {
                                family,
                                size,
                                style_bits,
                            } => CanvasReplayCommand::SetFont {
                                family: family.clone(),
                                size: *size,
                                style_bits: *style_bits,
                            },
                            CanvasCommand::DrawLine { start, end } => {
                                CanvasReplayCommand::DrawLine {
                                    start: era_runtime_protocol::CanvasPoint {
                                        x: start[0],
                                        y: start[1],
                                    },
                                    end: era_runtime_protocol::CanvasPoint {
                                        x: end[0],
                                        y: end[1],
                                    },
                                }
                            }
                            CanvasCommand::DrawText { text, point } => {
                                CanvasReplayCommand::DrawText {
                                    text: text.clone(),
                                    point: era_runtime_protocol::CanvasPoint {
                                        x: point[0],
                                        y: point[1],
                                    },
                                }
                            }
                            CanvasCommand::DrawCanvas {
                                source_canvas_id,
                                source_revision,
                                source,
                                destination,
                                color_matrix,
                                mask_canvas_id,
                                rotation_millidegrees,
                                rotation_center,
                            } => CanvasReplayCommand::DrawCanvas {
                                source_canvas_id: *source_canvas_id,
                                source_revision: *source_revision,
                                source: canvas_rect(*source),
                                destination: canvas_rect(*destination),
                                color_matrix: color_matrix.clone(),
                                mask_canvas_id: *mask_canvas_id,
                                rotation_millidegrees: *rotation_millidegrees,
                                rotation_center: rotation_center.map(|point| {
                                    era_runtime_protocol::CanvasPoint {
                                        x: point[0],
                                        y: point[1],
                                    }
                                }),
                            },
                            CanvasCommand::LoadEncodedImage {
                                content_digest,
                                encoded,
                            } => CanvasReplayCommand::LoadEncodedImage {
                                content_digest: content_digest.clone(),
                                encoded: encoded.clone(),
                            },
                        }
                    })
                    .collect(),
                revision: canvas.revision,
            })
            .collect();
        ResourceReplay {
            sprites,
            canvases,
            animation_timer_ms: self.animation_timer_ms,
        }
    }

    pub(crate) fn set_animation_timer(&mut self, milliseconds: i64) {
        self.animation_timer_ms = if milliseconds <= 0 {
            0
        } else {
            i32::try_from(milliseconds.clamp(10, i64::from(i16::MAX)))
                .expect("clamped animation timer fits i32")
        };
    }

    #[cfg(test)]
    pub(crate) const fn animation_timer(&self) -> i32 {
        self.animation_timer_ms
    }
}

fn is_image_path(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, extension)| {
        matches!(
            extension.to_ascii_lowercase().as_str(),
            "png" | "bmp" | "gif" | "jpg" | "jpeg" | "webp"
        )
    })
}

mod canvas;
mod manifest;
#[cfg(test)]
mod tests;

use self::canvas::{canvas_rect, opaque_rgb};
use self::manifest::parse_resource_manifest;
