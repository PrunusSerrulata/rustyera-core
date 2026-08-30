use super::model::{CbgLayerIndex, ImageLayerIndex};
use super::{PresentationModel, rgb_color};
use era_runtime_protocol::{
    AudioState, InteractionToken, LogicalLength, ProtocolValue, ResourceReplay, SceneAnchorV1,
    SceneDeltaV1, SceneInteractionV1, SceneLayerV1, SceneOffsetV1, SceneOperationV1,
    SceneScrollPolicyV1, SceneSizeV1, SceneSourceV1, SceneStateV1, TooltipFormat,
};

impl PresentationModel {
    pub(crate) fn play_bgm(&mut self, resource_id: String) {
        self.audio.clear();
        self.audio.push(AudioState {
            channel_id: 1,
            resource_id,
            repeat_count: -1,
            volume_millionths: 1_000_000,
            playing: true,
            revision: self.revision.saturating_add(1),
        });
        self.delivery.dirty.audio = true;
        self.bump();
    }

    pub(crate) fn stop_bgm(&mut self) {
        self.audio.clear();
        self.delivery.dirty.audio = true;
        self.bump();
    }

    pub(crate) fn set_bgm_volume(&mut self, volume: i64) {
        let volume = volume.clamp(0, 100);
        for state in &mut self.audio {
            state.volume_millionths = u32::try_from(volume).unwrap_or_default() * 10_000;
            state.revision = self.revision.saturating_add(1);
        }
        self.delivery.dirty.audio = true;
        self.bump();
    }

    pub(crate) fn projects_audio(&self) -> bool {
        self.project_audio
    }

    pub(crate) fn set_resource_replay(&mut self, resources: ResourceReplay) {
        self.resources = resources;
        self.resource_replay_stale = false;
        self.delivery.dirty.resources = true;
        self.bump();
    }

    pub(crate) fn set_animation_timer(&mut self, milliseconds: i32) {
        if self.resources.animation_timer_ms == milliseconds {
            return;
        }
        self.resources.animation_timer_ms = milliseconds;
        self.delivery.dirty.resources = true;
        self.bump();
    }

    pub(crate) fn mark_resource_replay_stale(&mut self) {
        self.resource_replay_stale = true;
    }

    pub(crate) const fn resource_replay_stale(&self) -> bool {
        self.resource_replay_stale
    }

    pub(crate) fn resource_replay_is_ready_to_publish(&self) -> bool {
        self.resource_replay_stale
            && (self.input_wait.is_some() || (self.redraw_enabled && self.delivery.dirty.redraw))
    }

    pub(crate) fn add_background(
        &mut self,
        resource_id: String,
        resource_revision: u64,
        depth: i64,
        opacity: i64,
    ) {
        let layer_id = self.allocate_scene_layer_id();
        let sequence = self.allocate_scene_sequence();
        let operation = SceneOperationV1::UpsertLayer {
            layer: Box::new(SceneLayerV1 {
                layer_id,
                sequence,
                source: SceneSourceV1::Sprite {
                    sprite_name: resource_id.clone(),
                    resource_revision,
                },
                depth,
                anchor: SceneAnchorV1::Viewport,
                offset: SceneOffsetV1 {
                    x: LogicalLength(0),
                    y: LogicalLength(0),
                },
                size: SceneSizeV1 {
                    width: self.settings.drawable_width,
                    height: LogicalLength(0),
                },
                opacity: u8::try_from(opacity).unwrap_or(if opacity < 0 { 0 } else { u8::MAX }),
                color_matrix: None,
                scroll_policy: SceneScrollPolicyV1::Fixed,
                interaction: None,
                scene_revision: self.scene.revision.saturating_add(1),
            }),
        };
        self.apply_scene_operations(vec![operation]);
        self.background_layers.push((resource_id, layer_id));
    }

    pub(crate) fn remove_background(&mut self, resource_id: &str) -> bool {
        let Some(index) = self
            .background_layers
            .iter()
            .position(|(current, _)| current == resource_id)
        else {
            return false;
        };
        let (_, layer_id) = self.background_layers.remove(index);
        self.apply_scene_operations(vec![SceneOperationV1::RemoveLayer { layer_id }]);
        true
    }

    pub(crate) fn clear_backgrounds(&mut self) {
        let operations = self
            .background_layers
            .drain(..)
            .map(|(_, layer_id)| SceneOperationV1::RemoveLayer { layer_id })
            .collect::<Vec<_>>();
        self.apply_scene_operations(operations);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn add_client_background(
        &mut self,
        source: SceneSourceV1,
        depth: i64,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        opacity: u8,
        color_matrix: Option<[i64; 25]>,
        button: Option<(InteractionToken, i64, Option<SceneSourceV1>, Option<String>)>,
    ) {
        let layer_id = self.allocate_scene_layer_id();
        let sequence = self.allocate_scene_sequence();
        let interaction =
            button
                .as_ref()
                .map(|(token, value, hover_source, title)| SceneInteractionV1 {
                    token: *token,
                    value: ProtocolValue::Integer(*value),
                    enabled: true,
                    hover_source: hover_source.clone(),
                    hit_map: self.cbg_button_map.clone(),
                    title: title.clone(),
                });
        self.apply_scene_operations(vec![SceneOperationV1::UpsertLayer {
            layer: Box::new(SceneLayerV1 {
                layer_id,
                sequence,
                source,
                depth,
                anchor: SceneAnchorV1::Viewport,
                offset: scene_offset(x, y),
                size: scene_size(width, height),
                opacity,
                color_matrix,
                scroll_policy: SceneScrollPolicyV1::Fixed,
                interaction,
                scene_revision: self.scene.revision.saturating_add(1),
            }),
        }]);
        self.cbg_layers.push(CbgLayerIndex {
            layer_id,
            depth,
            interaction: button.map(|(token, _, _, _)| token),
        });
    }

    pub(crate) fn clear_client_backgrounds(&mut self) -> Vec<InteractionToken> {
        let tokens = self
            .cbg_layers
            .iter()
            .filter_map(|entry| entry.interaction)
            .collect();
        let operations = self
            .cbg_layers
            .drain(..)
            .map(|entry| SceneOperationV1::RemoveLayer {
                layer_id: entry.layer_id,
            })
            .collect();
        self.cbg_button_map = None;
        self.apply_scene_operations(operations);
        tokens
    }

    pub(crate) fn clear_client_background_range(
        &mut self,
        minimum: i64,
        maximum: i64,
    ) -> Vec<InteractionToken> {
        if minimum > maximum {
            return Vec::new();
        }
        let mut operations = Vec::new();
        let mut tokens = Vec::new();
        self.cbg_layers.retain(|entry| {
            if (minimum..=maximum).contains(&entry.depth) {
                operations.push(SceneOperationV1::RemoveLayer {
                    layer_id: entry.layer_id,
                });
                tokens.extend(entry.interaction);
                false
            } else {
                true
            }
        });
        self.apply_scene_operations(operations);
        tokens
    }

    pub(crate) fn clear_client_background_buttons(&mut self) -> Vec<InteractionToken> {
        let mut operations = Vec::new();
        let mut tokens = Vec::new();
        self.cbg_layers.retain(|entry| {
            if let Some(token) = entry.interaction {
                operations.push(SceneOperationV1::RemoveLayer {
                    layer_id: entry.layer_id,
                });
                tokens.push(token);
                false
            } else {
                true
            }
        });
        self.cbg_button_map = None;
        self.apply_scene_operations(operations);
        tokens
    }

    pub(crate) fn set_client_background_button_map(&mut self, source: SceneSourceV1) -> bool {
        if self.cbg_button_map.as_ref() == Some(&source) {
            return false;
        }
        self.cbg_button_map = Some(source);
        self.refresh_client_background_interactions();
        true
    }

    pub(crate) fn clear_client_background_button_map(&mut self) {
        self.cbg_button_map = None;
        self.refresh_client_background_interactions();
    }

    fn refresh_client_background_interactions(&mut self) {
        let next_revision = self.scene.revision.saturating_add(1);
        let mut operations = Vec::new();
        for entry in self
            .cbg_layers
            .iter()
            .filter(|entry| entry.interaction.is_some())
        {
            let Some(mut layer) = self
                .scene
                .layers
                .iter()
                .find(|layer| layer.layer_id == entry.layer_id)
                .cloned()
            else {
                continue;
            };
            if let Some(interaction) = &mut layer.interaction {
                interaction.hit_map.clone_from(&self.cbg_button_map);
            }
            layer.scene_revision = next_revision;
            operations.push(SceneOperationV1::UpsertLayer {
                layer: Box::new(layer),
            });
        }
        if !operations.is_empty() {
            self.apply_scene_operations(operations);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn add_image_layer(
        &mut self,
        source: SceneSourceV1,
        depth: i64,
        anchor: SceneAnchorV1,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        opacity: u8,
        color_matrix: Option<[i64; 25]>,
        follow_content: bool,
    ) {
        let layer_id = self.allocate_scene_layer_id();
        let sequence = self.allocate_scene_sequence();
        self.apply_scene_operations(vec![SceneOperationV1::UpsertLayer {
            layer: Box::new(SceneLayerV1 {
                layer_id,
                sequence,
                source,
                depth,
                anchor,
                offset: scene_offset(x, y),
                size: scene_size(width, height),
                opacity,
                color_matrix,
                scroll_policy: if follow_content {
                    SceneScrollPolicyV1::FollowContent
                } else {
                    SceneScrollPolicyV1::Fixed
                },
                interaction: None,
                scene_revision: self.scene.revision.saturating_add(1),
            }),
        }]);
        self.image_layers.push(ImageLayerIndex { layer_id, depth });
    }

    pub(crate) fn clear_image_layer(&mut self, depth: i64) {
        let mut operations = Vec::new();
        self.image_layers.retain(|entry| {
            if entry.depth == depth {
                operations.push(SceneOperationV1::RemoveLayer {
                    layer_id: entry.layer_id,
                });
                false
            } else {
                true
            }
        });
        self.apply_scene_operations(operations);
    }

    pub(crate) fn clear_image_layers(&mut self) {
        let operations = self
            .image_layers
            .drain(..)
            .map(|entry| SceneOperationV1::RemoveLayer {
                layer_id: entry.layer_id,
            })
            .collect();
        self.apply_scene_operations(operations);
    }

    pub(crate) fn image_layer_exists(&self, depth: i64) -> bool {
        self.image_layers.iter().any(|entry| {
            entry.depth == depth
                && self
                    .scene
                    .layers
                    .iter()
                    .any(|layer| layer.layer_id == entry.layer_id)
        })
    }

    pub(crate) const fn current_line_id(&self) -> u64 {
        self.next_line
    }

    pub(crate) fn clear_anchored_scene_lines(&mut self, line_ids: &[u64]) {
        if line_ids.is_empty() {
            return;
        }
        let anchored_line_ids = self
            .scene
            .layers
            .iter()
            .filter_map(|layer| match layer.anchor {
                SceneAnchorV1::DisplayLine { line_id } if line_ids.contains(&line_id) => {
                    Some(line_id)
                }
                SceneAnchorV1::Viewport | SceneAnchorV1::DisplayLine { .. } => None,
            })
            .collect::<std::collections::BTreeSet<_>>();
        if anchored_line_ids.is_empty() {
            return;
        }
        let removed_layer_ids = self
            .scene
            .layers
            .iter()
            .filter_map(|layer| match layer.anchor {
                SceneAnchorV1::DisplayLine { line_id } if line_ids.contains(&line_id) => {
                    Some(layer.layer_id)
                }
                SceneAnchorV1::Viewport | SceneAnchorV1::DisplayLine { .. } => None,
            })
            .collect::<Vec<_>>();
        self.image_layers
            .retain(|entry| !removed_layer_ids.contains(&entry.layer_id));
        let operations = anchored_line_ids
            .into_iter()
            .map(|line_id| SceneOperationV1::ClearAnchoredLine { line_id })
            .collect();
        self.apply_scene_operations(operations);
    }

    pub(crate) fn rebind_scene_interactions(
        &mut self,
        tokens: &std::collections::BTreeMap<InteractionToken, InteractionToken>,
    ) {
        for entry in &mut self.cbg_layers {
            if let Some(token) = entry.interaction
                && let Some(rebound) = tokens.get(&token)
            {
                entry.interaction = Some(*rebound);
            }
        }
        let next_revision = self.scene.revision.saturating_add(1);
        let operations = self
            .scene
            .layers
            .iter()
            .filter_map(|layer| {
                let interaction = layer.interaction.as_ref()?;
                let rebound = tokens.get(&interaction.token)?;
                if *rebound == interaction.token {
                    return None;
                }
                let mut layer = layer.clone();
                layer.interaction.as_mut()?.token = *rebound;
                layer.scene_revision = next_revision;
                Some(SceneOperationV1::UpsertLayer {
                    layer: Box::new(layer),
                })
            })
            .collect::<Vec<_>>();
        if !operations.is_empty() {
            self.apply_scene_operations(operations);
        }
    }

    fn apply_scene_operations(&mut self, operations: Vec<SceneOperationV1>) {
        let delta = SceneDeltaV1 {
            base_revision: self.scene.revision,
            new_revision: self.scene.revision.saturating_add(1),
            operations: operations.clone(),
        };
        self.scene
            .apply_delta(&delta)
            .expect("runtime-created scene deltas satisfy the public contract");
        self.scene_operations.extend(operations);
        self.delivery.dirty.scene = true;
        self.bump();
    }

    fn allocate_scene_layer_id(&mut self) -> u64 {
        let layer_id = self.next_scene_layer_id;
        self.next_scene_layer_id = self.next_scene_layer_id.saturating_add(1);
        layer_id
    }

    fn allocate_scene_sequence(&mut self) -> u64 {
        let sequence = self.next_scene_sequence;
        self.next_scene_sequence = self.next_scene_sequence.saturating_add(1);
        sequence
    }

    pub(super) fn projected_scene(&self) -> SceneStateV1 {
        if self.project_graphics {
            self.scene.clone()
        } else {
            SceneStateV1 {
                revision: self.scene.revision,
                layers: Vec::new(),
            }
        }
    }

    pub(super) fn projected_scene_delta(&self, base_revision: u64) -> SceneDeltaV1 {
        SceneDeltaV1 {
            base_revision,
            new_revision: self.scene.revision,
            operations: if self.project_graphics {
                self.scene_operations.clone()
            } else {
                vec![SceneOperationV1::ReplaceScene {
                    scene: self.projected_scene(),
                }]
            },
        }
    }

    pub(crate) fn set_tooltip_colors(&mut self, foreground: i64, background: i64) {
        self.tooltip.foreground = rgb_color(foreground);
        self.tooltip.background = rgb_color(background);
        self.delivery.dirty.tooltip = true;
        self.bump();
    }

    pub(crate) fn set_tooltip_delay(&mut self, delay: i64) -> Result<(), &'static str> {
        self.tooltip.delay_ms =
            u32::try_from(delay).map_err(|_| "tooltip delay is out of range")?;
        self.delivery.dirty.tooltip = true;
        self.bump();
        Ok(())
    }

    pub(crate) fn set_tooltip_duration(&mut self, duration: i64) -> Result<(), &'static str> {
        let duration = u32::try_from(duration).map_err(|_| "tooltip duration is out of range")?;
        self.tooltip.duration_ms = duration.min(i16::MAX.cast_unsigned().into());
        self.delivery.dirty.tooltip = true;
        self.bump();
        Ok(())
    }

    pub(crate) fn set_tooltip_font(&mut self, family: String) {
        self.tooltip.font_family = (!family.is_empty()).then_some(family);
        self.delivery.dirty.tooltip = true;
        self.bump();
    }

    pub(crate) fn set_tooltip_font_size(&mut self, points: i64) -> Result<(), &'static str> {
        let points = u32::try_from(points).map_err(|_| "tooltip font size is out of range")?;
        self.tooltip.font_millipoints = points.saturating_mul(1_000);
        self.delivery.dirty.tooltip = true;
        self.bump();
        Ok(())
    }

    pub(crate) fn set_tooltip_custom(&mut self, enabled: bool) {
        self.tooltip.custom = enabled;
        self.delivery.dirty.tooltip = true;
        self.bump();
    }

    pub(crate) fn set_tooltip_format(&mut self, format: i64) {
        self.tooltip.format = format;
        self.tooltip.normalized_format = TooltipFormat::from_raw(format);
        self.delivery.dirty.tooltip = true;
        self.bump();
    }

    pub(crate) fn set_tooltip_images(&mut self, enabled: bool) {
        self.tooltip.images = enabled;
        self.delivery.dirty.tooltip = true;
        self.bump();
    }

    #[allow(clippy::fn_params_excessive_bools)]
    pub(crate) fn set_projection(
        &mut self,
        column_cells: bool,
        separators: bool,
        html: bool,
        graphics: bool,
        audio: bool,
    ) {
        self.project_column_cells = column_cells;
        self.project_separators = separators;
        self.project_html = html;
        self.project_graphics = graphics;
        self.project_audio = audio;
        self.delivery.dirty.force_snapshot = true;
    }

    /// Clear runtime-owned content without forgetting capabilities negotiated by the client.
    pub(crate) fn reset_preserving_projection(&mut self) {
        let projection = (
            self.project_column_cells,
            self.project_separators,
            self.project_html,
            self.project_graphics,
            self.project_audio,
        );
        *self = Self::default();
        self.set_projection(
            projection.0,
            projection.1,
            projection.2,
            projection.3,
            projection.4,
        );
    }

    pub(crate) fn configure_project(
        &mut self,
        project: &crate::project::NormalizedProjectSnapshot,
    ) {
        use era_config::ConfigValue;

        let integer = |code| match project.client_configuration.get_code(code) {
            Some(ConfigValue::Integer(value)) => Some(*value),
            _ => None,
        };
        let boolean = |code| match project.client_configuration.get_code(code) {
            Some(ConfigValue::Boolean(value)) => Some(*value),
            _ => None,
        };
        let color = |code| match project.client_configuration.get_code(code) {
            Some(ConfigValue::Color(value)) => Some(*value),
            _ => None,
        };
        let font = match project.client_configuration.get_code("FontName") {
            Some(ConfigValue::String(value)) => Some(value.clone()),
            _ => None,
        };
        let character_width_mode = match project.configuration.get_code("CharacterWidthMode") {
            Some(ConfigValue::Enum { value, .. }) => {
                erabasic_vm::CharacterWidthMode::from_config_code(value)
            }
            _ => erabasic_vm::CharacterWidthMode::Automatic,
        };
        let viewport_width = integer("WindowX")
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(project.viewport_width)
            .max(128);
        let viewport_height = integer("WindowY")
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(project.viewport_height)
            .max(128);
        let font_size = integer("FontSize")
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(project.font_size)
            .max(8);
        let line_height = integer("LineHeight")
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(project.line_height)
            .max(font_size);
        self.settings.drawable_width =
            LogicalLength(i64::from(viewport_width).saturating_mul(1_000));
        self.settings.drawable_height =
            LogicalLength(i64::from(viewport_height).saturating_mul(1_000));
        self.settings.line_height = LogicalLength(i64::from(line_height).saturating_mul(1_000));
        self.settings.maximum_physical_lines = integer("MaxLog")
            .and_then(|value| u32::try_from(value).ok())
            .map_or(5_000, |value| value.max(500));
        self.settings.prevent_button_wrap = boolean("ButtonWrap").unwrap_or(false);
        self.settings.legacy_nonbutton_wrap = boolean("CompatiLinefeedAs1739").unwrap_or(false);
        let mut default_style = self.default_style.clone();
        default_style.font_family = font;
        default_style.font_millipixels = font_size.saturating_mul(1_000);
        default_style.foreground = rgb_color(i64::from(color("ForeColor").unwrap_or(0x00c0_c0c0)));
        self.default_background = rgb_color(i64::from(color("BackColor").unwrap_or(0)));
        self.settings.background = self.default_background;
        self.settings.button_focus_foreground =
            rgb_color(i64::from(color("FocusColor").unwrap_or(0x00ff_ff00)));
        self.apply_project_default_style(default_style);
        self.print_c_length = project.print_c_length.max(1);
        self.set_character_width_mode(character_width_mode);
        self.trim_physical_history();
        self.delivery.dirty.settings = true;
        self.bump();
    }
}

fn scene_offset(x: i32, y: i32) -> SceneOffsetV1 {
    SceneOffsetV1 {
        x: LogicalLength(i64::from(x).saturating_mul(1_000)),
        y: LogicalLength(i64::from(y).saturating_mul(1_000)),
    }
}

fn scene_size(width: i32, height: i32) -> SceneSizeV1 {
    SceneSizeV1 {
        width: LogicalLength(i64::from(width.max(0)).saturating_mul(1_000)),
        height: LogicalLength(i64::from(height.max(0)).saturating_mul(1_000)),
    }
}
