use era_runtime_protocol::{FileCategory, InputIntent, WaitKind};
use erabasic_vm::VmValue;
use serde::Serialize;

const INPUT_REPLAY_SCHEMA_VERSION: u32 = 2;
const MAXIMUM_REPLAY_STEPS: usize = 4096;
const MAXIMUM_REPLAY_BYTES: u64 = 16 * 1024 * 1024;
const REPLAY_LIMITATIONS: [&str; 3] = [
    "starting payload is not included in this archive",
    "reproduction requires the same starting state in a real client",
    "external time, devices, and services are not guaranteed deterministic",
];

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ReplayProject {
    pub(crate) revision: String,
    pub(crate) identity: String,
    pub(crate) locale: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NewGameTrigger {
    Start,
    ReturnToTitle,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ReplayOriginDetails {
    NewGame {
        seed: String,
        trigger: NewGameTrigger,
    },
    TraditionalSave {
        payload_digest: String,
        description: String,
        save_version: String,
    },
    OrdinarySave {
        slot: u32,
        storage_path: String,
        payload_digest: String,
    },
    VmSnapshot {
        payload_digest: String,
        snapshot_format: String,
        snapshot_origin: String,
        original_project_identity: String,
    },
    HotReload {
        before_revision: String,
        before_identity: String,
        after_revision: String,
        after_identity: String,
        changes: Vec<ReplayFileChange>,
    },
    InputUndo {
        checkpoint_slot: u32,
        save_digest: String,
        retained_input_count: usize,
    },
    ConfigurationUpdate {
        before_revision: String,
        before_identity: String,
        after_revision: String,
        after_identity: String,
        changed_codes: Vec<String>,
    },
    ExternalDataLoad {
        storage_path: String,
        payload_digest: String,
        data_type: ReplayExternalDataType,
    },
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ReplayOrigin {
    #[serde(flatten)]
    pub(crate) details: ReplayOriginDetails,
    pub(crate) project: ReplayProject,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ReplayFileChange {
    pub(crate) operation: ReplayFileOperation,
    pub(crate) relative_path: String,
    pub(crate) category: ReplayFileCategory,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReplayFileOperation {
    Upsert,
    Remove,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReplayExternalDataType {
    Global,
    Character,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReplayFileCategory {
    Csv,
    Erh,
    Erb,
    Configuration,
    Resource,
    ResourceManifest,
    Als,
    Erd,
}

impl From<FileCategory> for ReplayFileCategory {
    fn from(value: FileCategory) -> Self {
        match value {
            FileCategory::Csv => Self::Csv,
            FileCategory::Erh => Self::Erh,
            FileCategory::Erb => Self::Erb,
            FileCategory::Configuration => Self::Configuration,
            FileCategory::Resource => Self::Resource,
            FileCategory::ResourceManifest => Self::ResourceManifest,
            FileCategory::Als => Self::Als,
            FileCategory::Erd => Self::Erd,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub(crate) enum ReplayValue {
    Integer(String),
    String(String),
}

impl ReplayValue {
    pub(crate) fn from_vm(value: &VmValue) -> Option<Self> {
        match value {
            VmValue::Integer(value) => Some(Self::Integer(value.to_string())),
            VmValue::String(value) => Some(Self::String(value.clone())),
            VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ReplayButton {
    pub(crate) visible_text: String,
    pub(crate) title: Option<String>,
    pub(crate) alt_text: Option<String>,
    pub(crate) value: ReplayValue,
    pub(crate) ordinal: usize,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ReplayPrimitive {
    pub(crate) input_type: i32,
    pub(crate) result_1: i32,
    pub(crate) result_2: i32,
    pub(crate) result_3: i32,
    pub(crate) result_4: i32,
    pub(crate) selection: Option<ReplayValue>,
}

impl ReplayPrimitive {
    pub(crate) fn from_result(fields: [i32; 5], selection: Option<ReplayValue>) -> Self {
        Self {
            input_type: fields[0],
            result_1: fields[1],
            result_2: fields[2],
            result_3: fields[3],
            result_4: fields[4],
            selection,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReplayAction {
    Enter,
    AnyKey,
    Text,
    Button,
    Continue,
    Primitive,
    Timeout,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ReplayStep {
    pub(crate) source: Option<crate::input_source::InputSource>,
    pub(crate) record: &'static str,
    pub(crate) sequence: usize,
    pub(crate) action: ReplayAction,
    pub(crate) wait_kind: ReplayWaitKind,
    pub(crate) result: Option<ReplayValue>,
    pub(crate) message_skip: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) button: Option<ReplayButton>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) primitive: Option<ReplayPrimitive>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReplayWaitKind {
    EnterKey,
    AnyKey,
    IntegerValue,
    StringValue,
    Void,
    AnyValue,
    IntegerButton,
    StringButton,
    PrimitiveMouseKey,
}

impl From<WaitKind> for ReplayWaitKind {
    fn from(value: WaitKind) -> Self {
        match value {
            WaitKind::EnterKey => Self::EnterKey,
            WaitKind::AnyKey => Self::AnyKey,
            WaitKind::IntegerValue => Self::IntegerValue,
            WaitKind::StringValue => Self::StringValue,
            WaitKind::Void => Self::Void,
            WaitKind::AnyValue => Self::AnyValue,
            WaitKind::IntegerButton => Self::IntegerButton,
            WaitKind::StringButton => Self::StringButton,
            WaitKind::PrimitiveMouseKey => Self::PrimitiveMouseKey,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ReplayStepDraft {
    pub(crate) source: Option<crate::input_source::InputSource>,
    pub(crate) action: ReplayAction,
    pub(crate) wait_kind: ReplayWaitKind,
    pub(crate) result: Option<ReplayValue>,
    pub(crate) message_skip: bool,
    pub(crate) text: Option<String>,
    pub(crate) button: Option<ReplayButton>,
    pub(crate) primitive: Option<ReplayPrimitive>,
}

impl ReplayStepDraft {
    pub(crate) fn into_step(self, sequence: usize) -> ReplayStep {
        ReplayStep {
            source: self.source,
            record: "step",
            sequence,
            action: self.action,
            wait_kind: self.wait_kind,
            result: self.result,
            message_skip: self.message_skip,
            text: self.text,
            button: self.button,
            primitive: self.primitive,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct InputReplayHistory {
    origin: Option<ReplayOrigin>,
    steps: Vec<ReplayStep>,
    unavailable_reason: Option<&'static str>,
    available_header_bytes: u64,
    encoded_step_bytes: u64,
}

#[derive(Serialize)]
struct ReplayHeader<'a> {
    record: &'static str,
    schema_version: u32,
    fidelity: &'static str,
    status: &'static str,
    step_count: usize,
    limitations: [&'static str; 3],
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    origin: Option<&'a ReplayOrigin>,
}

impl InputReplayHistory {
    pub(crate) fn establish(&mut self, origin: ReplayOrigin, negotiated_limit: u64) {
        self.origin = Some(origin);
        self.steps.clear();
        self.unavailable_reason = None;
        self.available_header_bytes = self.encoded_header_bytes(true, 0);
        self.encoded_step_bytes = 0;
        if self.available_header_bytes > replay_size_limit(negotiated_limit) {
            self.make_unavailable(negotiated_limit);
        }
    }

    pub(crate) fn record(&mut self, draft: ReplayStepDraft, negotiated_limit: u64) {
        if self.origin.is_none() || self.unavailable_reason.is_some() {
            return;
        }
        if self.steps.len() == MAXIMUM_REPLAY_STEPS {
            self.make_unavailable(negotiated_limit);
            return;
        }
        let step = draft.into_step(self.steps.len() + 1);
        let encoded_len = serde_json::to_vec(&step)
            .ok()
            .and_then(|bytes| u64::try_from(bytes.len()).ok())
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let next_step_count = self.steps.len() + 1;
        let next_header_bytes = self.available_header_bytes.saturating_add(u64::from(
            decimal_digits(next_step_count) > decimal_digits(self.steps.len()),
        ));
        let estimated_bytes = self
            .available_header_bytes
            .max(next_header_bytes)
            .saturating_add(self.encoded_step_bytes)
            .saturating_add(encoded_len);
        if estimated_bytes > replay_size_limit(negotiated_limit) {
            self.make_unavailable(negotiated_limit);
            return;
        }
        self.available_header_bytes = next_header_bytes;
        self.encoded_step_bytes = self.encoded_step_bytes.saturating_add(encoded_len);
        self.steps.push(step);
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, serde_json::Error> {
        let available = self.origin.is_some() && self.unavailable_reason.is_none();
        let header = ReplayHeader {
            record: "header",
            schema_version: INPUT_REPLAY_SCHEMA_VERSION,
            fidelity: "manual_path",
            status: if available {
                "available"
            } else {
                "unavailable"
            },
            step_count: if available { self.steps.len() } else { 0 },
            limitations: REPLAY_LIMITATIONS,
            reason: self
                .unavailable_reason
                .or_else(|| self.origin.is_none().then_some("origin_unavailable")),
            origin: self.origin.as_ref(),
        };
        let mut bytes = serde_json::to_vec(&header)?;
        bytes.push(b'\n');
        if available {
            for step in &self.steps {
                serde_json::to_writer(&mut bytes, step)?;
                bytes.push(b'\n');
            }
        }
        Ok(bytes)
    }

    fn make_unavailable(&mut self, negotiated_limit: u64) {
        self.steps.clear();
        self.unavailable_reason = Some("history_limit_exceeded");
        self.available_header_bytes = 0;
        self.encoded_step_bytes = 0;
        if self.encoded_header_bytes(false, 0) > replay_size_limit(negotiated_limit) {
            self.origin = None;
        }
    }

    fn encoded_header_bytes(&self, available: bool, step_count: usize) -> u64 {
        let header = ReplayHeader {
            record: "header",
            schema_version: INPUT_REPLAY_SCHEMA_VERSION,
            fidelity: "manual_path",
            status: if available {
                "available"
            } else {
                "unavailable"
            },
            step_count,
            limitations: REPLAY_LIMITATIONS,
            reason: (!available).then_some("history_limit_exceeded"),
            origin: self.origin.as_ref(),
        };
        serde_json::to_vec(&header)
            .ok()
            .and_then(|bytes| u64::try_from(bytes.len()).ok())
            .unwrap_or(u64::MAX)
            .saturating_add(1)
    }
}

const fn replay_size_limit(negotiated_limit: u64) -> u64 {
    if negotiated_limit < MAXIMUM_REPLAY_BYTES {
        negotiated_limit
    } else {
        MAXIMUM_REPLAY_BYTES
    }
}

const fn decimal_digits(mut value: usize) -> u32 {
    let mut digits = 1;
    while value >= 10 {
        value /= 10;
        digits += 1;
    }
    digits
}

pub(crate) fn action_for_intent(intent: &InputIntent) -> Option<ReplayAction> {
    match intent {
        InputIntent::Enter => Some(ReplayAction::Enter),
        InputIntent::AnyKey(_) => Some(ReplayAction::AnyKey),
        InputIntent::CommitText(_) => Some(ReplayAction::Text),
        InputIntent::Activate(_) => Some(ReplayAction::Button),
        InputIntent::Continue => Some(ReplayAction::Continue),
        InputIntent::Primitive(_) => Some(ReplayAction::Primitive),
        InputIntent::Cancel | InputIntent::ActivateKeyMacro { .. } => None,
    }
}

pub(crate) fn digest_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

pub(crate) fn identity_hex(identity: &[u8; 32]) -> String {
    blake3::Hash::from_bytes(*identity).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history() -> InputReplayHistory {
        let mut history = InputReplayHistory::default();
        history.establish(
            ReplayOrigin {
                details: ReplayOriginDetails::NewGame {
                    seed: u64::MAX.to_string(),
                    trigger: NewGameTrigger::Start,
                },
                project: ReplayProject {
                    revision: u64::MAX.to_string(),
                    identity: "ab".repeat(32),
                    locale: "en-US".into(),
                },
            },
            MAXIMUM_REPLAY_BYTES,
        );
        history
    }

    fn text_step() -> ReplayStepDraft {
        ReplayStepDraft {
            source: None,
            action: ReplayAction::Text,
            wait_kind: ReplayWaitKind::StringValue,
            result: Some(ReplayValue::String("actual".into())),
            message_skip: false,
            text: Some("entered".into()),
            button: None,
            primitive: None,
        }
    }

    #[test]
    fn jsonl_is_deterministic_utf8_with_string_integers_and_trailing_newlines() {
        let mut history = history();
        history.record(text_step(), MAXIMUM_REPLAY_BYTES);

        let first = history.encode().expect("encode replay");
        let second = history.encode().expect("encode replay again");

        assert_eq!(first, second);
        assert!(!first.starts_with(&[0xef, 0xbb, 0xbf]));
        assert!(first.ends_with(b"\n"));
        let lines = std::str::from_utf8(&first)
            .expect("UTF-8 replay")
            .lines()
            .collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains(&format!(r#""seed":"{}""#, u64::MAX)));
        assert!(lines[0].contains(r#""status":"available""#));
        assert!(lines[1].contains(r#""sequence":1"#));
    }

    #[test]
    fn exceeding_the_step_limit_makes_only_the_current_segment_unavailable() {
        let mut history = history();
        for _ in 0..=MAXIMUM_REPLAY_STEPS {
            history.record(text_step(), MAXIMUM_REPLAY_BYTES);
        }
        let unavailable = String::from_utf8(history.encode().expect("encode unavailable replay"))
            .expect("UTF-8 unavailable replay");
        assert_eq!(unavailable.lines().count(), 1);
        assert!(unavailable.contains(r#""status":"unavailable""#));
        assert!(unavailable.contains(r#""reason":"history_limit_exceeded""#));

        history.establish(
            ReplayOrigin {
                details: ReplayOriginDetails::NewGame {
                    seed: "7".into(),
                    trigger: NewGameTrigger::ReturnToTitle,
                },
                project: ReplayProject {
                    revision: "2".into(),
                    identity: "cd".repeat(32),
                    locale: "ja".into(),
                },
            },
            MAXIMUM_REPLAY_BYTES,
        );
        history.record(text_step(), MAXIMUM_REPLAY_BYTES);
        let recovered = String::from_utf8(history.encode().expect("encode recovered replay"))
            .expect("UTF-8 recovered replay");
        assert!(recovered.contains(r#""status":"available""#));
        assert_eq!(recovered.lines().count(), 2);
    }

    #[test]
    fn every_origin_schema_serializes_with_a_stable_kind_and_replaces_steps() {
        let project = ReplayProject {
            revision: "12".into(),
            identity: "ef".repeat(32),
            locale: "zh-CN".into(),
        };
        let origins = vec![
            ReplayOriginDetails::TraditionalSave {
                payload_digest: "01".repeat(32),
                description: "slot description".into(),
                save_version: "3".into(),
            },
            ReplayOriginDetails::OrdinarySave {
                slot: 7,
                storage_path: "save07.sav".into(),
                payload_digest: "02".repeat(32),
            },
            ReplayOriginDetails::VmSnapshot {
                payload_digest: "03".repeat(32),
                snapshot_format: "runtime_snapshot_v5".into(),
                snapshot_origin: "normal".into(),
                original_project_identity: "04".repeat(32),
            },
            ReplayOriginDetails::HotReload {
                before_revision: "12".into(),
                before_identity: "05".repeat(32),
                after_revision: "13".into(),
                after_identity: "06".repeat(32),
                changes: [FileCategory::Erb, FileCategory::Als, FileCategory::Erd]
                    .into_iter()
                    .map(|category| ReplayFileChange {
                        operation: ReplayFileOperation::Upsert,
                        relative_path: format!("ERB/reloaded.{category:?}").to_ascii_lowercase(),
                        category: category.into(),
                    })
                    .collect(),
            },
            ReplayOriginDetails::InputUndo {
                checkpoint_slot: 0,
                save_digest: "07".repeat(32),
                retained_input_count: 2,
            },
            ReplayOriginDetails::ConfigurationUpdate {
                before_revision: "13".into(),
                before_identity: "08".repeat(32),
                after_revision: "13".into(),
                after_identity: "09".repeat(32),
                changed_codes: vec!["AudioVolume".into()],
            },
            ReplayOriginDetails::ExternalDataLoad {
                storage_path: "global.sav".into(),
                payload_digest: "0a".repeat(32),
                data_type: ReplayExternalDataType::Global,
            },
            ReplayOriginDetails::ExternalDataLoad {
                storage_path: "chara01.sav".into(),
                payload_digest: "0b".repeat(32),
                data_type: ReplayExternalDataType::Character,
            },
        ];
        let expected_kinds = [
            "traditional_save",
            "ordinary_save",
            "vm_snapshot",
            "hot_reload",
            "input_undo",
            "configuration_update",
            "external_data_load",
            "external_data_load",
        ];
        let mut history = history();
        history.record(text_step(), MAXIMUM_REPLAY_BYTES);
        for (details, expected_kind) in origins.into_iter().zip(expected_kinds) {
            history.establish(
                ReplayOrigin {
                    details,
                    project: project.clone(),
                },
                MAXIMUM_REPLAY_BYTES,
            );
            let jsonl = String::from_utf8(history.encode().expect("encode replaced segment"))
                .expect("UTF-8 segment");
            assert_eq!(jsonl.lines().count(), 1);
            assert!(jsonl.contains(&format!(r#""kind":"{expected_kind}""#)));
            assert!(jsonl.contains(r#""step_count":0"#));
            if expected_kind == "hot_reload" {
                for category in ["erb", "als", "erd"] {
                    assert!(jsonl.contains(&format!(r#""category":"{category}""#)));
                }
            }
        }
    }

    #[test]
    fn every_public_action_name_is_stable_and_non_semantic_intents_are_excluded() {
        let token = era_runtime_protocol::InteractionToken { epoch: 1, id: 2 };
        let primitive = era_runtime_protocol::PrimitiveInput {
            input_type: 1,
            result_1: 2,
            result_2: 3,
            result_3: 4,
            result_4: 5,
            selection_token: None,
        };
        let cases = [
            (InputIntent::Enter, Some("enter")),
            (InputIntent::AnyKey("x".into()), Some("any_key")),
            (InputIntent::CommitText("x".into()), Some("text")),
            (InputIntent::Activate(token), Some("button")),
            (InputIntent::Continue, Some("continue")),
            (InputIntent::Primitive(primitive), Some("primitive")),
            (InputIntent::Cancel, None),
            (InputIntent::ActivateKeyMacro { group: 1, slot: 2 }, None),
        ];
        for (intent, expected) in cases {
            assert_eq!(
                action_for_intent(&intent)
                    .map(|action| serde_json::to_value(action).expect("serialize action")),
                expected.map(serde_json::Value::from)
            );
        }
        assert_eq!(
            serde_json::to_value(ReplayAction::Timeout).expect("serialize timeout"),
            "timeout"
        );
    }

    #[test]
    fn negotiated_encoded_size_limit_marks_the_segment_unavailable() {
        let mut history = history();
        let mut oversized = text_step();
        oversized.text = Some("x".repeat(4096));
        history.record(oversized, 1024);
        let jsonl = String::from_utf8(history.encode().expect("encode unavailable segment"))
            .expect("UTF-8 segment");
        assert!(jsonl.contains(r#""reason":"history_limit_exceeded""#));
        assert_eq!(jsonl.lines().count(), 1);
    }

    #[test]
    fn large_origin_with_many_steps_keeps_exact_limits_and_deterministic_output() {
        let changes = (0..512)
            .map(|index| ReplayFileChange {
                operation: ReplayFileOperation::Upsert,
                relative_path: format!("ERB/{index:04}-{}.erb", "segment".repeat(48)),
                category: ReplayFileCategory::Erb,
            })
            .collect();
        let mut history = InputReplayHistory::default();
        history.establish(
            ReplayOrigin {
                details: ReplayOriginDetails::HotReload {
                    before_revision: "1".into(),
                    before_identity: "01".repeat(32),
                    after_revision: "2".into(),
                    after_identity: "02".repeat(32),
                    changes,
                },
                project: ReplayProject {
                    revision: "2".into(),
                    identity: "02".repeat(32),
                    locale: "en-US".into(),
                },
            },
            MAXIMUM_REPLAY_BYTES,
        );
        for _ in 0..128 {
            history.record(text_step(), MAXIMUM_REPLAY_BYTES);
        }

        let first = history.encode().expect("encode large replay origin");
        let second = history.encode().expect("encode large replay origin again");
        assert_eq!(first, second);
        assert_eq!(std::str::from_utf8(&first).unwrap().lines().count(), 129);
        assert!(u64::try_from(first.len()).unwrap() <= MAXIMUM_REPLAY_BYTES);

        let current_bytes = history
            .available_header_bytes
            .saturating_add(history.encoded_step_bytes);
        history.record(text_step(), current_bytes);
        let unavailable = history.encode().expect("encode unavailable large replay");
        assert_eq!(
            std::str::from_utf8(&unavailable).unwrap().lines().count(),
            1
        );
        assert!(
            unavailable
                .windows(b"history_limit_exceeded".len())
                .any(|window| { window == b"history_limit_exceeded" })
        );
    }
}
