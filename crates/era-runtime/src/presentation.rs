#[cfg(test)]
use era_runtime_protocol::PresentationHistoryOperation;
use era_runtime_protocol::{
    CellAlignment, CellWidthIntent, Color, DisplayLine, DisplayRun, InteractionToken,
    LineAlignment, LogicalLength, MediaPlacement, PresentationLength, ProtocolValue,
    RationalOpacity, SeparatorRole, Shape, SystemTextArgument, SystemTextKey, SystemTextRef,
    TextStyle,
};
use erabasic_vm::{CharacterWidthMode, VmValue};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

mod model;

use self::model::{PresentationDelivery, PresentationDirty, PresentationHistoryEdit};
pub(crate) use self::model::{PresentationModel, PresentationUpdate};

include!("presentation/content.rs");
include!("presentation/style_and_history.rs");
include!("presentation/style_helpers.rs");

mod defaults;
mod delivery;
mod media;
mod projection;
mod scene;
#[cfg(test)]
mod tests;

pub(crate) use self::projection::display_value;
use self::projection::{
    append_html_run, append_log_run, auto_button_values, bind_auto_buttons, color_rgb,
    enabled_button_value, project_lines, rebind_runs, rgb_color, run_is_empty,
};
