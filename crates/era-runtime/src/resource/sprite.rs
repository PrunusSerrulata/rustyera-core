//! Serializable sprite identity and frame geometry.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SpriteDefinition {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) revision: u64,
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
