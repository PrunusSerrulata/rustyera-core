//! Deterministic runtime performance replay for isolated snake-TW project copies.

mod input;
mod output;
mod session;
mod trace;

use std::collections::BTreeSet;
use std::error::Error;
use std::io::BufRead;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::Instant;

use era_protocol::{VersionRange, WireLimits};
use era_runtime::{ProjectProgressReporter, RuntimeDriveState};
use era_runtime_protocol::{
    ClientHello, ConfigurationClientProfile, RUNTIME_PROTOCOL_VERSION, RuntimeLogLevel,
    RuntimeMessage, StartMode, StartRequest,
};
use erabasic_compat::CompatibilityProfileId;
use serde::Serialize;
use serde_json::json;

use super::perf_allocator as allocator;
use allocator::{AllocationStats, MeasurementMode};
use input::PreparedProject;
use output::{JsonlSink, WatchdogThrottle};
use session::{PerfSession, PumpObservation, drive_state_name};
use trace::{PerfTrace, TraceStep};

pub(super) type AuditResult<T> = Result<T, Box<dyn Error>>;
pub(super) const OUTPUT_SCHEMA_VERSION: u32 = 2;
pub(super) const TRACE_SCHEMA_VERSION: u32 = 2;
pub(super) const SNAKE_PROFILE: CompatibilityProfileId = CompatibilityProfileId::EmueraSkiaSnake;
const MAX_ITERATIONS: u32 = 100;
const DEFAULT_MAX_PUMPS: u64 = 200_000;
const CALIBRATION_SAMPLES: u32 = 10_000;

#[derive(Debug, Eq, PartialEq)]
pub(super) struct Cli {
    project: PathBuf,
    profile: CompatibilityProfileId,
    scenario: String,
    trace: PathBuf,
    iterations: NonZeroU32,
    output: Option<PathBuf>,
    pause_at: Option<String>,
    pause_iteration: Option<u32>,
    maximum_pumps: u64,
    allocator: MeasurementMode,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct PhaseTotals {
    drive_elapsed_ns: u128,
    vm_instructions: u64,
    runtime_transitions: u64,
    envelope_count: usize,
    envelope_bytes: usize,
    snapshot_count: usize,
    delta_count: usize,
    allocator: Option<AllocationStats>,
}

impl PhaseTotals {
    fn observe(&mut self, observation: &PumpObservation) {
        self.drive_elapsed_ns = self.drive_elapsed_ns.saturating_add(observation.elapsed_ns);
        self.vm_instructions = self
            .vm_instructions
            .saturating_add(observation.report.vm_instructions);
        self.runtime_transitions = self
            .runtime_transitions
            .saturating_add(u64::from(observation.report.runtime_transitions));
        self.envelope_count = self
            .envelope_count
            .saturating_add(observation.envelope_count);
        self.envelope_bytes = self
            .envelope_bytes
            .saturating_add(observation.envelope_bytes);
        self.snapshot_count = self
            .snapshot_count
            .saturating_add(observation.snapshot_count);
        self.delta_count = self.delta_count.saturating_add(observation.delta_count);
        self.observe_allocator(observation.allocations);
    }

    fn observe_allocator(&mut self, window: Option<AllocationStats>) {
        if let Some(window) = window {
            let aggregate = self.allocator.get_or_insert_with(AllocationStats::default);
            aggregate.allocations = aggregate.allocations.saturating_add(window.allocations);
            aggregate.deallocations = aggregate.deallocations.saturating_add(window.deallocations);
            aggregate.allocated_bytes = aggregate
                .allocated_bytes
                .saturating_add(window.allocated_bytes);
            aggregate.deallocated_bytes = aggregate
                .deallocated_bytes
                .saturating_add(window.deallocated_bytes);
            aggregate.peak_net_bytes = aggregate.peak_net_bytes.max(window.peak_net_bytes);
            aggregate.net_bytes = aggregate.net_bytes.saturating_add(window.net_bytes);
        }
    }
}

enum DriveTarget<'a> {
    ServerHello,
    AnyOutput,
    ProjectLoad,
    Checkpoint(&'a TraceStep),
}

struct DriveContext<'a> {
    cli: &'a Cli,
    iteration: u32,
    sink: &'a mut JsonlSink,
    watchdog: &'a mut WatchdogThrottle,
}

pub(super) fn run_cli() -> AuditResult<()> {
    let arguments = std::env::args().skip(2).collect::<Vec<_>>();
    let output = output_path_hint(&arguments);
    let mut sink = JsonlSink::new(output.as_deref())?;
    let cli = match parse_cli(arguments) {
        Ok(cli) => cli,
        Err(error) => {
            sink.terminal("failed", json!({"error": error.to_string()}))?;
            return Err(error);
        }
    };
    match run(&cli, &mut sink) {
        Ok(()) => sink.terminal(
            "passed",
            json!({"scenario": cli.scenario, "iterations": cli.iterations.get()}),
        ),
        Err(error) => {
            let terminal = sink.terminal(
                "failed",
                json!({"scenario": cli.scenario, "error": error.to_string()}),
            );
            if let Err(write_error) = terminal {
                return Err(format!(
                    "{error}; additionally failed to flush terminal event: {write_error}"
                )
                .into());
            }
            Err(error)
        }
    }
}

fn output_path_hint(arguments: &[String]) -> Option<PathBuf> {
    let positions = arguments
        .iter()
        .enumerate()
        .filter(|(_, argument)| argument.as_str() == "--output")
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if positions.len() != 1 {
        return None;
    }
    arguments
        .get(positions[0].saturating_add(1))
        .filter(|value| !value.starts_with("--"))
        .map(PathBuf::from)
}

fn run(cli: &Cli, sink: &mut JsonlSink) -> AuditResult<()> {
    input::validate_isolated_project(&cli.project)?;
    let trace = trace::load(&cli.trace, cli)?;
    let mut watchdog = WatchdogThrottle::new();
    let calibration = allocator::calibrate(CALIBRATION_SAMPLES);
    let input_started = Instant::now();
    let (prepared, input_allocations) = allocator::measure(cli.allocator, || {
        input::prepare(&cli.project, &mut |path, completed| {
            allocator::without_counting(|| {
                super::watchdog::publish_or_exit(json!({
                    "phase": "input_preparation", "scenario": cli.scenario,
                    "path": path, "completed": completed, "lastFullResponse": null
                }));
            });
        })
    });
    let prepared = prepared?;
    sink.emit(
        "phase",
        json!({
            "period": "loading", "stage": "input_preparation",
            "wallElapsedNs": input_started.elapsed().as_nanos(),
            "allocator": input_allocations, "files": prepared.input_count,
            "sourceBytes": prepared.source_bytes, "resourceBytes": prepared.resource_bytes
        }),
    )?;
    if prepared.digest != trace.project_digest {
        return Err(format!(
            "project digest mismatch: trace={} actual={}",
            trace.project_digest, prepared.digest
        )
        .into());
    }
    let manifest_payload_bytes = RuntimeMessage::ProjectManifest(prepared.manifest.clone())
        .encode_payload()?
        .len();
    let probe = PerfSession::new();
    if manifest_payload_bytes > probe.wire_limits.maximum_payload_bytes {
        return Err(format!(
            "encoded manifest is {manifest_payload_bytes} bytes, exceeding creator payload limit {}",
            probe.wire_limits.maximum_payload_bytes
        )
        .into());
    }
    sink.emit("metadata", json!({
        "scenario": cli.scenario, "traceDigest": trace.trace_digest,
        "traceSchemaVersion": trace.schema_version, "project": cli.project,
        "projectDigest": prepared.digest, "profile": cli.profile.to_string(),
        "iterations": cli.iterations.get(), "seed": trace.seed,
        "runtimeProtocol": RUNTIME_PROTOCOL_VERSION, "pid": std::process::id(),
        "allocatorMode": allocator_name(cli.allocator), "allocatorCalibration": calibration,
        "input": {"files": prepared.input_count, "sourceBytes": prepared.source_bytes,
            "resourceBytes": prepared.resource_bytes, "manifestPayloadBytes": manifest_payload_bytes},
        "requestedLimits": probe.requested_limits,
        "creatorWireLimits": wire_limits_json(probe.wire_limits)
    }))?;
    watchdog.publish(json!({"phase": "prepared", "scenario": cli.scenario}), true)?;
    for iteration in 0..cli.iterations.get() {
        run_iteration(cli, &trace, &prepared, iteration, sink, &mut watchdog)?;
    }
    Ok(())
}

fn run_iteration(
    cli: &Cli,
    trace: &PerfTrace,
    prepared: &PreparedProject,
    iteration: u32,
    sink: &mut JsonlSink,
    watchdog: &mut WatchdogThrottle,
) -> AuditResult<()> {
    sink.emit("iterationBegin", json!({"iteration": iteration}))?;
    let started = Instant::now();
    let mut session = PerfSession::new();
    let scenario = cli.scenario.clone();
    session
        .runtime
        .set_project_progress_reporter(Some(ProjectProgressReporter::new(move |progress| {
            super::watchdog::publish_or_exit(json!({
                "phase": {"projectStage": progress.stage}, "scenario": scenario,
                "iteration": iteration, "projectProgress": progress,
                "lastFullResponse": null
            }));
        })));
    session.send(RuntimeMessage::ClientHello(ClientHello {
        runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
        client_name: "runtime-perf-audit".into(),
        configuration_profile: Some(ConfigurationClientProfile::Tauri),
        features: trace.client.features.clone(),
        requested_limits: session.requested_limits,
        capabilities: trace.client.capabilities.clone(),
        preferred_locales: vec!["ja".into()],
    }))?;
    let mut drive = DriveContext {
        cli,
        iteration,
        sink,
        watchdog,
    };
    drive_until(
        &mut session,
        "handshake",
        DriveTarget::ServerHello,
        &mut drive,
    )?;
    for setup in &trace.setup_messages {
        session.send(setup.clone())?;
        drive_until(&mut session, "setup", DriveTarget::AnyOutput, &mut drive)?;
    }
    session.send(RuntimeMessage::ProjectManifest(prepared.manifest.clone()))?;
    let load_messages = drive_until(
        &mut session,
        "project_load",
        DriveTarget::ProjectLoad,
        &mut drive,
    )?;
    validate_load(&load_messages)?;
    session.send(RuntimeMessage::Start(StartRequest {
        mode: StartMode::NewGame {
            seed: Some(trace.seed),
        },
    }))?;

    for (step_index, step) in trace.steps.iter().enumerate() {
        let stage = format!("step:{step_index}:{}", step.id);
        let messages = drive_until(
            &mut session,
            &stage,
            DriveTarget::Checkpoint(step),
            &mut drive,
        )?;
        let variables = session.validate_variables(&step.expect.variables)?;
        let normalized = session.normalized_state(&messages, &variables)?;
        let signature = trace::canonical_digest(&normalized)?;
        if signature != step.expect.state_signature {
            return Err(format!(
                "checkpoint {} signature mismatch: expected={} actual={signature}",
                step.checkpoint, step.expect.state_signature
            )
            .into());
        }
        drive.sink.emit(
            "checkpoint",
            json!({
                "iteration": iteration, "step": step_index, "id": step.id,
                "checkpoint": step.checkpoint, "stateSignature": signature,
                "normalizedState": normalized, "presentation": session.metrics(),
                "rssBytes": rss_bytes()
            }),
        )?;
        drive.watchdog.publish(
            json!({
                "phase": "checkpoint", "scenario": cli.scenario, "iteration": iteration,
                "completed": step_index + 1, "total": trace.steps.len(),
                "checkpoint": step.checkpoint, "normalizedState": normalized
            }),
            true,
        )?;
        if should_pause(cli, iteration, &step.checkpoint) {
            pause_for_profiler(cli, iteration, step, drive.sink)?;
        }
        session.apply_action(
            &step.action,
            &trace.protocol_results,
            &messages,
            step_index as u64,
        )?;
    }
    drive.sink.emit(
        "iterationEnd",
        json!({
            "iteration": iteration, "elapsedNs": started.elapsed().as_nanos(),
            "phase": session.phase(), "presentation": session.metrics(),
            "rssBytes": rss_bytes(), "status": "passed"
        }),
    )?;
    Ok(())
}

fn drive_until(
    session: &mut PerfSession,
    stage: &str,
    target: DriveTarget<'_>,
    context: &mut DriveContext<'_>,
) -> AuditResult<Vec<RuntimeMessage>> {
    let started = Instant::now();
    let mut messages = Vec::new();
    let mut totals = PhaseTotals::default();
    for pump in 0..context.cli.maximum_pumps {
        let observation = session.pump(context.cli.allocator)?;
        totals.observe(&observation);
        emit_pump(
            context.sink,
            context.iteration,
            stage,
            pump,
            session,
            &observation,
        )?;
        messages.extend(observation.messages);
        let reached = target_reached(session, &messages, &target);
        context.watchdog.publish_with(reached, || {
            let watchdog_state = session.normalized_state(&messages, &Default::default())?;
            Ok(json!({
                "phase": "drive", "scenario": context.cli.scenario,
                "iteration": context.iteration,
                "stage": stage, "runtime": watchdog_state
            }))
        })?;
        if reached {
            context.sink.emit(
                "phase",
                json!({
                    "iteration": context.iteration, "stage": stage, "totals": totals,
                    "period": stage_period(stage),
                    "wallElapsedNs": started.elapsed().as_nanos(),
                    "phase": session.phase(), "rssBytes": rss_bytes()
                }),
            )?;
            return Ok(messages);
        }
        if is_terminal_drift(observation.report.state) {
            return Err(format!(
                "{stage} drifted before its target: driveState={} phase={:?}",
                drive_state_name(observation.report.state),
                session.phase()
            )
            .into());
        }
    }
    Err(format!("{stage} exceeded {} pumps", context.cli.maximum_pumps).into())
}

fn stage_period(stage: &str) -> &'static str {
    if matches!(stage, "handshake" | "setup" | "project_load") {
        "loading"
    } else {
        "runtime"
    }
}

fn target_reached(
    session: &mut PerfSession,
    messages: &[RuntimeMessage],
    target: &DriveTarget<'_>,
) -> bool {
    match target {
        DriveTarget::ServerHello => {
            session.has_session()
                && messages
                    .iter()
                    .any(|message| matches!(message, RuntimeMessage::ServerHello(_)))
        }
        DriveTarget::AnyOutput => !messages.is_empty(),
        DriveTarget::ProjectLoad => messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::ProjectLoadReport(_))),
        DriveTarget::Checkpoint(step) => session.checkpoint_matches(messages, &step.expect),
    }
}

const fn is_terminal_drift(state: RuntimeDriveState) -> bool {
    matches!(
        state,
        RuntimeDriveState::Idle | RuntimeDriveState::Stopped | RuntimeDriveState::Faulted
    )
}

fn emit_pump(
    sink: &mut JsonlSink,
    iteration: u32,
    stage: &str,
    pump: u64,
    session: &PerfSession,
    observation: &PumpObservation,
) -> AuditResult<()> {
    sink.emit("pump", json!({
        "iteration": iteration, "stage": stage, "pump": pump,
        "period": stage_period(stage),
        "elapsedNs": observation.elapsed_ns, "phase": session.phase(),
        "driveState": drive_state_name(observation.report.state),
        "vmInstructions": observation.report.vm_instructions,
        "runtimeTransitions": observation.report.runtime_transitions,
        "queuedEnvelopes": observation.report.queued_envelopes,
        "cooperativeBackgroundWork": observation.report.cooperative_background_work,
        "envelopes": {"count": observation.envelope_count, "bytes": observation.envelope_bytes},
        "presentationMessages": {"snapshots": observation.snapshot_count, "deltas": observation.delta_count},
        "presentation": session.metrics(), "allocator": observation.allocations
    }))
}

fn validate_load(messages: &[RuntimeMessage]) -> AuditResult<()> {
    let report = messages
        .iter()
        .find_map(|message| match message {
            RuntimeMessage::ProjectLoadReport(report) => Some(report),
            _ => None,
        })
        .ok_or("project load report is absent")?;
    if !report.success
        || report.compatibility.as_ref()
            != Some(&erabasic_compat::CompatibilityIdentity::for_profile(
                SNAKE_PROFILE,
            ))
        || report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.level == RuntimeLogLevel::Error)
    {
        return Err(
            format!("snake project load failed or resolved wrong identity: {report:?}").into(),
        );
    }
    Ok(())
}

fn should_pause(cli: &Cli, iteration: u32, checkpoint: &str) -> bool {
    cli.pause_at.as_deref() == Some(checkpoint)
        && cli
            .pause_iteration
            .is_none_or(|selected| selected == iteration)
}

fn pause_for_profiler(
    cli: &Cli,
    iteration: u32,
    step: &TraceStep,
    sink: &mut JsonlSink,
) -> AuditResult<()> {
    sink.emit(
        "profilerPause",
        json!({
            "iteration": iteration, "scenario": cli.scenario, "checkpoint": step.checkpoint,
            "pid": std::process::id(), "resume": "write one line to stdin"
        }),
    )?;
    sink.flush()?;
    super::watchdog::publish(json!({
        "phase": "profiler_pause", "scenario": cli.scenario, "iteration": iteration,
        "checkpoint": step.checkpoint, "pid": std::process::id(), "lastFullResponse": null
    }))?;
    let mut line = String::new();
    resume_profiler(std::io::stdin().lock(), &mut line)
}

fn resume_profiler(mut reader: impl BufRead, line: &mut String) -> AuditResult<()> {
    if reader.read_line(line)? == 0 {
        return Err("profiler pause reached EOF before an explicit resume line".into());
    }
    Ok(())
}

fn parse_cli(arguments: impl IntoIterator<Item = String>) -> AuditResult<Cli> {
    let mut project = None;
    let mut profile = None;
    let mut scenario = None;
    let mut trace = None;
    let mut iterations = None;
    let mut output = None;
    let mut pause_at = None;
    let mut pause_iteration = None;
    let mut maximum_pumps = DEFAULT_MAX_PUMPS;
    let mut allocator = None;
    let mut seen = BTreeSet::new();
    let mut arguments = arguments.into_iter();
    while let Some(option) = arguments.next() {
        if !seen.insert(option.clone()) {
            return Err(format!("duplicate perf-run option {option}").into());
        }
        if !matches!(
            option.as_str(),
            "--project"
                | "--profile"
                | "--scenario"
                | "--trace"
                | "--iterations"
                | "--output"
                | "--pause-at"
                | "--pause-iteration"
                | "--maximum-pumps"
                | "--allocator"
        ) {
            return Err(format!("unknown perf-run option {option}").into());
        }
        let value = arguments
            .next()
            .ok_or_else(|| format!("{option} requires a value"))?;
        if value.starts_with("--") {
            return Err(format!("{option} requires a value, got option {value}").into());
        }
        match option.as_str() {
            "--project" => project = Some(PathBuf::from(value)),
            "--profile" => profile = Some(value.parse::<CompatibilityProfileId>()?),
            "--scenario" => scenario = Some(value),
            "--trace" => trace = Some(PathBuf::from(value)),
            "--iterations" => iterations = NonZeroU32::new(value.parse()?),
            "--output" => output = Some(PathBuf::from(value)),
            "--pause-at" => pause_at = Some(value),
            "--pause-iteration" => pause_iteration = Some(value.parse()?),
            "--maximum-pumps" => maximum_pumps = value.parse()?,
            "--allocator" => {
                allocator = Some(match value.as_str() {
                    "counting" => MeasurementMode::Counting,
                    "off" => MeasurementMode::Off,
                    _ => return Err("--allocator must be counting or off".into()),
                })
            }
            _ => unreachable!("known options were checked before consuming the value"),
        }
    }
    let profile = profile.ok_or("perf-run requires --profile emuera.skia.snake")?;
    if profile != SNAKE_PROFILE {
        return Err("perf-run only accepts --profile emuera.skia.snake".into());
    }
    let iterations = iterations.ok_or("perf-run requires positive --iterations")?;
    if iterations.get() > MAX_ITERATIONS {
        return Err(format!("--iterations must not exceed {MAX_ITERATIONS}").into());
    }
    let scenario = scenario.ok_or("perf-run requires --scenario")?;
    if scenario.is_empty()
        || scenario.len() > 128
        || !scenario
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err("--scenario must be a non-empty ASCII identifier".into());
    }
    if maximum_pumps == 0 {
        return Err("--maximum-pumps must be positive".into());
    }
    if pause_iteration.is_some() && pause_at.is_none() {
        return Err("--pause-iteration requires --pause-at".into());
    }
    if pause_at.is_some() && iterations.get() > 1 && pause_iteration.is_none() {
        return Err("multi-iteration pause requires --pause-iteration".into());
    }
    if pause_iteration.is_some_and(|selected| selected >= iterations.get()) {
        return Err("--pause-iteration must be less than --iterations".into());
    }
    Ok(Cli {
        project: project.ok_or("perf-run requires --project with an isolated project copy")?,
        profile,
        scenario,
        trace: trace.ok_or("perf-run requires --trace")?,
        iterations,
        output,
        pause_at,
        pause_iteration,
        maximum_pumps,
        allocator: allocator.ok_or("perf-run requires --allocator counting|off")?,
    })
}

const fn allocator_name(mode: MeasurementMode) -> &'static str {
    match mode {
        MeasurementMode::Counting => "counting",
        MeasurementMode::Off => "off",
    }
}

fn wire_limits_json(limits: WireLimits) -> serde_json::Value {
    json!({
        "maximumEnvelopeBytes": limits.maximum_envelope_bytes,
        "maximumPayloadBytes": limits.maximum_payload_bytes,
    })
}

fn rss_bytes() -> Option<u64> {
    let output = std::process::Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(|kib| kib.saturating_mul(1024))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> AuditResult<Cli> {
        parse_cli(arguments.iter().map(|argument| (*argument).to_owned()))
    }

    #[test]
    fn cli_requires_explicit_snake_identity_and_allocator() -> AuditResult<()> {
        let valid = [
            "--project",
            "copy",
            "--profile",
            "emuera.skia.snake",
            "--scenario",
            "fixture",
            "--trace",
            "trace.json",
            "--iterations",
            "1",
            "--allocator",
            "off",
        ];
        assert_eq!(parse(&valid)?.allocator, MeasurementMode::Off);
        let mut wrong = valid;
        wrong[3] = "emuera.em";
        assert!(parse(&wrong).is_err());
        assert!(parse(&valid[..valid.len() - 2]).is_err());
        Ok(())
    }

    #[test]
    fn pause_is_bounded_to_one_iteration() {
        let base = [
            "--project",
            "copy",
            "--profile",
            "emuera.skia.snake",
            "--scenario",
            "fixture",
            "--trace",
            "trace.json",
            "--iterations",
            "2",
            "--allocator",
            "off",
            "--pause-at",
            "ready",
        ];
        assert!(parse(&base).is_err());
        let mut selected = base.to_vec();
        selected.extend(["--pause-iteration", "1"]);
        assert!(parse(&selected).is_ok());
    }

    #[test]
    fn cli_rejects_duplicate_options_and_option_tokens_as_values() {
        let duplicate = [
            "--project",
            "copy",
            "--project",
            "other",
            "--profile",
            "emuera.skia.snake",
            "--scenario",
            "fixture",
            "--trace",
            "trace.json",
            "--iterations",
            "1",
            "--allocator",
            "off",
        ];
        assert!(parse(&duplicate).is_err());
        let missing = ["--project", "--profile", "emuera.skia.snake"];
        assert!(parse(&missing).is_err());
    }

    #[test]
    fn idle_is_immediate_trace_drift() {
        assert!(is_terminal_drift(RuntimeDriveState::Idle));
        assert!(is_terminal_drift(RuntimeDriveState::Stopped));
        assert!(!is_terminal_drift(RuntimeDriveState::MoreWork));
    }

    #[test]
    fn profiler_pause_requires_an_explicit_line_not_eof() {
        let mut line = String::new();
        assert!(resume_profiler(std::io::Cursor::new(Vec::<u8>::new()), &mut line).is_err());
        assert!(resume_profiler(std::io::Cursor::new(b"resume\n"), &mut line).is_ok());
    }

    #[test]
    fn creator_wire_limits_use_the_metadata_schema() {
        assert_eq!(
            wire_limits_json(WireLimits {
                maximum_envelope_bytes: 128,
                maximum_payload_bytes: 127,
            }),
            json!({"maximumEnvelopeBytes": 128, "maximumPayloadBytes": 127})
        );
    }

    #[test]
    fn phase_allocator_sums_signed_net_and_keeps_largest_window_peak() {
        let mut totals = PhaseTotals::default();
        totals.observe_allocator(Some(AllocationStats {
            allocations: 2,
            deallocations: 1,
            allocated_bytes: 30,
            deallocated_bytes: 10,
            net_bytes: 20,
            peak_net_bytes: 25,
        }));
        totals.observe_allocator(Some(AllocationStats {
            allocations: 1,
            deallocations: 2,
            allocated_bytes: 5,
            deallocated_bytes: 13,
            net_bytes: -8,
            peak_net_bytes: 5,
        }));
        let aggregate = totals.allocator.unwrap();
        assert_eq!(aggregate.net_bytes, 12);
        assert_eq!(aggregate.peak_net_bytes, 25);
        assert_eq!(aggregate.allocations, 3);
        assert_eq!(aggregate.deallocations, 3);
    }
}
