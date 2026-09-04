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

impl ResourceReplay {
    /// Validate every exact mutable-resource edge as an atomic identity/revision pair.
    ///
    /// # Errors
    ///
    /// Returns an error when the replay contains duplicate exact identities, an incomplete
    /// canvas identity/revision pair, or a reference whose exact definition is absent.
    pub fn validate_exact_references(&self) -> Result<(), String> {
        let mut sprites = BTreeSet::new();
        let mut canvases = BTreeSet::new();
        for sprite in &self.sprites {
            let key = (sprite.name.to_ascii_uppercase(), sprite.revision);
            if !sprites.insert(key) {
                return Err("duplicate exact sprite identity in resource replay".into());
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
