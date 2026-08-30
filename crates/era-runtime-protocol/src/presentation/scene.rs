use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use super::LogicalLength;
use crate::{InteractionToken, ProtocolValue};

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SceneOffsetV1 {
    #[n(0)]
    pub x: LogicalLength,
    #[n(1)]
    pub y: LogicalLength,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SceneSizeV1 {
    #[n(0)]
    pub width: LogicalLength,
    #[n(1)]
    pub height: LogicalLength,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SceneSourceV1 {
    #[n(0)]
    Resource {
        #[n(0)]
        resource_id: String,
        #[n(1)]
        resource_revision: u64,
    },
    #[n(1)]
    Sprite {
        #[n(0)]
        sprite_name: String,
        #[n(1)]
        resource_revision: u64,
    },
    #[n(2)]
    Canvas {
        #[n(0)]
        canvas_id: i64,
        #[n(1)]
        resource_revision: u64,
    },
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SceneAnchorV1 {
    #[n(0)]
    Viewport,
    #[n(1)]
    DisplayLine {
        #[n(0)]
        line_id: u64,
    },
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum SceneScrollPolicyV1 {
    /// Keep the layer in its anchor's coordinate space while the viewport scrolls.
    #[n(0)]
    Fixed,
    /// Apply the canonical document scroll offset before frontend projection.
    #[n(1)]
    FollowContent,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SceneInteractionV1 {
    #[n(0)]
    pub token: InteractionToken,
    #[n(1)]
    pub value: ProtocolValue,
    #[n(2)]
    pub enabled: bool,
    /// Optional source rendered while the pointer selects this interaction.
    #[n(3)]
    pub hover_source: Option<SceneSourceV1>,
    /// Optional canvas whose pixel values select one of the sibling interactions.
    #[n(4)]
    pub hit_map: Option<SceneSourceV1>,
    /// Optional hover text supplied by CBG button APIs.
    #[n(5)]
    pub title: Option<String>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SceneLayerV1 {
    #[n(0)]
    pub layer_id: u64,
    /// Stable insertion order. Updates retain the original sequence.
    #[n(1)]
    pub sequence: u64,
    #[n(2)]
    pub source: SceneSourceV1,
    #[n(3)]
    pub depth: i64,
    #[n(4)]
    pub anchor: SceneAnchorV1,
    #[n(5)]
    pub offset: SceneOffsetV1,
    #[n(6)]
    pub size: SceneSizeV1,
    #[n(7)]
    pub opacity: u8,
    /// Row-major 5x5 values in 1/256 fixed-point units.
    #[n(8)]
    pub color_matrix: Option<[i64; 25]>,
    #[n(9)]
    pub scroll_policy: SceneScrollPolicyV1,
    #[n(10)]
    pub interaction: Option<SceneInteractionV1>,
    #[n(11)]
    pub scene_revision: u64,
}

#[derive(Clone, Debug, Default, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SceneStateV1 {
    #[n(0)]
    pub revision: u64,
    /// Reference visual order: descending depth, then stable insertion sequence.
    #[n(1)]
    pub layers: Vec<SceneLayerV1>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SceneOperationV1 {
    #[n(0)]
    UpsertLayer {
        #[n(0)]
        layer: Box<SceneLayerV1>,
    },
    #[n(1)]
    RemoveLayer {
        #[n(0)]
        layer_id: u64,
    },
    #[n(2)]
    ClearDepth {
        #[n(0)]
        depth: i64,
    },
    #[n(3)]
    ClearAnchoredLine {
        #[n(0)]
        line_id: u64,
    },
    #[n(4)]
    ReplaceScene {
        #[n(0)]
        scene: SceneStateV1,
    },
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SceneDeltaV1 {
    #[n(0)]
    pub base_revision: u64,
    #[n(1)]
    pub new_revision: u64,
    #[n(2)]
    pub operations: Vec<SceneOperationV1>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SceneReplayError {
    RevisionMismatch,
    NonMonotonicRevision,
    InvalidReplacementRevision,
    DuplicateLayerId,
    DuplicateSequence,
    NonMonotonicSequence,
    ChangedSequence,
    InvalidLayerRevision,
}

impl SceneStateV1 {
    /// Apply one revision-bound delta atomically.
    ///
    /// Frontends can use the same implementation for live delivery and reconnect
    /// replay; invalid deltas leave the previous scene untouched.
    ///
    /// # Errors
    ///
    /// Returns a `SceneReplayError` when revisions, stable layer identity,
    /// insertion order, or replacement contents violate the replay contract.
    pub fn apply_delta(&mut self, delta: &SceneDeltaV1) -> Result<(), SceneReplayError> {
        if self.revision != delta.base_revision {
            return Err(SceneReplayError::RevisionMismatch);
        }
        if delta.new_revision <= delta.base_revision {
            return Err(SceneReplayError::NonMonotonicRevision);
        }
        let mut candidate = self.clone();
        for operation in &delta.operations {
            candidate.apply_operation(operation, delta.new_revision)?;
        }
        candidate.revision = delta.new_revision;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    fn apply_operation(
        &mut self,
        operation: &SceneOperationV1,
        new_revision: u64,
    ) -> Result<(), SceneReplayError> {
        match operation {
            SceneOperationV1::UpsertLayer { layer } => {
                let layer = layer.as_ref().clone();
                if let Some(current) = self
                    .layers
                    .iter_mut()
                    .find(|current| current.layer_id == layer.layer_id)
                {
                    if current.sequence != layer.sequence {
                        return Err(SceneReplayError::ChangedSequence);
                    }
                    if layer.scene_revision < current.scene_revision {
                        return Err(SceneReplayError::InvalidLayerRevision);
                    }
                    *current = layer;
                } else {
                    if self
                        .layers
                        .iter()
                        .map(|current| current.sequence)
                        .max()
                        .is_some_and(|sequence| sequence > layer.sequence)
                    {
                        return Err(SceneReplayError::NonMonotonicSequence);
                    }
                    self.layers.push(layer);
                }
            }
            SceneOperationV1::RemoveLayer { layer_id } => {
                self.layers.retain(|layer| layer.layer_id != *layer_id);
            }
            SceneOperationV1::ClearDepth { depth } => {
                self.layers.retain(|layer| layer.depth != *depth);
            }
            SceneOperationV1::ClearAnchoredLine { line_id } => {
                self.layers.retain(|layer| {
                    !matches!(
                        layer.anchor,
                        SceneAnchorV1::DisplayLine {
                            line_id: anchored
                        } if anchored == *line_id
                    )
                });
            }
            SceneOperationV1::ReplaceScene { scene } => {
                if scene.revision != new_revision {
                    return Err(SceneReplayError::InvalidReplacementRevision);
                }
                *self = scene.clone();
            }
        }
        Ok(())
    }

    fn validate(&mut self) -> Result<(), SceneReplayError> {
        self.layers
            .sort_by_key(|layer| (std::cmp::Reverse(layer.depth), layer.sequence));
        let mut layer_ids = BTreeSet::new();
        if self
            .layers
            .iter()
            .any(|layer| !layer_ids.insert(layer.layer_id))
        {
            return Err(SceneReplayError::DuplicateLayerId);
        }
        let mut sequences = self
            .layers
            .iter()
            .map(|layer| layer.sequence)
            .collect::<Vec<_>>();
        sequences.sort_unstable();
        if sequences.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(SceneReplayError::DuplicateSequence);
        }
        if self
            .layers
            .iter()
            .any(|layer| layer.scene_revision > self.revision)
        {
            return Err(SceneReplayError::InvalidLayerRevision);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(layer_id: u64, sequence: u64, depth: i64, anchor: SceneAnchorV1) -> SceneLayerV1 {
        SceneLayerV1 {
            layer_id,
            sequence,
            source: SceneSourceV1::Resource {
                resource_id: format!("resource-{layer_id}"),
                resource_revision: 1,
            },
            depth,
            anchor,
            offset: SceneOffsetV1 {
                x: LogicalLength(0),
                y: LogicalLength(0),
            },
            size: SceneSizeV1 {
                width: LogicalLength(1_000),
                height: LogicalLength(1_000),
            },
            opacity: 255,
            color_matrix: None,
            scroll_policy: SceneScrollPolicyV1::Fixed,
            interaction: None,
            scene_revision: 1,
        }
    }

    #[test]
    fn every_scene_operation_replays_to_the_authoritative_snapshot() {
        let line = SceneAnchorV1::DisplayLine { line_id: 9 };
        let mut state = SceneStateV1::default();
        state
            .apply_delta(&SceneDeltaV1 {
                base_revision: 0,
                new_revision: 1,
                operations: vec![
                    SceneOperationV1::UpsertLayer {
                        layer: Box::new(layer(1, 1, 3, SceneAnchorV1::Viewport)),
                    },
                    SceneOperationV1::UpsertLayer {
                        layer: Box::new(layer(2, 2, 3, line)),
                    },
                    SceneOperationV1::UpsertLayer {
                        layer: Box::new(layer(3, 3, 1, line)),
                    },
                ],
            })
            .unwrap();
        assert_eq!(
            state
                .layers
                .iter()
                .map(|layer| layer.layer_id)
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );

        let mut updated = layer(1, 1, 5, SceneAnchorV1::Viewport);
        updated.scene_revision = 2;
        state
            .apply_delta(&SceneDeltaV1 {
                base_revision: 1,
                new_revision: 2,
                operations: vec![
                    SceneOperationV1::UpsertLayer {
                        layer: Box::new(updated),
                    },
                    SceneOperationV1::ClearAnchoredLine { line_id: 9 },
                    SceneOperationV1::RemoveLayer { layer_id: 99 },
                ],
            })
            .unwrap();
        assert_eq!(state.layers.len(), 1);
        assert_eq!(state.layers[0].depth, 5);

        state
            .apply_delta(&SceneDeltaV1 {
                base_revision: 2,
                new_revision: 3,
                operations: vec![SceneOperationV1::ClearDepth { depth: 5 }],
            })
            .unwrap();
        assert!(state.layers.is_empty());

        let replacement = SceneStateV1 {
            revision: 4,
            layers: vec![layer(4, 4, 0, SceneAnchorV1::Viewport)],
        };
        state
            .apply_delta(&SceneDeltaV1 {
                base_revision: 3,
                new_revision: 4,
                operations: vec![SceneOperationV1::ReplaceScene {
                    scene: replacement.clone(),
                }],
            })
            .unwrap();
        assert_eq!(state, replacement);
    }

    #[test]
    fn invalid_scene_delta_is_atomic() {
        let mut state = SceneStateV1::default();
        let before = state.clone();
        let duplicate = layer(2, 1, 0, SceneAnchorV1::Viewport);
        assert_eq!(
            state.apply_delta(&SceneDeltaV1 {
                base_revision: 0,
                new_revision: 1,
                operations: vec![
                    SceneOperationV1::UpsertLayer {
                        layer: Box::new(layer(1, 1, 0, SceneAnchorV1::Viewport)),
                    },
                    SceneOperationV1::UpsertLayer {
                        layer: Box::new(duplicate),
                    },
                ],
            }),
            Err(SceneReplayError::DuplicateSequence)
        );
        assert_eq!(state, before);

        state
            .apply_delta(&SceneDeltaV1 {
                base_revision: 0,
                new_revision: 1,
                operations: vec![SceneOperationV1::UpsertLayer {
                    layer: Box::new(layer(1, 2, 0, SceneAnchorV1::Viewport)),
                }],
            })
            .unwrap();
        let before = state.clone();
        assert_eq!(
            state.apply_delta(&SceneDeltaV1 {
                base_revision: 1,
                new_revision: 2,
                operations: vec![SceneOperationV1::UpsertLayer {
                    layer: Box::new(layer(2, 1, 0, SceneAnchorV1::Viewport)),
                }],
            }),
            Err(SceneReplayError::NonMonotonicSequence)
        );
        assert_eq!(state, before);

        let mut rollback = layer(1, 2, 1, SceneAnchorV1::Viewport);
        rollback.scene_revision = 0;
        assert_eq!(
            state.apply_delta(&SceneDeltaV1 {
                base_revision: 1,
                new_revision: 2,
                operations: vec![SceneOperationV1::UpsertLayer {
                    layer: Box::new(rollback),
                }],
            }),
            Err(SceneReplayError::InvalidLayerRevision)
        );
        assert_eq!(state, before);
    }
}
