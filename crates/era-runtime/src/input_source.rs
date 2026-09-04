//! Provenance of one admission and its expanded fragments; no parser here.
use erabasic_bytecode::{Digest, SymbolKey};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct SequenceSite {
    pub artifact: Digest,
    pub function: SymbolKey,
    pub instruction: u32,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct PendingSequence {
    pub text: String,
    pub site: SequenceSite,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum InputRoot {
    External,
    Sequence(SequenceSite),
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct InputSource {
    pub root: InputRoot,
    pub admission: u64,
    pub fragment: u32,
    pub raw: std::sync::Arc<String>,
    pub macro_enabled: bool,
    pub message_skip: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct RecordedInput {
    pub value: String,
    pub source: Option<InputSource>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct QueuedInput {
    pub text: String,
    pub message_skip: bool,
    pub source: InputSource,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct InputController {
    pub pending_sequence: Option<PendingSequence>,
    pub macro_enabled: bool,
    pub next_admission: u64,
}
impl Default for InputController {
    fn default() -> Self {
        Self {
            pending_sequence: None,
            macro_enabled: true,
            next_admission: 1,
        }
    }
}
impl InputController {
    pub(crate) fn admit(
        &mut self,
        root: InputRoot,
        raw: String,
        message_skip: bool,
    ) -> Result<InputSource, &'static str> {
        let admission = self.next_admission;
        self.next_admission = admission
            .checked_add(1)
            .ok_or("input admission identity exhausted")?;
        Ok(InputSource {
            root,
            admission,
            fragment: 0,
            raw: std::sync::Arc::new(raw),
            macro_enabled: self.macro_enabled,
            message_skip,
        })
    }
}

impl InputSource {
    // Admission ids distinguish queue pieces in one execution. Rejected external
    // attempts need not recur during undo; compare semantic provenance instead.
    pub(crate) fn same_replay_origin(&self, other: &Self) -> bool {
        self.root == other.root
            && self.fragment == other.fragment
            && self.raw == other.raw
            && self.macro_enabled == other.macro_enabled
            && self.message_skip == other.message_skip
    }
}

impl RecordedInput {
    pub(crate) fn storage_bytes(&self) -> Option<u64> {
        (self.value.len() as u64).checked_add(
            self.source
                .as_ref()
                .map_or(0, |source| source.raw.len() as u64),
        )
    }
}
