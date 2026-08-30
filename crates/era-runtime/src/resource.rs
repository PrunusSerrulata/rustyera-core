use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::mem::size_of;

use era_protocol::ProtocolBytes;
use era_runtime_protocol::{
    CanvasReplay, CanvasReplayCommand, CanvasSize, FileCategory, FilePayload,
    ImageMetadataResponse, ProjectManifest, ResourceReplay, SceneSourceV1, SpriteFrameReplay,
    SpriteReplay, validate_relative_path,
};
use serde::{Deserialize, Serialize};

mod sprite;

pub(crate) use sprite::{SpriteDefinition, SpriteFrame};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ResourceGraph {
    images: BTreeMap<String, ResourceImage>,
    sprites: BTreeMap<String, SpriteDefinition>,
    canvases: BTreeMap<i64, CanvasSurface>,
    /// Immutable definitions retained only while an exact revision is referenced.
    #[serde(default)]
    exact_revisions: ExactRevisionStore,
    #[serde(skip, default)]
    retained_canvas_command_bytes: usize,
    animation_timer_ms: i32,
    canvas_defaults: CanvasDefaults,
    #[serde(default)]
    static_sprite_revision: u64,
    #[serde(default = "default_next_sprite_revision")]
    next_sprite_revision: u64,
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
            exact_revisions: ExactRevisionStore::default(),
            retained_canvas_command_bytes: 0,
            animation_timer_ms: 0,
            canvas_defaults: CanvasDefaults {
                brush_argb: 0xff00_0000,
                pen_argb: 0xffc0_c0c0,
                font_family: "sans-serif".into(),
                font_size: 100,
                font_style: 0,
            },
            static_sprite_revision: 0,
            next_sprite_revision: default_next_sprite_revision(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct ExactRevisionStore {
    sprites: BTreeMap<String, BTreeMap<u64, SpriteDefinition>>,
    canvases: BTreeMap<i64, BTreeMap<u64, CanvasSurface>>,
    #[serde(skip, default)]
    retained_canvas_command_bytes: usize,
}

#[derive(Clone, Debug, Default)]
struct ReplayClosure {
    sprites: BTreeSet<(String, u64)>,
    canvases: BTreeSet<(i64, u64)>,
    retained_sprites: BTreeSet<(String, u64)>,
    retained_canvases: BTreeSet<(i64, u64)>,
}

enum ReplayWork {
    Sprite(String, u64, bool),
    Canvas(i64, u64, bool),
}

const fn default_next_sprite_revision() -> u64 {
    1
}

fn static_sprite_revision(manifest: &ProjectManifest) -> u64 {
    let mut files = manifest
        .files
        .iter()
        .filter(|file| {
            matches!(
                file.category,
                FileCategory::Resource | FileCategory::ResourceManifest
            )
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|file| {
        (
            file.relative_path.to_ascii_lowercase(),
            file.relative_path.as_str(),
        )
    });
    let mut hasher = blake3::Hasher::new_derive_key("rustyera.static-sprite-revision.v1");
    for file in files {
        hasher.update(file.relative_path.as_bytes());
        hasher.update(&[file.category as u8]);
        if let Some(content_hash) = &file.content_hash {
            hasher.update(content_hash.as_slice());
            continue;
        }
        match &file.payload {
            FilePayload::Utf8(value) => {
                hasher.update(value.as_bytes());
            }
            FilePayload::Bytes(value) => {
                hasher.update(value.as_slice());
            }
            FilePayload::ExternalResource(resource) => {
                hasher.update(&resource.byte_length.to_le_bytes());
                if let Some(metadata) = &resource.image_metadata {
                    hasher.update(&metadata.width.to_le_bytes());
                    hasher.update(&metadata.height.to_le_bytes());
                    hasher.update(metadata.format.as_bytes());
                    hasher.update(&[u8::from(metadata.animated)]);
                }
            }
            FilePayload::IoError(_) => {
                hasher.update(b"io-error");
            }
        }
    }
    let digest = hasher.finalize();
    u64::from_le_bytes(
        digest.as_bytes()[..size_of::<u64>()]
            .try_into()
            .expect("BLAKE3 digest contains a u64"),
    )
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CanvasSurface {
    width: u32,
    height: u32,
    revision: u64,
    commands: Vec<CanvasCommand>,
    #[serde(default)]
    polygon_points: Vec<[i32; 2]>,
    #[serde(skip, default)]
    retained_command_bytes: usize,
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
        resource_revision: u64,
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
        mask_revision: Option<u64>,
        rotation_millidegrees: i64,
        rotation_center: Option<[i32; 2]>,
    },
    LoadEncodedImage {
        content_digest: Vec<u8>,
        encoded: Vec<u8>,
    },
    PolygonPointAdd {
        point: [i32; 2],
    },
    PolygonPointClear,
    DrawPolygon,
    FillPolygon,
}

impl CanvasCommand {
    fn retained_bytes(&self) -> usize {
        let dynamic = match self {
            Self::DrawSprite {
                name, color_matrix, ..
            } => name.len().saturating_add(
                color_matrix
                    .as_ref()
                    .map_or(0, |values| values.len().saturating_mul(size_of::<i64>())),
            ),
            Self::SetFont { family, .. } => family.len(),
            Self::DrawText { text, .. } => text.len(),
            Self::DrawCanvas { color_matrix, .. } => color_matrix
                .as_ref()
                .map_or(0, |values| values.len().saturating_mul(size_of::<i64>())),
            Self::LoadEncodedImage {
                content_digest,
                encoded,
            } => content_digest.len().saturating_add(encoded.len()),
            Self::Clear { .. }
            | Self::SetPixel { .. }
            | Self::FillRectangle { .. }
            | Self::SetBrush { .. }
            | Self::SetPen { .. }
            | Self::SetDashStyle { .. }
            | Self::DrawLine { .. }
            | Self::PolygonPointAdd { .. }
            | Self::PolygonPointClear
            | Self::DrawPolygon
            | Self::FillPolygon => 0,
        };
        size_of::<Self>().saturating_add(dynamic)
    }
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
        self.ensure_canvas_retained_bytes();
        let mut roots = self
            .exact_revisions
            .sprites
            .iter()
            .flat_map(|(name, revisions)| {
                revisions.keys().map(|revision| SceneSourceV1::Sprite {
                    sprite_name: name.clone(),
                    resource_revision: *revision,
                })
            })
            .collect::<Vec<_>>();
        roots.extend(
            self.exact_revisions
                .canvases
                .iter()
                .flat_map(|(canvas_id, revisions)| {
                    revisions.keys().map(|revision| SceneSourceV1::Canvas {
                        canvas_id: *canvas_id,
                        resource_revision: *revision,
                    })
                }),
        );
        let (exact_revisions, _) = self.collect_replay_closure(&roots, true)?;
        if self.total_canvas_bytes_with(&exact_revisions) > canvas::MAXIMUM_CANVAS_COMMAND_BYTES {
            return Err(
                "runtime snapshot exact replay closure exceeds canvas command budget".into(),
            );
        }
        self.exact_revisions = exact_revisions;
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
        let mut graph = Self {
            static_sprite_revision: static_sprite_revision(manifest),
            next_sprite_revision: default_next_sprite_revision(),
            ..Self::default()
        };
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
            .ok_or("image resource is unavailable")?;
        let digest = image.digest;
        let image = image
            .metadata
            .as_ref()
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
                frame.content_digest = Some(digest);
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

    pub(crate) fn create_file_sprite(
        &mut self,
        name: &str,
        requested_path: &str,
        declaring_source: Option<&str>,
        relative_to_source: bool,
    ) -> bool {
        let key = name.to_ascii_uppercase();
        if name.is_empty() {
            return false;
        }
        if self.sprites.contains_key(&key) {
            // The fixed snake implementation treats repeated file-backed creation
            // as an idempotent success, unlike canvas-backed SPRITECREATE.
            return true;
        }
        let Ok(requested_path) = validate_relative_path(requested_path) else {
            return false;
        };
        let resolved = if relative_to_source {
            let Some(source) = declaring_source.and_then(|path| validate_relative_path(path).ok())
            else {
                return false;
            };
            let directory = source
                .rsplit_once('/')
                .map_or("", |(directory, _)| directory);
            let joined = if directory.is_empty() {
                requested_path
            } else {
                format!("{directory}/{requested_path}")
            };
            let Ok(joined) = validate_relative_path(&joined) else {
                return false;
            };
            joined
        } else {
            requested_path
        };
        let Some(image) = self.images.get(&resolved.to_ascii_lowercase()) else {
            return false;
        };
        let Some(metadata) = &image.metadata else {
            return false;
        };
        let image_path = image.relative_path.clone();
        let content_digest = image.digest;
        let width = metadata.width;
        let height = metadata.height;
        let revision = self.allocate_sprite_revision();
        self.sprites.insert(
            key.clone(),
            SpriteDefinition {
                name: key,
                revision,
                width,
                height,
                frames: vec![SpriteFrame {
                    image_path,
                    content_digest: Some(content_digest),
                    canvas_id: None,
                    canvas_revision: None,
                    source_x: 0,
                    source_y: 0,
                    source_width: Some(width),
                    source_height: Some(height),
                    offset_x: 0,
                    offset_y: 0,
                    delay_ms: 1_000,
                    destination_width: None,
                    destination_height: None,
                }],
                dynamic: true,
                position_x: 0,
                position_y: 0,
                canvas_id: None,
                canvas_revision: None,
                canvas_rectangle: None,
            },
        );
        true
    }

    pub(crate) fn move_sprite(&mut self, name: &str, x: i32, y: i32, relative: bool) -> bool {
        let key = name.to_ascii_uppercase();
        let Some(sprite) = self.sprites.get(&key) else {
            return false;
        };
        let position = if relative {
            (
                sprite.position_x.saturating_add(x),
                sprite.position_y.saturating_add(y),
            )
        } else {
            (x, y)
        };
        if (sprite.position_x, sprite.position_y) == position {
            return true;
        }
        let revision = self.allocate_sprite_revision();
        let sprite = self.sprites.get_mut(&key).expect("sprite was checked");
        sprite.position_x = position.0;
        sprite.position_y = position.1;
        sprite.revision = revision;
        true
    }

    pub(crate) fn sprite_revision(&self, name: &str) -> Option<u64> {
        self.sprite(name).map(|sprite| sprite.revision)
    }

    pub(super) fn allocate_sprite_revision(&mut self) -> u64 {
        let revision = self.next_sprite_revision;
        self.next_sprite_revision = self.next_sprite_revision.saturating_add(1);
        revision
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

    /// Retain the exact immutable definition named by a scene source.
    ///
    /// Resource files are content-addressed already. Mutable sprite and canvas
    /// definitions are copied only when a scene or another replay object creates
    /// an exact revision edge, so history remains the dependency closure rather
    /// than an unbounded mutation journal.
    pub(crate) fn retain_scene_source(&mut self, source: &SceneSourceV1) -> bool {
        self.retain_scene_sources(std::slice::from_ref(source))
    }

    /// Capture a complete immutable dependency closure, then commit it as one unit.
    pub(crate) fn retain_scene_sources(&mut self, sources: &[SceneSourceV1]) -> bool {
        self.ensure_canvas_retained_bytes();
        let Ok((candidate, _)) = self.collect_replay_closure(sources, false) else {
            return false;
        };
        if self.total_canvas_bytes_with(&candidate) > canvas::MAXIMUM_CANVAS_COMMAND_BYTES {
            return false;
        }
        self.exact_revisions = candidate;
        true
    }

    #[cfg(test)]
    pub(crate) fn replay(&self) -> ResourceReplay {
        let Ok((_, closure)) = self.collect_replay_closure(&[], true) else {
            return ResourceReplay::default();
        };
        self.replay_selected(&closure.sprites, &closure.canvases)
    }

    /// Publish current resources plus the exact historical closure required by every pending root.
    /// The exact store is pruned and replaced only after the complete graph has been validated.
    pub(crate) fn replay_for_roots(
        &mut self,
        sources: &[SceneSourceV1],
    ) -> Result<ResourceReplay, String> {
        self.ensure_canvas_retained_bytes();
        let (mut candidate, closure) = self.collect_replay_closure(sources, true)?;
        candidate.sprites.retain(|name, revisions| {
            revisions.retain(|revision, _| {
                closure
                    .retained_sprites
                    .contains(&(name.clone(), *revision))
            });
            !revisions.is_empty()
        });
        candidate.canvases.retain(|canvas_id, revisions| {
            revisions
                .retain(|revision, _| closure.retained_canvases.contains(&(*canvas_id, *revision)));
            !revisions.is_empty()
        });
        candidate.rebuild_retained_bytes();
        if self.total_canvas_bytes_with(&candidate) > canvas::MAXIMUM_CANVAS_COMMAND_BYTES {
            return Err("exact replay dependency closure exceeds canvas command budget".into());
        }
        let replay = self.replay_selected_from(&candidate, &closure.sprites, &closure.canvases);
        replay.validate_exact_references()?;
        self.exact_revisions = candidate;
        Ok(replay)
    }

    // Keeping the work queue in one function makes the all-or-nothing closure construction
    // auditable: no helper may mutate the graph before the complete closure is validated.
    #[allow(clippy::too_many_lines)]
    fn collect_replay_closure(
        &self,
        sources: &[SceneSourceV1],
        include_current: bool,
    ) -> Result<(ExactRevisionStore, ReplayClosure), String> {
        let mut candidate = self.exact_revisions.clone();
        let mut closure = ReplayClosure::default();
        let mut work = VecDeque::new();
        if include_current {
            work.extend(
                self.sprites
                    .values()
                    .map(|sprite| ReplayWork::Sprite(sprite.name.clone(), sprite.revision, false)),
            );
            work.extend(
                self.canvases.iter().map(|(canvas_id, canvas)| {
                    ReplayWork::Canvas(*canvas_id, canvas.revision, false)
                }),
            );
        }
        for source in sources {
            match source {
                SceneSourceV1::Resource {
                    resource_id,
                    resource_revision,
                } => {
                    let valid = *resource_revision == self.static_sprite_revision
                        && self
                            .images
                            .values()
                            .any(|image| image.relative_path.eq_ignore_ascii_case(resource_id));
                    if !valid {
                        return Err(format!(
                            "missing exact project resource {resource_id}@{resource_revision}"
                        ));
                    }
                }
                SceneSourceV1::Sprite {
                    sprite_name,
                    resource_revision,
                } => work.push_back(ReplayWork::Sprite(
                    sprite_name.to_ascii_uppercase(),
                    *resource_revision,
                    true,
                )),
                SceneSourceV1::Canvas {
                    canvas_id,
                    resource_revision,
                } => {
                    work.push_back(ReplayWork::Canvas(*canvas_id, *resource_revision, true));
                }
            }
        }
        while let Some(item) = work.pop_front() {
            match item {
                ReplayWork::Sprite(name, revision, retain_exact) => {
                    let key = (name.to_ascii_uppercase(), revision);
                    let first_visit = closure.sprites.insert(key.clone());
                    let already_exact = candidate
                        .sprites
                        .get(&key.0)
                        .is_some_and(|revisions| revisions.contains_key(&revision));
                    if retain_exact {
                        closure.retained_sprites.insert(key.clone());
                    }
                    if !first_visit && (!retain_exact || already_exact) {
                        continue;
                    }
                    let sprite = self
                        .sprites
                        .get(&key.0)
                        .filter(|sprite| sprite.revision == revision)
                        .or_else(|| candidate.sprites.get(&key.0)?.get(&revision))
                        .cloned()
                        .ok_or_else(|| format!("missing exact sprite {}@{}", key.0, revision))?;
                    for frame in &sprite.frames {
                        if let Some(digest) = frame.content_digest {
                            let matches_manifest = self
                                .images
                                .get(&frame.image_path.to_ascii_lowercase())
                                .is_some_and(|image| image.digest == digest);
                            if !matches_manifest {
                                return Err(format!(
                                    "exact sprite {}@{} refers to a changed project resource {}",
                                    key.0, revision, frame.image_path
                                ));
                            }
                        }
                    }
                    validate_canvas_pair(sprite.canvas_id, sprite.canvas_revision, "sprite")?;
                    if retain_exact {
                        candidate
                            .sprites
                            .entry(key.0)
                            .or_default()
                            .insert(revision, sprite.clone());
                    }
                    if let Some((canvas_id, revision)) =
                        sprite.canvas_id.zip(sprite.canvas_revision)
                    {
                        work.push_back(ReplayWork::Canvas(canvas_id, revision, true));
                    }
                    for frame in &sprite.frames {
                        validate_canvas_pair(
                            frame.canvas_id,
                            frame.canvas_revision,
                            "sprite frame",
                        )?;
                        if let Some((canvas_id, revision)) =
                            frame.canvas_id.zip(frame.canvas_revision)
                        {
                            work.push_back(ReplayWork::Canvas(canvas_id, revision, true));
                        }
                    }
                }
                ReplayWork::Canvas(canvas_id, revision, retain_exact) => {
                    let key = (canvas_id, revision);
                    let first_visit = closure.canvases.insert(key);
                    let already_exact = candidate
                        .canvases
                        .get(&canvas_id)
                        .is_some_and(|revisions| revisions.contains_key(&revision));
                    if retain_exact {
                        closure.retained_canvases.insert(key);
                    }
                    if !first_visit && (!retain_exact || already_exact) {
                        continue;
                    }
                    let canvas = self
                        .canvases
                        .get(&canvas_id)
                        .filter(|canvas| canvas.revision == revision)
                        .or_else(|| candidate.canvases.get(&canvas_id)?.get(&revision))
                        .cloned()
                        .ok_or_else(|| format!("missing exact canvas {canvas_id}@{revision}"))?;
                    if retain_exact {
                        candidate
                            .canvases
                            .entry(canvas_id)
                            .or_default()
                            .insert(revision, canvas.clone());
                    }
                    for command in &canvas.commands {
                        match command {
                            CanvasCommand::DrawSprite {
                                name,
                                resource_revision,
                                ..
                            } => work.push_back(ReplayWork::Sprite(
                                name.to_ascii_uppercase(),
                                *resource_revision,
                                true,
                            )),
                            CanvasCommand::DrawCanvas {
                                source_canvas_id,
                                source_revision,
                                mask_canvas_id,
                                mask_revision,
                                ..
                            } => {
                                validate_canvas_pair(
                                    *mask_canvas_id,
                                    *mask_revision,
                                    "canvas mask",
                                )?;
                                work.push_back(ReplayWork::Canvas(
                                    *source_canvas_id,
                                    *source_revision,
                                    true,
                                ));
                                if let Some((mask, revision)) =
                                    (*mask_canvas_id).zip(*mask_revision)
                                {
                                    work.push_back(ReplayWork::Canvas(mask, revision, true));
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        candidate.rebuild_retained_bytes();
        Ok((candidate, closure))
    }

    fn sprite_definition_at(&self, key: &(String, u64)) -> Option<&SpriteDefinition> {
        self.sprites
            .get(&key.0)
            .filter(|sprite| sprite.revision == key.1)
            .or_else(|| {
                self.exact_revisions
                    .sprites
                    .get(&key.0)
                    .and_then(|revisions| revisions.get(&key.1))
            })
    }

    fn canvas_definition_at(&self, key: (i64, u64)) -> Option<&CanvasSurface> {
        self.canvases
            .get(&key.0)
            .filter(|canvas| canvas.revision == key.1)
            .or_else(|| {
                self.exact_revisions
                    .canvases
                    .get(&key.0)
                    .and_then(|revisions| revisions.get(&key.1))
            })
    }

    fn replay_selected_from(
        &self,
        exact_revisions: &ExactRevisionStore,
        sprite_keys: &BTreeSet<(String, u64)>,
        canvas_keys: &BTreeSet<(i64, u64)>,
    ) -> ResourceReplay {
        let mut graph = self.clone();
        graph.exact_revisions.clone_from(exact_revisions);
        graph.replay_selected(sprite_keys, canvas_keys)
    }

    fn total_canvas_bytes_with(&self, exact_revisions: &ExactRevisionStore) -> usize {
        let historical = exact_revisions
            .canvases
            .iter()
            .flat_map(|(canvas_id, revisions)| {
                revisions.iter().filter_map(move |(revision, canvas)| {
                    let is_current = self
                        .canvases
                        .get(canvas_id)
                        .is_some_and(|current| current.revision == *revision);
                    (!is_current).then_some(canvas.retained_command_bytes)
                })
            })
            .fold(0, usize::saturating_add);
        self.retained_canvas_command_bytes
            .saturating_add(historical)
    }

    // This is deliberately one exhaustive translation table so adding an
    // internal command cannot silently omit its public replay equivalent.
    #[allow(clippy::too_many_lines)]
    fn replay_selected(
        &self,
        sprite_keys: &BTreeSet<(String, u64)>,
        canvas_keys: &BTreeSet<(i64, u64)>,
    ) -> ResourceReplay {
        let mut sprites = sprite_keys
            .iter()
            .filter_map(|key| self.sprite_definition_at(key))
            .map(|sprite| SpriteReplay {
                name: sprite.name.clone(),
                revision: sprite.revision,
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
                        canvas_revision: frame.canvas_revision,
                        content_digest: frame
                            .content_digest
                            .map(|digest| ProtocolBytes::new(digest.to_vec())),
                    })
                    .collect(),
                canvas_id: sprite.canvas_id,
                canvas_revision: sprite.canvas_revision,
                canvas_rectangle: sprite.canvas_rectangle.map(canvas_rect),
            })
            .collect::<Vec<_>>();
        let mut replay_resource_ordinal = 0_u64;
        let canvases = canvas_keys
            .iter()
            .filter_map(|key| {
                self.canvas_definition_at(*key)
                    .map(|canvas| (key.0, canvas))
            })
            .map(|(canvas_id, canvas)| CanvasReplay {
                canvas_id,
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
                                revision: canvas.revision,
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
                                    canvas_revision: None,
                                    content_digest: Some(ProtocolBytes::new(image.digest.to_vec())),
                                }],
                                canvas_id: None,
                                canvas_revision: None,
                                canvas_rectangle: None,
                            });
                            return CanvasReplayCommand::DrawSprite {
                                name,
                                resource_revision: canvas.revision,
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
                                resource_revision,
                                destination,
                                color_matrix,
                            } => CanvasReplayCommand::DrawSprite {
                                name: name.clone(),
                                resource_revision: *resource_revision,
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
                                mask_revision,
                                rotation_millidegrees,
                                rotation_center,
                            } => CanvasReplayCommand::DrawCanvas {
                                source_canvas_id: *source_canvas_id,
                                source_revision: *source_revision,
                                source: canvas_rect(*source),
                                destination: canvas_rect(*destination),
                                color_matrix: color_matrix.clone(),
                                mask_canvas_id: *mask_canvas_id,
                                mask_revision: *mask_revision,
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
                                content_digest: ProtocolBytes::new(content_digest.clone()),
                                encoded: ProtocolBytes::new(encoded.clone()),
                            },
                            CanvasCommand::PolygonPointAdd { point } => {
                                CanvasReplayCommand::PolygonPointAdd {
                                    point: era_runtime_protocol::CanvasPoint {
                                        x: point[0],
                                        y: point[1],
                                    },
                                }
                            }
                            CanvasCommand::PolygonPointClear => {
                                CanvasReplayCommand::PolygonPointClear
                            }
                            CanvasCommand::DrawPolygon => CanvasReplayCommand::DrawPolygon,
                            CanvasCommand::FillPolygon => CanvasReplayCommand::FillPolygon,
                        }
                    })
                    .collect(),
                revision: canvas.revision,
            })
            .collect();
        sprites
            .sort_by(|left, right| (&left.name, left.revision).cmp(&(&right.name, right.revision)));
        ResourceReplay {
            sprites,
            canvases,
            animation_timer_ms: self.animation_timer_ms,
        }
    }

    pub(crate) fn set_animation_timer(&mut self, milliseconds: i64) -> bool {
        if !(i64::from(i32::MIN)..=i64::from(i16::MAX)).contains(&milliseconds) {
            return false;
        }
        self.animation_timer_ms = if milliseconds <= 0 {
            0
        } else {
            i32::try_from(milliseconds.max(10)).expect("clamped animation timer fits i32")
        };
        true
    }

    pub(crate) const fn animation_timer(&self) -> i32 {
        self.animation_timer_ms
    }
}

impl ExactRevisionStore {
    fn rebuild_retained_bytes(&mut self) {
        for canvas in self.canvases.values_mut().flat_map(BTreeMap::values_mut) {
            canvas.retained_command_bytes = canvas
                .commands
                .iter()
                .map(CanvasCommand::retained_bytes)
                .fold(0, usize::saturating_add);
        }
        self.retained_canvas_command_bytes = self
            .canvases
            .values()
            .flat_map(BTreeMap::values)
            .map(|canvas| canvas.retained_command_bytes)
            .fold(0, usize::saturating_add);
    }
}

fn validate_canvas_pair(
    canvas_id: Option<i64>,
    canvas_revision: Option<u64>,
    owner: &str,
) -> Result<(), String> {
    if canvas_id.is_some() == canvas_revision.is_some() {
        Ok(())
    } else {
        Err(format!("{owner} has a partial exact canvas reference"))
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
