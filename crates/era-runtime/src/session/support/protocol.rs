#[allow(clippy::wildcard_imports)]
use super::super::*;

pub(in super::super) fn selected_capabilities(client: &ClientCapabilities) -> ClientCapabilities {
    let services = selected_service_capabilities(&client.services);
    let font_metrics = client.font_metrics
        && services.iter().any(|capability| {
            capability.kind == ServiceKind::FontMetrics
                && capability.operation == GGET_TEXT_SIZE_OPERATION
        });
    ClientCapabilities {
        input_modalities: client.input_modalities.clone(),
        rich_text: client.rich_text,
        html: client.html,
        graphics: client.graphics,
        audio: client.audio,
        // Video still requires a typed playback contract.
        video: false,
        font_metrics,
        column_cells: client.column_cells,
        separators: client.separators,
        available_fonts: {
            let mut fonts = client.available_fonts.clone();
            fonts.sort_by_key(|name| name.to_lowercase());
            fonts.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
            fonts
        },
        services,
        storage: client.storage,
    }
}

pub(in super::super) fn selected_service_capabilities(
    client: &[ServiceCapability],
) -> Vec<ServiceCapability> {
    let mut selected = client
        .iter()
        .filter_map(|capability| {
            let supported = match (capability.kind, capability.operation.as_str()) {
                (ServiceKind::Clock, LOCAL_DATE_TIME_OPERATION) => {
                    LOCAL_DATE_TIME_OPERATION_VERSION
                }
                (ServiceKind::Entropy, RANDOM_SEED_OPERATION) => RANDOM_SEED_OPERATION_VERSION,
                (ServiceKind::InputState, GET_KEY_STATE_OPERATION) => {
                    GET_KEY_STATE_OPERATION_VERSION
                }
                (ServiceKind::Image, IMAGE_METADATA_OPERATION) => IMAGE_METADATA_OPERATION_VERSION,
                (ServiceKind::Image, IMAGE_PIXEL_OPERATION) => IMAGE_PIXEL_OPERATION_VERSION,
                (ServiceKind::Network, UPDATE_CHECK_OPERATION) => UPDATE_CHECK_OPERATION_VERSION,
                (ServiceKind::OpenUrl, OPEN_URL_OPERATION) => OPEN_URL_OPERATION_VERSION,
                (ServiceKind::PresentationQuery, GET_DISPLAY_LINE_OPERATION) => {
                    GET_DISPLAY_LINE_OPERATION_VERSION
                }
                (ServiceKind::PresentationQuery, HTML_GET_PRINTED_STR_OPERATION) => {
                    HTML_GET_PRINTED_STR_OPERATION_VERSION
                }
                (ServiceKind::PresentationQuery, HTML_STRING_LEN_OPERATION) => {
                    HTML_STRING_LEN_OPERATION_VERSION
                }
                (ServiceKind::PresentationQuery, HTML_SUBSTRING_OPERATION) => {
                    HTML_SUBSTRING_OPERATION_VERSION
                }
                (ServiceKind::PresentationQuery, HTML_STRING_LINES_OPERATION) => {
                    HTML_STRING_LINES_OPERATION_VERSION
                }
                (ServiceKind::PresentationQuery, SERIALIZE_PHYSICAL_HISTORY_OPERATION) => {
                    SERIALIZE_PHYSICAL_HISTORY_OPERATION_VERSION
                }
                (ServiceKind::FontMetrics, GGET_TEXT_SIZE_OPERATION) => {
                    GGET_TEXT_SIZE_OPERATION_VERSION
                }
                (ServiceKind::Canvas, SAMPLE_CANVAS_PIXEL_OPERATION) => {
                    SAMPLE_CANVAS_PIXEL_OPERATION_VERSION
                }
                (ServiceKind::Canvas, DECODE_CANVAS_IMAGE_OPERATION) => {
                    DECODE_CANVAS_IMAGE_OPERATION_VERSION
                }
                (ServiceKind::Canvas, ENCODE_CANVAS_PNG_OPERATION) => {
                    ENCODE_CANVAS_PNG_OPERATION_VERSION
                }
                // Extension operations are application-defined. Select the client's
                // maximum now; a later registry declaration must bind that exact version.
                (ServiceKind::Extension, _) => capability.versions.maximum,
                _ => return None,
            };
            negotiate_version(capability.versions, VersionRange::exact(supported)).map(|version| {
                ServiceCapability {
                    kind: capability.kind,
                    operation: if capability.kind == ServiceKind::Extension {
                        capability.operation.to_ascii_lowercase()
                    } else {
                        capability.operation.clone()
                    },
                    versions: VersionRange::exact(version),
                }
            })
        })
        .collect::<Vec<_>>();
    selected.sort_by(|left, right| {
        (left.kind, left.operation.as_str()).cmp(&(right.kind, right.operation.as_str()))
    });
    selected.dedup_by(|left, right| left.kind == right.kind && left.operation == right.operation);
    selected
}

pub(in super::super) fn select_locale(preferred: &[String]) -> &'static str {
    for locale in preferred {
        let locale = locale.to_ascii_lowercase();
        if locale == "zh-hans" || locale.starts_with("zh-cn") || locale.starts_with("zh-sg") {
            return "zh-Hans";
        }
        if locale == "en" || locale.starts_with("en-") {
            return "en";
        }
        if locale == "ja" || locale.starts_with("ja-") {
            return "ja";
        }
    }
    "ja"
}

pub(in super::super) fn localized_system_text(locale: &str, key: SystemTextKey) -> String {
    let value = match (locale, key) {
        ("zh-Hans", SystemTextKey::InvalidValue) => "输入无效",
        ("zh-Hans", SystemTextKey::SaveQuestion) => "请选择保存位置",
        ("zh-Hans", SystemTextKey::LoadQuestion) => "请选择要读取的存档",
        ("zh-Hans", SystemTextKey::OverwriteQuestion) => "要覆盖这个存档吗？",
        ("zh-Hans", SystemTextKey::NotEnoughMoney) => "金钱不足",
        ("zh-Hans", SystemTextKey::OutOfStock) => "无法购买",
        ("zh-Hans", SystemTextKey::AutoSaveFailed) => "自动保存失败",
        ("zh-Hans", SystemTextKey::AutoSaveSkipped) => "已跳过自动保存",
        ("zh-Hans", SystemTextKey::ContinuousTrainProgress) => "＜连续执行：第 {0}/{1} 个命令＞",
        ("zh-Hans", SystemTextKey::ContinuousTrainCommandFailed) => "无法执行命令",
        ("zh-Hans", SystemTextKey::PressAnyKey) => "请按任意键",
        ("zh-Hans", SystemTextKey::SaveSlot) => "存档",
        ("zh-Hans", SystemTextKey::Back) => "返回",
        ("zh-Hans", SystemTextKey::NewGame) => "开始新游戏",
        ("zh-Hans", SystemTextKey::LoadGame) => "读取存档",
        ("en", SystemTextKey::InvalidValue) => "Invalid value",
        ("en", SystemTextKey::SaveQuestion) => "Select a save slot",
        ("en", SystemTextKey::LoadQuestion) => "Select a save to load",
        ("en", SystemTextKey::OverwriteQuestion) => "Overwrite this save?",
        ("en", SystemTextKey::NotEnoughMoney) => "Not enough money",
        ("en", SystemTextKey::OutOfStock) => "This item cannot be purchased",
        ("en", SystemTextKey::AutoSaveFailed) => "Autosave failed",
        ("en", SystemTextKey::AutoSaveSkipped) => "Autosave skipped",
        ("en", SystemTextKey::ContinuousTrainProgress) => "<Continuous command: {0}/{1}>",
        ("en", SystemTextKey::ContinuousTrainCommandFailed) => "The command could not be executed",
        ("en", SystemTextKey::PressAnyKey) => "Press any key",
        ("en", SystemTextKey::SaveSlot) => "Save",
        ("en", SystemTextKey::Back) => "Back",
        ("en", SystemTextKey::NewGame) => "Start a new game",
        ("en", SystemTextKey::LoadGame) => "Load game",
        (_, SystemTextKey::InvalidValue) => "入力が正しくありません",
        (_, SystemTextKey::SaveQuestion) => "セーブするデータを選択してください",
        (_, SystemTextKey::LoadQuestion) => "ロードするデータを選択してください",
        (_, SystemTextKey::OverwriteQuestion) => "上書きしてよろしいですか？",
        (_, SystemTextKey::NotEnoughMoney) => "所持金が足りません",
        (_, SystemTextKey::OutOfStock) => "購入できません",
        (_, SystemTextKey::AutoSaveFailed) => "オートセーブに失敗しました",
        (_, SystemTextKey::AutoSaveSkipped) => "オートセーブをスキップしました",
        (_, SystemTextKey::ContinuousTrainProgress) => "＜コマンド連続実行：{0}/{1}＞",
        (_, SystemTextKey::ContinuousTrainCommandFailed) => "コマンドを実行できませんでした",
        (_, SystemTextKey::PressAnyKey) => "何かキーを押してください",
        (_, SystemTextKey::SaveSlot) => "セーブデータ",
        (_, SystemTextKey::Back) => "戻る",
        (_, SystemTextKey::NewGame) => "最初からはじめる",
        (_, SystemTextKey::LoadGame) => "ロードする",
    };
    value.into()
}

pub(in super::super) fn protocol_to_vm(value: &era_runtime_protocol::ProtocolValue) -> VmValue {
    match value {
        era_runtime_protocol::ProtocolValue::Integer(value) => VmValue::Integer(*value),
        era_runtime_protocol::ProtocolValue::String(value) => VmValue::String(value.clone()),
        era_runtime_protocol::ProtocolValue::Boolean(value) => VmValue::Integer(i64::from(*value)),
        era_runtime_protocol::ProtocolValue::Bytes(_) => VmValue::String(String::new()),
    }
}

pub(in super::super) fn extension_protocol_value(
    value: era_runtime_protocol::ProtocolValue,
) -> Option<VmValue> {
    match value {
        era_runtime_protocol::ProtocolValue::Integer(value) => Some(VmValue::Integer(value)),
        era_runtime_protocol::ProtocolValue::String(value) => Some(VmValue::String(value)),
        era_runtime_protocol::ProtocolValue::Boolean(value) => {
            Some(VmValue::Integer(i64::from(value)))
        }
        era_runtime_protocol::ProtocolValue::Bytes(_) => None,
    }
}

pub(in super::super) fn calendar_number(time: LocalDateTimeResponse) -> i64 {
    let date = i64::from(time.year) * 10_000_000_000
        + i64::from(time.month) * 100_000_000
        + i64::from(time.day) * 1_000_000
        + i64::from(time.hour) * 10_000
        + i64::from(time.minute) * 100
        + i64::from(time.second);
    date * 1000 + i64::from(time.millisecond)
}

pub(in super::super) fn complete_frozen_clock(
    vm: &mut RuntimeVm,
    request: &VmHostRequest,
    time: LocalDateTimeResponse,
) -> Result<(), RuntimeError> {
    let name = request.import.import.name.to_ascii_uppercase();
    let operation = match name.as_str() {
        "GETTIME" => ClockOperation::Time,
        "GETTIMES" => ClockOperation::Times,
        "GETMILLISECOND" => ClockOperation::Millisecond,
        "GETSECOND" => ClockOperation::Second,
        _ => {
            return Err(RuntimeError::Internal(format!(
                "clock operation {name} has no frozen candidate implementation"
            )));
        }
    };
    let mut writes = Vec::new();
    let value = if request.import.import.result.is_none() {
        if let Some(target) = global_place(vm, "RESULT") {
            writes.push(HostWrite {
                target,
                value: VmValue::Integer(calendar_number(time)),
            });
        }
        if let Some(target) = global_place(vm, "RESULTS") {
            writes.push(HostWrite {
                target,
                value: VmValue::String(calendar_string(time)),
            });
        }
        None
    } else {
        Some(match operation {
            ClockOperation::Time => VmValue::Integer(calendar_number(time)),
            ClockOperation::Times => VmValue::String(calendar_string(time)),
            ClockOperation::Millisecond => VmValue::Integer(milliseconds_since_year_one(time)),
            ClockOperation::Second => VmValue::Integer(milliseconds_since_year_one(time) / 1_000),
        })
    };
    commit_completion(
        vm,
        request.id,
        VmHostCompletion::Ready(HostReady { value, writes }),
    )
}

pub(in super::super) fn calendar_string(time: LocalDateTimeResponse) -> String {
    format!(
        "{:04}/{:02}/{:02} {:02}:{:02}:{:02}",
        time.year, time.month, time.day, time.hour, time.minute, time.second
    )
}

pub(in super::super) fn milliseconds_since_year_one(time: LocalDateTimeResponse) -> i64 {
    const DAYS_BEFORE_MONTH: [i64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    // This is the proleptic Gregorian calendar used by DateTime.Now.Ticks.
    let year_before = i64::from(time.year) - 1;
    let days_before_year =
        year_before * 365 + year_before / 4 - year_before / 100 + year_before / 400;
    let mut days = days_before_year
        + DAYS_BEFORE_MONTH[usize::from(time.month.saturating_sub(1).min(11))]
        + i64::from(time.day.saturating_sub(1));
    if time.month > 2 && is_leap_year(time.year) {
        days += 1;
    }
    (((days * 24 + i64::from(time.hour)) * 60 + i64::from(time.minute)) * 60
        + i64::from(time.second))
        * 1000
        + i64::from(time.millisecond)
}

const fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

pub(in super::super) fn intersect_limits(
    left: RuntimeLimits,
    right: RuntimeLimits,
) -> RuntimeLimits {
    RuntimeLimits {
        maximum_envelope_bytes: left
            .maximum_envelope_bytes
            .min(right.maximum_envelope_bytes),
        maximum_payload_bytes: left.maximum_payload_bytes.min(right.maximum_payload_bytes),
        maximum_pending_requests: left
            .maximum_pending_requests
            .min(right.maximum_pending_requests),
        maximum_journal_entries: left
            .maximum_journal_entries
            .min(right.maximum_journal_entries),
        maximum_drive_instructions: left
            .maximum_drive_instructions
            .min(right.maximum_drive_instructions),
        maximum_transfer_bytes: left
            .maximum_transfer_bytes
            .min(right.maximum_transfer_bytes),
        maximum_journal_bytes: left.maximum_journal_bytes.min(right.maximum_journal_bytes),
    }
}

pub(in super::super) fn debugger_suspends_message(message: &RuntimeMessage) -> bool {
    matches!(
        message,
        RuntimeMessage::ProjectManifest(_)
            | RuntimeMessage::ProjectLoad(_)
            | RuntimeMessage::ProjectAnalysisRequest(_)
            | RuntimeMessage::KeyMacroProfileSubmit(_)
            | RuntimeMessage::KeyMacroCommand(_)
            | RuntimeMessage::ExtensionRegistrySubmit(_)
            | RuntimeMessage::ReturnToTitle(_)
            | RuntimeMessage::Start(_)
            | RuntimeMessage::Input(_)
            | RuntimeMessage::InputUndoRequest(_)
            | RuntimeMessage::ServiceResponse(_)
            | RuntimeMessage::StorageResponse(_)
            | RuntimeMessage::StateExportRequest(_)
            | RuntimeMessage::StateImportBegin(_)
            | RuntimeMessage::StateImportChunk(_)
            | RuntimeMessage::StateImportCommit(_)
            | RuntimeMessage::StateExportChunkRequest(_)
            | RuntimeMessage::StateTransferCancel(_)
            | RuntimeMessage::FullProjectManifest(_)
            | RuntimeMessage::StateExportCancel(_)
            | RuntimeMessage::ReloadProject(_)
            | RuntimeMessage::ApplyClientPreferences(_)
    )
}
