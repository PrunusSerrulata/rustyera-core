use std::collections::BTreeSet;

use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use super::{
    AudioState, CanvasPoint, CanvasRect, CanvasSize, PresentationHistory, PresentationSettings,
    RedrawState, SceneStateV1, TooltipSettings,
};
use crate::{InputWait, ProtocolBytes};

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SpriteFrameReplay {
    #[n(0)]
    pub resource_id: String,
    #[n(1)]
    pub source_rectangle: [i32; 4],
    #[n(2)]
    pub offset: [i32; 2],
    #[n(3)]
    pub delay_ms: u32,
    #[n(4)]
    pub destination_size: Option<[u32; 2]>,
    /// Runtime-created animation frames can reference a replay canvas instead of a file resource.
    #[n(5)]
    pub canvas_id: Option<i64>,
    /// Exact immutable identity of a project resource. Canvas-backed frames have no digest.
    #[n(6)]
    pub content_digest: Option<ProtocolBytes>,
    /// Exact canvas revision used by a canvas-backed frame.
    #[n(7)]
    pub canvas_revision: Option<u64>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SpriteReplay {
    #[n(0)]
    pub name: String,
    #[n(1)]
    pub size: [u32; 2],
    #[n(2)]
    pub position: [i32; 2],
    #[n(3)]
    pub frames: Vec<SpriteFrameReplay>,
    #[n(4)]
    pub canvas_id: Option<i64>,
    #[n(5)]
    pub canvas_rectangle: Option<CanvasRect>,
    /// Monotonic identity of the exact sprite definition referenced by a scene layer.
    #[n(6)]
    pub revision: u64,
    /// Exact canvas revision used by a canvas-backed sprite.
    #[n(7)]
    pub canvas_revision: Option<u64>,
    /// True only for the current name alias; false for immutable historical definitions.
    /// Missing on older replays, whose names are usable only when unambiguous.
    #[n(8)]
    #[serde(default)]
    pub current_alias: Option<bool>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CanvasReplayCommand {
    #[n(0)]
    Clear {
        #[n(0)]
        argb: u32,
        #[n(1)]
        rectangle: Option<CanvasRect>,
    },
    #[n(1)]
    DrawSprite {
        #[n(0)]
        name: String,
        #[n(1)]
        destination: CanvasRect,
        #[n(2)]
        color_matrix: Option<Vec<i64>>,
        #[n(3)]
        resource_revision: u64,
    },
    #[n(2)]
    SetPixel {
        #[n(0)]
        point: CanvasPoint,
        #[n(1)]
        argb: u32,
    },
    #[n(3)]
    FillRectangle {
        #[n(0)]
        rectangle: CanvasRect,
        #[n(1)]
        brush_argb: u32,
    },
    #[n(4)]
    SetBrush {
        #[n(0)]
        argb: u32,
    },
    #[n(5)]
    SetPen {
        #[n(0)]
        argb: u32,
        #[n(1)]
        width: i64,
    },
    #[n(6)]
    SetDashStyle {
        #[n(0)]
        style: i64,
        #[n(1)]
        cap: i64,
    },
    #[n(7)]
    SetFont {
        #[n(0)]
        family: String,
        #[n(1)]
        size: i64,
        #[n(2)]
        style_bits: u8,
    },
    #[n(8)]
    DrawLine {
        #[n(0)]
        start: CanvasPoint,
        #[n(1)]
        end: CanvasPoint,
    },
    #[n(9)]
    DrawText {
        #[n(0)]
        text: String,
        #[n(1)]
        point: CanvasPoint,
    },
    #[n(10)]
    DrawCanvas {
        #[n(0)]
        source_canvas_id: i64,
        #[n(1)]
        source_revision: u64,
        #[n(2)]
        source: CanvasRect,
        #[n(3)]
        destination: CanvasRect,
        /// 5x5 matrix values in reference 1/256 fixed-point units.
        #[n(4)]
        color_matrix: Option<Vec<i64>>,
        #[n(5)]
        mask_canvas_id: Option<i64>,
        #[n(6)]
        rotation_millidegrees: i64,
        #[n(7)]
        rotation_center: Option<CanvasPoint>,
        #[n(8)]
        mask_revision: Option<u64>,
    },
    #[n(11)]
    LoadEncodedImage {
        #[n(0)]
        content_digest: ProtocolBytes,
        #[n(1)]
        encoded: ProtocolBytes,
    },
    #[n(12)]
    PolygonPointAdd {
        #[n(0)]
        point: CanvasPoint,
    },
    #[n(13)]
    PolygonPointClear,
    #[n(14)]
    DrawPolygon,
    #[n(15)]
    FillPolygon,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct CanvasReplay {
    #[n(0)]
    pub canvas_id: i64,
    #[n(1)]
    pub size: CanvasSize,
    #[n(2)]
    pub commands: Vec<CanvasReplayCommand>,
    #[n(3)]
    pub revision: u64,
}

#[derive(Clone, Debug, Default, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ResourceReplay {
    #[n(0)]
    pub sprites: Vec<SpriteReplay>,
    #[n(1)]
    pub canvases: Vec<CanvasReplay>,
    /// Canonical redraw cadence selected by SETANIMETIMER. Frontends schedule rendering from
    /// this value but never advance game time or choose animation frames for the runtime.
    #[n(2)]
    pub animation_timer_ms: i32,
}

/// One nonoverlapping edit in the original resource-list baseline's coordinates.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ResourceReplayListEdit<T> {
    #[n(0)]
    pub start: u32,
    #[n(1)]
    pub delete_count: u32,
    #[n(2)]
    pub insert: Vec<T>,
}

/// Lossless edits to the resource lists at the enclosing presentation base revision.
/// Unchanged entries retain their exact identity and revision; snapshots remain full baselines.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ResourceReplayDelta {
    #[n(0)]
    pub sprite_edits: Vec<ResourceReplayListEdit<SpriteReplay>>,
    #[n(1)]
    pub canvas_edits: Vec<ResourceReplayListEdit<CanvasReplay>>,
    #[n(2)]
    pub animation_timer_ms: i32,
}

impl ResourceReplay {
    /// Reconstruct a resource delta atomically from its enclosing presentation baseline.
    ///
    /// # Errors
    ///
    /// Rejects overlapping/out-of-bounds edits and invalid exact resource dependencies.
    pub fn apply_delta(&self, delta: &ResourceReplayDelta) -> Result<Self, String> {
        let next = Self {
            sprites: apply_resource_list_edits(&self.sprites, &delta.sprite_edits)?,
            canvases: apply_resource_list_edits(&self.canvases, &delta.canvas_edits)?,
            animation_timer_ms: delta.animation_timer_ms,
        };
        next.validate_exact_references()?;
        Ok(next)
    }

    /// Validate every exact mutable-resource edge as an atomic identity/revision pair.
    ///
    /// # Errors
    ///
    /// Returns an error when the replay contains duplicate exact identities/current aliases, an incomplete
    /// canvas identity/revision pair, or a reference whose exact definition is absent.
    pub fn validate_exact_references(&self) -> Result<(), String> {
        let mut sprites = BTreeSet::new();
        let mut current_aliases = BTreeSet::new();
        let mut canvases = BTreeSet::new();
        for sprite in &self.sprites {
            let key = (sprite.name.to_ascii_uppercase(), sprite.revision);
            if !sprites.insert(key) {
                return Err("duplicate exact sprite identity in resource replay".into());
            }
            if sprite.current_alias == Some(true)
                && !current_aliases.insert(sprite.name.to_ascii_uppercase())
            {
                return Err("duplicate current sprite alias in resource replay".into());
            }
            validate_optional_canvas_pair(sprite.canvas_id, sprite.canvas_revision, "sprite")?;
            for frame in &sprite.frames {
                validate_optional_canvas_pair(
                    frame.canvas_id,
                    frame.canvas_revision,
                    "sprite frame",
                )?;
            }
        }
        for canvas in &self.canvases {
            if !canvases.insert((canvas.canvas_id, canvas.revision)) {
                return Err("duplicate exact canvas identity in resource replay".into());
            }
        }
        for sprite in &self.sprites {
            for edge in sprite
                .canvas_id
                .zip(sprite.canvas_revision)
                .into_iter()
                .chain(
                    sprite
                        .frames
                        .iter()
                        .filter_map(|frame| frame.canvas_id.zip(frame.canvas_revision)),
                )
            {
                if !canvases.contains(&edge) {
                    return Err(format!("missing exact canvas {}@{}", edge.0, edge.1));
                }
            }
        }
        for canvas in &self.canvases {
            for command in &canvas.commands {
                match command {
                    CanvasReplayCommand::DrawSprite {
                        name,
                        resource_revision,
                        ..
                    } => {
                        let edge = (name.to_ascii_uppercase(), *resource_revision);
                        if !sprites.contains(&edge) {
                            return Err(format!("missing exact sprite {}@{}", edge.0, edge.1));
                        }
                    }
                    CanvasReplayCommand::DrawCanvas {
                        source_canvas_id,
                        source_revision,
                        mask_canvas_id,
                        mask_revision,
                        ..
                    } => {
                        validate_optional_canvas_pair(
                            *mask_canvas_id,
                            *mask_revision,
                            "canvas mask",
                        )?;
                        let source = (*source_canvas_id, *source_revision);
                        if !canvases.contains(&source) {
                            return Err(format!("missing exact canvas {}@{}", source.0, source.1));
                        }
                        if let Some(mask) = (*mask_canvas_id).zip(*mask_revision)
                            && !canvases.contains(&mask)
                        {
                            return Err(format!("missing exact canvas {}@{}", mask.0, mask.1));
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

fn apply_resource_list_edits<T: Clone>(
    baseline: &[T],
    edits: &[ResourceReplayListEdit<T>],
) -> Result<Vec<T>, String> {
    let mut output = Vec::new();
    let mut cursor = 0;
    let mut previous_start = None;
    for edit in edits {
        let start = edit.start as usize;
        let count = edit.delete_count as usize;
        if start < cursor
            || previous_start.is_some_and(|previous| start <= previous)
            || start > baseline.len()
            || count > baseline.len() - start
            || (count == 0 && edit.insert.is_empty())
        {
            return Err("resource list edit is out of bounds or overlapping".into());
        }
        output.extend_from_slice(&baseline[cursor..start]);
        output.extend_from_slice(&edit.insert);
        cursor = start + count;
        previous_start = Some(start);
    }
    output.extend_from_slice(&baseline[cursor..]);
    Ok(output)
}

fn validate_optional_canvas_pair(
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

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct PresentationSnapshot {
    #[n(0)]
    pub revision: u64,
    #[n(1)]
    pub title: String,
    #[n(2)]
    pub history: PresentationHistory,
    #[n(3)]
    pub scene: SceneStateV1,
    #[n(4)]
    pub audio: Vec<AudioState>,
    #[n(5)]
    pub input_wait: Option<InputWait>,
    #[n(6)]
    pub settings: PresentationSettings,
    #[n(7)]
    pub tooltip: TooltipSettings,
    #[n(8)]
    pub resources: ResourceReplay,
    /// Independent top-layer HTML documents, in script insertion order.
    #[n(9)]
    pub html_island: Vec<erabasic_html::HtmlDocument>,
    #[n(10)]
    pub redraw: RedrawState,
}

#[cfg(test)]
mod resource_delta_tests {
    use super::*;

    fn canvas(id: i64) -> CanvasReplay {
        CanvasReplay {
            canvas_id: id,
            size: CanvasSize {
                width: 1,
                height: 1,
            },
            commands: Vec::new(),
            revision: 1,
        }
    }

    fn delta(edits: Vec<ResourceReplayListEdit<CanvasReplay>>) -> ResourceReplayDelta {
        ResourceReplayDelta {
            sprite_edits: Vec::new(),
            canvas_edits: edits,
            animation_timer_ms: 25,
        }
    }

    #[test]
    fn resource_delta_cbor_json_and_original_offsets_round_trip() {
        let original = ResourceReplay {
            canvases: vec![canvas(1), canvas(2), canvas(3)],
            ..ResourceReplay::default()
        };
        let change = delta(vec![
            ResourceReplayListEdit {
                start: 0,
                delete_count: 1,
                insert: Vec::new(),
            },
            ResourceReplayListEdit {
                start: 2,
                delete_count: 1,
                insert: vec![canvas(4), canvas(5)],
            },
        ]);
        let bytes = era_protocol::encode_canonical(&change).unwrap();
        assert_eq!(
            era_protocol::decode_canonical::<ResourceReplayDelta>(&bytes).unwrap(),
            change
        );
        assert_eq!(
            serde_json::from_slice::<ResourceReplayDelta>(&serde_json::to_vec(&change).unwrap())
                .unwrap(),
            change
        );
        let next = original.apply_delta(&change).unwrap();
        assert_eq!(next.canvases, vec![canvas(2), canvas(4), canvas(5)]);
        assert_eq!(next.animation_timer_ms, 25);
        assert_eq!(original.canvases, vec![canvas(1), canvas(2), canvas(3)]);
    }

    #[test]
    fn invalid_resource_edits_are_atomic() {
        let original = ResourceReplay {
            canvases: vec![canvas(1)],
            ..ResourceReplay::default()
        };
        let edit = |start, delete_count, insert| ResourceReplayListEdit {
            start,
            delete_count,
            insert,
        };
        for edits in [
            vec![edit(2, 0, vec![canvas(2)])],
            vec![edit(0, 2, Vec::new())],
            vec![edit(0, 0, Vec::new())],
            vec![edit(0, 0, vec![canvas(2)]), edit(0, 0, vec![canvas(3)])],
            vec![edit(0, 1, vec![canvas(2)]), edit(0, 1, vec![canvas(3)])],
            vec![edit(1, 0, vec![canvas(1)])],
        ] {
            assert!(original.apply_delta(&delta(edits)).is_err());
            assert_eq!(original.canvases, vec![canvas(1)]);
        }
    }
}
