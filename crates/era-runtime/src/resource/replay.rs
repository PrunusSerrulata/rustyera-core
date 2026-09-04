use super::canvas::{self, canvas_rect};
use super::{
    CanvasCommand, CanvasSurface, ExactRevisionStore, ReplayClosure, ReplayWork, ResourceGraph,
    SpriteDefinition,
};
use era_protocol::ProtocolBytes;
use era_runtime_protocol::{
    CanvasReplay, CanvasReplayCommand, CanvasSize, ResourceReplay, SceneSourceV1,
    SpriteFrameReplay, SpriteReplay,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

impl ResourceGraph {
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
    pub(super) fn collect_replay_closure(
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

    pub(super) fn total_canvas_bytes_with(&self, exact_revisions: &ExactRevisionStore) -> usize {
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
}
impl ExactRevisionStore {
    pub(super) fn rebuild_retained_bytes(&mut self) {
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
