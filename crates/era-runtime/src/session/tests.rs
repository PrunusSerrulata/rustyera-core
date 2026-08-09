use era_debug_protocol::{
    AuthorizedDebugRequest, DEBUG_PROTOCOL_VERSION, DebugCommand, DebugHello, DebugMessage,
    DebugRevoke, DebugScope, GrantToken,
};
use era_protocol::{Channel, Envelope, ProtocolBytes, decode_envelope, encode_envelope};
use era_runtime_protocol::{
    DisplayLine, DisplayRun, FileCategory, FileChange, FilePayload, PresentationOperation,
    PresentationSnapshot, ProjectIdentity, ProjectManifest, ProjectionLength, ProjectionSize,
    ProjectionTransform, SubmittedFile,
};
use erabasic_vm::VmDebugInspect;

use super::*;

fn capabilities() -> ClientCapabilities {
    ClientCapabilities {
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
        ],
        storage: StorageCapabilities {
            revisions: true,
            atomic_replace: true,
            missing_precondition: true,
            delete: true,
        },
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

mod debug_flow;
mod host_runtime;
mod host_system;
mod input_flow;
mod key_macro_input;
mod protocol_handshake;
mod protocol_project;
mod reload_transfer;
mod save_lifecycle;
