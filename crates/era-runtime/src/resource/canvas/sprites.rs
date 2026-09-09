impl ResourceGraph {
    fn ensure_live_canvas_sprite_index(&mut self) {
        if self.live_canvas_sprites.is_some() {
            return;
        }
        let mut index =
            std::collections::BTreeMap::<i64, std::collections::BTreeSet<String>>::new();
        for (name, sprite) in &self.sprites {
            for id in sprite
                .canvas_id
                .into_iter()
                .chain(sprite.frames.iter().filter_map(|frame| frame.canvas_id))
            {
                index.entry(id).or_default().insert(name.clone());
            }
        }
        self.live_canvas_sprites = Some(index);
    }

    fn index_live_canvas_sprite(&mut self, name: &str, canvas_id: i64) {
        if let Some(index) = &mut self.live_canvas_sprites {
            index.entry(canvas_id).or_default().insert(name.to_owned());
        }
    }

    fn unindex_live_canvas_sprite(&mut self, name: &str) {
        let Some(index) = &mut self.live_canvas_sprites else {
            return;
        };
        let Some(sprite) = self.sprites.get(name) else {
            return;
        };
        for id in sprite
            .canvas_id
            .into_iter()
            .chain(sprite.frames.iter().filter_map(|frame| frame.canvas_id))
        {
            if let Some(names) = index.get_mut(&id) {
                names.remove(name);
                if names.is_empty() {
                    index.remove(&id);
                }
            }
        }
    }

    fn canvas_has_live_sprites(&mut self, id: i64) -> bool {
        self.ensure_live_canvas_sprite_index();
        self.live_canvas_sprites
            .as_ref()
            .is_some_and(|index| index.contains_key(&id))
    }

    pub(crate) fn has_stale_live_canvas_sprites(&mut self) -> bool {
        self.ensure_live_canvas_sprite_index();
        self.live_canvas_sprites
            .as_ref()
            .into_iter()
            .flat_map(|index| index.iter())
            .any(|(id, names)| {
                self.canvases.get(id).is_some_and(|canvas| {
                    names.iter().any(|name| {
                        let sprite = &self.sprites[name];
                        (sprite.canvas_id == Some(*id)
                            && sprite.canvas_revision != Some(canvas.revision))
                            || sprite.frames.iter().any(|frame| {
                                frame.canvas_id == Some(*id)
                                    && frame.canvas_revision != Some(canvas.revision)
                            })
                    })
                })
            })
    }

    /// `SpriteG` and animation frames refer to a live `GraphicsImage` in both reference engines.
    /// Only current definitions follow its contents; retained draw/scene edges stay exact.
    /// A known mutation visits aliases of one canvas, without scanning the static manifest.
    pub(crate) fn refresh_live_canvas_sprites(&mut self, canvas_id: Option<i64>) -> bool {
        self.ensure_live_canvas_sprite_index();
        let index = self
            .live_canvas_sprites
            .as_ref()
            .expect("index was prepared");
        let names = match canvas_id {
            Some(id) => index.get(&id).cloned().unwrap_or_default(),
            None => index.values().flatten().cloned().collect(),
        };
        let mut changed = false;
        for name in names {
            let sprite = &self.sprites[&name];
            let stale = |id: Option<i64>, revision: Option<u64>| {
                id.and_then(|id| self.canvases.get(&id))
                    .is_some_and(|canvas| revision != Some(canvas.revision))
            };
            if !stale(sprite.canvas_id, sprite.canvas_revision)
                && !sprite
                    .frames
                    .iter()
                    .any(|frame| stale(frame.canvas_id, frame.canvas_revision))
            {
                continue;
            }
            let revision = self.allocate_sprite_revision();
            let sprite = self
                .sprites
                .get_mut(&name)
                .expect("live sprite was checked");
            if let Some(canvas) = sprite.canvas_id.and_then(|id| self.canvases.get(&id)) {
                sprite.canvas_revision = Some(canvas.revision);
            }
            for frame in &mut sprite.frames {
                if let Some(canvas) = frame.canvas_id.and_then(|id| self.canvases.get(&id)) {
                    frame.canvas_revision = Some(canvas.revision);
                }
            }
            sprite.revision = revision;
            changed = true;
        }
        changed
    }

    // Recreating an ID must not reuse an identity held by an immutable drawing edge.
    fn canvas_creation_revision(&self, id: i64, initial: u64) -> u64 {
        self.exact_revisions
            .canvases
            .get(&id)
            .and_then(|revisions| revisions.last_key_value())
            .map_or(initial, |(revision, _)| {
                revision.saturating_add(1).max(initial)
            })
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
        let canvas_revision = canvas.revision;
        let source = era_runtime_protocol::SceneSourceV1::Canvas {
            canvas_id,
            resource_revision: canvas_revision,
        };
        let Ok((exact_revisions, _)) = self.collect_replay_closure(&[source], false) else {
            return false;
        };
        let revision = self.allocate_sprite_revision();
        self.sprites.insert(
            key.clone(),
            SpriteDefinition {
                name: key.clone(),
                revision,
                width: size[0],
                height: size[1],
                frames: Vec::new(),
                dynamic: true,
                position_x: position[0],
                position_y: position[1],
                canvas_id: Some(canvas_id),
                canvas_revision: Some(canvas_revision),
                canvas_rectangle: Some(rectangle),
            },
        );
        self.exact_revisions = exact_revisions;
        self.index_live_canvas_sprite(&key, canvas_id);
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
                canvas_revision: None,
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
        let canvas_revision = canvas.revision;
        let source = era_runtime_protocol::SceneSourceV1::Canvas {
            canvas_id,
            resource_revision: canvas_revision,
        };
        let Ok((exact_revisions, _)) = self.collect_replay_closure(&[source], false) else {
            return false;
        };
        let revision = self.allocate_sprite_revision();
        let sprite = self.sprites.get_mut(&key).expect("sprite was checked");
        sprite.frames.push(SpriteFrame {
            image_path: String::new(),
            content_digest: None,
            canvas_id: Some(canvas_id),
            canvas_revision: Some(canvas_revision),
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
        self.exact_revisions = exact_revisions;
        self.index_live_canvas_sprite(&key, canvas_id);
        true
    }

    pub(crate) fn dispose_sprite(&mut self, name: &str) -> bool {
        let key = name.to_ascii_uppercase();
        if self.sprites.get(&key).is_none_or(|sprite| !sprite.dynamic) {
            return false;
        }
        self.unindex_live_canvas_sprite(&key);
        self.sprites.remove(&key).is_some()
    }

    pub(crate) fn dispose_sprites(&mut self, include_static: bool) -> usize {
        let before = self.sprites.len();
        self.live_canvas_sprites = Some(std::collections::BTreeMap::new());
        self.sprites
            .retain(|_, sprite| !sprite.dynamic && !include_static);
        before.saturating_sub(self.sprites.len())
    }

    /// Preserve game-created replay objects while replacing submitted static resources.
    pub(crate) fn inherit_runtime_graph(
        &mut self,
        previous: &Self,
        roots: &[era_runtime_protocol::SceneSourceV1],
    ) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate.next_sprite_revision = candidate
            .next_sprite_revision
            .max(previous.next_sprite_revision);
        candidate.canvases.clone_from(&previous.canvases);
        candidate
            .exact_revisions
            .clone_from(&previous.exact_revisions);
        candidate.retained_canvas_command_bytes = previous.retained_canvas_command_bytes;
        candidate.animation_timer_ms = previous.animation_timer_ms;
        let mut inherited_metadata = Vec::new();
        for image in candidate.images.values_mut() {
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
            let _ = candidate.validate_image_frames(&path);
        }
        for (name, sprite) in &previous.sprites {
            let resources_still_match = sprite.frames.iter().all(|frame| {
                frame.content_digest.map_or_else(
                    || frame.image_path.is_empty(),
                    |digest| {
                        candidate
                            .images
                            .get(&frame.image_path.to_ascii_lowercase())
                            .is_some_and(|image| image.digest == digest)
                    },
                )
            });
            if sprite.dynamic && resources_still_match {
                candidate.sprites.insert(name.clone(), sprite.clone());
            }
        }
        candidate.live_canvas_sprites = None;
        candidate.ensure_canvas_retained_bytes();
        let _ = candidate.replay_for_roots(roots)?;
        if candidate.total_canvas_bytes_with(&candidate.exact_revisions)
            > MAXIMUM_CANVAS_COMMAND_BYTES
        {
            return Err("hot reload exact replay closure exceeds canvas command budget".into());
        }
        *self = candidate;
        Ok(())
    }

    pub(crate) fn reset_runtime_graph(&mut self) {
        self.canvases = std::collections::BTreeMap::default();
        self.live_canvas_sprites = Some(std::collections::BTreeMap::new());
        self.exact_revisions = ExactRevisionStore::default();
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

    pub(super) fn ensure_canvas_retained_bytes(&mut self) {
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
        self.exact_revisions.rebuild_retained_bytes();
    }

    fn preflight_exact_command(
        &mut self,
        target_id: i64,
        roots: &[era_runtime_protocol::SceneSourceV1],
        command: &CanvasCommand,
    ) -> Option<ExactRevisionStore> {
        self.ensure_canvas_retained_bytes();
        let target = self.canvases.get(&target_id)?;
        let (candidate, _) = self.collect_replay_closure(roots, false).ok()?;
        let target_becomes_historical = candidate
            .canvases
            .get(&target_id)
            .is_some_and(|revisions| revisions.contains_key(&target.revision));
        let additional_history = if target_becomes_historical {
            target.retained_command_bytes
        } else {
            0
        };
        let total = self
            .total_canvas_bytes_with(&candidate)
            .saturating_add(additional_history)
            .saturating_add(command.retained_bytes());
        (total <= MAXIMUM_CANVAS_COMMAND_BYTES).then_some(candidate)
    }

    fn push_canvas_command(&mut self, id: i64, command: CanvasCommand) -> bool {
        self.ensure_canvas_retained_bytes();
        let Some(target) = self.canvases.get(&id) else {
            return false;
        };
        let target_becomes_historical = self
            .exact_revisions
            .canvases
            .get(&id)
            .is_some_and(|revisions| revisions.contains_key(&target.revision));
        let historical_retained = if target_becomes_historical {
            target.retained_command_bytes
        } else {
            0
        };
        let next_total = self
            .total_canvas_bytes_with(&self.exact_revisions)
            .saturating_add(historical_retained)
            .saturating_add(command.retained_bytes());
        if next_total > MAXIMUM_CANVAS_COMMAND_BYTES {
            return false;
        }
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
