use era_debug_protocol::{
    AuthorizedDebugRequest, DEBUG_PROTOCOL_VERSION, DebugCommand, DebugHello, DebugMessage,
    DebugRevoke, DebugScope, GrantToken,
};
use era_protocol::{
    Channel, Envelope, ProtocolBytes, decode_envelope, encode_canonical, encode_envelope,
};
use era_runtime_protocol::{
    CanvasReplayCommand, ConfigurationValueKind, DisplayLine, DisplayRun, FileCategory, FileChange,
    FilePayload, PresentationOperation, PresentationSnapshot, ProjectIdentity, ProjectManifest,
    ProjectionLength, ProjectionSize, ProjectionTransform, ShutdownRequest, SubmittedFile,
};
use erabasic_vm::VmDebugInspect;

use super::*;

fn capabilities() -> ClientCapabilities {
    ClientCapabilities {
        environment: Vec::new(),
        input_modalities: vec![era_runtime_protocol::InputModality::Keyboard],
        rich_text: false,
        html: false,
        graphics: false,
        audio: false,
        video: false,
        font_metrics: false,
        column_cells: true,
        separators: true,
        available_fonts: vec!["sans-serif".into()],
        services: vec![
            ServiceCapability {
                kind: ServiceKind::Clock,
                operation: LOCAL_DATE_TIME_OPERATION.into(),
                versions: VersionRange::exact(LOCAL_DATE_TIME_OPERATION_VERSION),
            },
            ServiceCapability {
                kind: ServiceKind::Entropy,
                operation: RANDOM_SEED_OPERATION.into(),
                versions: VersionRange::exact(RANDOM_SEED_OPERATION_VERSION),
            },
            ServiceCapability {
                kind: ServiceKind::InputState,
                operation: GET_KEY_STATE_OPERATION.into(),
                versions: VersionRange::exact(GET_KEY_STATE_OPERATION_VERSION),
            },
            ServiceCapability {
                kind: ServiceKind::Sql,
                operation: SQL_OPERATION.into(),
                versions: VersionRange::exact(SQL_OPERATION_VERSION),
            },
        ],
        storage: StorageCapabilities {
            revisions: true,
            atomic_replace: true,
            missing_precondition: true,
            delete: true,
        },
    }
}

fn profile_configuration_file(profile: erabasic_compat::CompatibilityProfileId) -> SubmittedFile {
    SubmittedFile {
        relative_path: "reraconfig.toml".into(),
        category: FileCategory::Configuration,
        payload: FilePayload::Utf8(format!(
            "[meta]\nschema_version = 4\n[compatibility]\nprofile = \"{}\"\n",
            profile.as_str()
        )),
        content_hash: None,
    }
}

#[allow(clippy::needless_pass_by_value)]
fn submit(session: &mut RuntimeSession, sequence: u64, message: RuntimeMessage) {
    let mut envelope = Envelope::new(
        Channel::Runtime,
        RUNTIME_PROTOCOL_VERSION,
        sequence,
        sequence + 1,
        message.tag(),
        ProtocolBytes::new(message.encode_payload().expect("encode message")),
    );
    if sequence != 0 {
        envelope.session = Some(session.options.session_id);
        envelope.session_epoch = Some(session.epoch);
    }
    let bytes = encode_envelope(&envelope, WireLimits::default()).expect("encode envelope");
    session.submit_envelope(&bytes).expect("submit envelope");
}

fn drain(session: &mut RuntimeSession) -> Vec<RuntimeMessage> {
    let mut messages = Vec::new();
    while let Some(bytes) = session.poll_envelope() {
        let envelope = decode_envelope(&bytes, WireLimits::default()).expect("decode envelope");
        messages.push(RuntimeMessage::from_envelope(&envelope).expect("decode message"));
    }
    messages
}

fn negotiated_session() -> RuntimeSession {
    negotiated_session_with_capabilities(capabilities())
}

fn negotiated_session_without_sql() -> RuntimeSession {
    let mut client_capabilities = capabilities();
    client_capabilities
        .services
        .retain(|service| service.kind != ServiceKind::Sql);
    negotiated_session_with_capabilities(client_capabilities)
}

fn negotiated_session_with_capabilities(client_capabilities: ClientCapabilities) -> RuntimeSession {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: client_capabilities,
            preferred_locales: vec!["ja".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    session
}

fn begin_state_import(
    session: &mut RuntimeSession,
    sequence: u64,
    kind: StateExportKind,
    total_bytes: u64,
    digest: Option<ProtocolBytes>,
) -> u64 {
    submit(
        session,
        sequence,
        RuntimeMessage::StateImportBegin(StateImportBegin {
            kind,
            total_bytes,
            digest,
            artifact_id: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StateImportAccepted(value) => Some(value.transfer_id),
            _ => None,
        })
        .expect("state import should be accepted")
}

fn projected_run_text(run: &DisplayRun) -> String {
    match run {
        DisplayRun::Text { text, .. } | DisplayRun::TextLayout { text, .. } => text.clone(),
        DisplayRun::Button { runs, .. } => runs.iter().map(projected_run_text).collect(),
        DisplayRun::ColumnCell { content, .. } => content.iter().map(projected_run_text).collect(),
        DisplayRun::HtmlDocument { .. }
        | DisplayRun::Image { .. }
        | DisplayRun::Shape { .. }
        | DisplayRun::Separator { .. }
        | DisplayRun::Space { .. } => String::new(),
    }
}

fn projected_line_text(line: &DisplayLine) -> String {
    line.runs.iter().map(projected_run_text).collect()
}

fn projected_presentation_text(snapshot: &PresentationSnapshot) -> String {
    snapshot
        .history
        .logical_lines
        .iter()
        .map(projected_line_text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn input_replay_records(session: &RuntimeSession) -> Vec<serde_json::Value> {
    std::str::from_utf8(&session.input_replay.encode().expect("encode input replay"))
        .expect("input replay is UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("parse input replay record"))
        .collect()
}

fn submit_debug(session: &mut RuntimeSession, sequence: u64, message: &DebugMessage) {
    let envelope = message
        .envelope(
            Some(session.options.session_id),
            Some(session.epoch),
            sequence,
            10_000 + sequence,
            None,
        )
        .expect("debug envelope");
    let bytes = encode_envelope(&envelope, WireLimits::default()).expect("encode debug");
    session.submit_envelope(&bytes).expect("submit debug");
}

mod audio_runtime;
mod debug_flow;
mod host_runtime;
mod host_system;
mod input_flow;
mod input_replay;
mod key_macro_input;
mod protocol_handshake;
mod protocol_project;
mod reload_transfer;
mod resource_storage;
mod save_lifecycle;
mod sql_map_snapshot;
mod sql_runtime;
