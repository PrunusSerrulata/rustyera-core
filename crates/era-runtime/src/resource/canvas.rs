use std::mem::size_of;

use era_runtime_protocol::CanvasRect;

use super::{
    CanvasCommand, CanvasSurface, ExactRevisionStore, ResourceGraph, SpriteDefinition, SpriteFrame,
};

pub(super) const MAXIMUM_CANVAS_COMMAND_BYTES: usize = 64 * 1024 * 1024;
const MAXIMUM_CANVASES: usize = 65_536;

include!("canvas/drawing.rs");
include!("canvas/sprites.rs");
include!("canvas/support.rs");
