use std::collections::{BTreeMap, BTreeSet, VecDeque};

use era_runtime_protocol::{
    CellAlignment, CellWidthIntent, Color, DisplayLine, DisplayRun, InteractionToken,
    ProtocolValue, TextStyle,
};
use erabasic_vm::{CharacterWidthMode, VmValue, display_width, emuera_display_width};
use unicode_segmentation::UnicodeSegmentation as _;

mod serialization;
pub(in crate::presentation) use serialization::{append_html_run, append_log_run};
pub(super) use serialization::{append_plain_run, append_printed_html_run};

include!("projection/interactions.rs");
include!("projection/layout.rs");
