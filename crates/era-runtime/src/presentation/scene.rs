use super::PresentationModel;
use super::model::{CbgLayerIndex, ImageLayerIndex};
use era_runtime_protocol::{
    InteractionToken, LogicalLength, ProtocolValue, SceneAnchorV1, SceneDeltaV1,
    SceneInteractionV1, SceneLayerV1, SceneOffsetV1, SceneOperationV1, SceneScrollPolicyV1,
    SceneSizeV1, SceneSourceV1, SceneStateV1,
};

impl PresentationModel {
    /// All committed and pending sources that keep an exact resource revision live.
    pub(crate) fn resource_roots(&self) -> Vec<SceneSourceV1> {
        let mut roots = Vec::new();
        for layer in &self.scene.layers {
            roots.push(layer.source.clone());
            if let Some(interaction) = &layer.interaction {
                roots.extend(interaction.hover_source.iter().cloned());
                roots.extend(interaction.hit_map.iter().cloned());
            }
        }
        roots.extend(self.cbg_button_map.iter().cloned());
        roots
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
                document_origin_y: LogicalLength(0),
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
                document_origin_y: LogicalLength(0),
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
        let revision = self.revision;
        self.refresh_client_background_interactions();
        self.resource_replay_stale = true;
        if self.revision == revision {
            self.bump();
        }
        true
    }

    pub(crate) fn clear_client_background_button_map(&mut self) {
        if self.cbg_button_map.is_none() {
            return;
        }
        self.cbg_button_map = None;
        let revision = self.revision;
        self.refresh_client_background_interactions();
        self.resource_replay_stale = true;
        if self.revision == revision {
            self.bump();
        }
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
                document_origin_y: match (anchor, follow_content) {
                    (SceneAnchorV1::Viewport, true) => self.canonical_document_cursor_y,
                    _ => LogicalLength(0),
                },
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
        // Scene edges are roots of the exact resource-revision closure. Re-materialize
        // resources even when no canvas or sprite mutated so removed roots release history.
        self.resource_replay_stale = true;
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
