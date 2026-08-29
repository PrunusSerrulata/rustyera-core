//! Actual observations of the shared snake compatibility fixture; expected answers are never read.
//! Each case owns a session. A synthetic title calls the unchanged entry/argument expression,
//! then waits after a completion marker so the authorized debugger can inspect its memory.

use era_debug_protocol::DebugCommand;
use era_runtime_protocol::{
    AdvanceTime, DisplayLine, FrontendInput, InputIntent, RuntimeMessage, RuntimePhase, StartMode,
    StartRequest, WaitKind,
};
use erabasic_compat::{CompatibilityIdentity, CompatibilityProfileId};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
mod fixture;
mod input;
mod session;
mod storage;
use fixture::{Case, Fixture, build_manifest, fixture_identity, git_output};
use session::ObservationSession;
type AuditResult<T> = Result<T, Box<dyn Error>>;
const COMPLETE: &str = "__RUSTYERA_COMPAT_OBSERVATION_COMPLETE__";

pub fn run_cli() -> AuditResult<()> {
    let mut fixture_root = super::tool_root().join("fixture-snake-compatibility");
    let mut profile = None;
    let mut output = None;
    let mut selected_cases = Vec::new();
    let mut arguments = std::env::args().skip(2);
    while let Some(argument) = arguments.next() {
        let value = arguments.next().ok_or("option requires a value")?;
        match argument.as_str() {
            "--profile" => profile = Some(value.parse::<CompatibilityProfileId>()?),
            "--fixture" => fixture_root = PathBuf::from(value),
            "--output" => output = Some(PathBuf::from(value)),
            "--case" => selected_cases.push(value),
            _ => return Err(format!("unknown snake-observations option {argument}").into()),
        }
    }
    let profile = profile.ok_or("snake-observations requires --profile")?;
    let fixture: Fixture = serde_json::from_slice(&fs::read(fixture_root.join("cases.json"))?)?;
    if fixture.version != 1 {
        return Err("unsupported fixture version".into());
    }
    let identity = CompatibilityIdentity::for_profile(profile);
    let source_fixture = fixture_identity(&fixture_root)?;
    let core_sha = git_output(&["rev-parse", "HEAD"])?;
    if core_sha.len() != 40 || !core_sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("core HEAD is not a full SHA".into());
    }
    let dirty = !git_output(&["status", "--porcelain", "--untracked-files=all"])?.is_empty();
    let mut cases = Vec::new();
    for case in fixture.cases {
        if !selected_cases.is_empty()
            && !selected_cases
                .iter()
                .any(|selection| selection == &case.id || selection == &case.group)
        {
            continue;
        }
        // A failed group still emits its real load diagnostic; subsequent groups get fresh VMs.
        cases.push(observe_case(&fixture_root, &identity, fixture.seed, &case)?);
    }
    if cases.is_empty() {
        return Err("no matching observation cases".into());
    }
    if fixture_identity(&fixture_root)? != source_fixture {
        return Err("fixture changed while collecting observations".into());
    }
    let report = json!({
        "version": 1, "coreSha": core_sha, "dirty": dirty, "profile": identity,
        "seed": fixture.seed, "sourceFixture": source_fixture, "cases": cases,
        "selectedCases": selected_cases,
        "harness": {
            "version": 1,
            "runEngine": "runtime_session_compiled_call",
            "evalEngine": "runtime_compiled_expression",
            "watches": "authorized_debug_list_variables_and_read_variable",
            "completion": "synthetic title marker followed by INPUT; original entry returned",
            "presentation": "runtime logical lines, no pixel renderer or font measurement",
            "keyInput": "primitive trace to get_key_state v1 pressed/toggle/frontend_active",
            "supervision": "parent process, complete state every five seconds, identical state terminates"
        }
    });
    let encoded = serde_json::to_string_pretty(&report)?;
    if let Some(path) = output {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, format!("{encoded}\n"))?;
    } else {
        println!("{encoded}");
    }
    Ok(())
}

fn observe_case(
    root: &Path,
    identity: &CompatibilityIdentity,
    seed: u64,
    case: &Case,
) -> AuditResult<Value> {
    if case.requests.is_empty() {
        return Ok(json!({"id": case.id, "group": case.group, "steps": [],
            "status": "blocked", "reason": "fixture assertions require reference input atomicity/reset commands; runtime protocol has no equivalent input trace transaction/reset endpoint"}));
    }
    let (manifest, harness) =
        build_manifest(root, identity, &case.group, &case.requests[0].request)?;
    let storage = if case.group == "COLUMNS" {
        Some(storage::FixtureStorage::from_manifest(&manifest)?)
    } else {
        None
    };
    let mut runtime =
        ObservationSession::new(&case.id, &case.group, &case.requests[0].request, storage)?;
    runtime.send(RuntimeMessage::ProjectManifest(manifest))?;
    if let Err(error) = runtime.pump_until(|runtime| runtime.load.is_some()) {
        runtime.blocked = Some(format!("project loading did not produce a report: {error}"));
    }
    if runtime
        .load
        .as_ref()
        .is_some_and(|report| report.success && report.compatibility.as_ref() != Some(identity))
    {
        runtime.blocked =
            Some("loaded compatibility identity differs from the requested profile".into());
    }
    let mut steps = Vec::new();
    runtime.setup_host_logs = std::mem::take(&mut runtime.host_logs);
    for (index, step) in case.requests.iter().enumerate() {
        let observation = if let Some(reason) = &runtime.blocked {
            json!({"request": step.request, "status": "blocked", "reason": reason,
                "result": {"ok": false, "termination": "blocked", "output": [], "watches": {},
                    "diagnostics": runtime.diagnostics, "hostLogs": runtime.host_logs, "presentation": runtime.lines}})
        } else if !runtime.load.as_ref().is_some_and(|report| report.success) {
            json!({"request": step.request, "status": "executed", "result": {
                "ok": false, "termination": "compileError", "output": [], "watches": {},
                "diagnostics": runtime.setup_diagnostics.iter().filter(|diagnostic| diagnostic["source"].is_object()).collect::<Vec<_>>(), "presentation": []
            }})
        } else {
            match observe_step(&mut runtime, &step.request, seed, index == 0) {
                Ok(observation) => observation,
                Err(error) => {
                    json!({"request": step.request, "status": "blocked", "reason": error.to_string(),
                    "result": {"ok": false, "termination": "blocked", "output": [], "watches": {},
                        "diagnostics": runtime.diagnostics, "hostLogs": runtime.host_logs, "presentation": runtime.lines}})
                }
            }
        };
        steps.push(observation);
    }
    let mut observation = json!({"id": case.id, "group": case.group, "steps": steps, "harness": harness, "load": runtime.load, "setupDiagnostics": runtime.setup_diagnostics,
        "setupHostLogs": runtime.setup_host_logs, "rawMessages": runtime.raw_messages});
    if let Some(evidence) = runtime.storage_evidence {
        observation["storageEvidence"] = json!(evidence);
        observation["storageBackend"] =
            json!("owned-fixture-memory; not a frontend host validation");
    }
    Ok(observation)
}

fn observe_step(
    runtime: &mut ObservationSession,
    request: &Value,
    seed: u64,
    initial: bool,
) -> AuditResult<Value> {
    runtime.begin_step(request, initial)?;
    let mut blocks = Vec::new();
    let mut required_observation_blocked = false;
    if let Err(error) = runtime.install_input_trace(&request["inputTrace"]) {
        return Ok(
            json!({"request": request, "status": "blocked", "reason": error.to_string(),
            "result": {"ok": false, "termination": "blocked", "output": [], "watches": {}, "diagnostics": runtime.diagnostics, "hostLogs": runtime.host_logs}}),
        );
    }
    if initial {
        runtime.send(RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(seed) },
        }))?;
    }
    let mut inputs: VecDeque<String> = request["inputs"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or("input must be text")
        })
        .collect::<Result<_, _>>()?;
    let termination = loop {
        runtime.pump()?;
        if let Some(reason) = &runtime.blocked {
            blocks.push(reason.clone());
            break "blocked";
        }
        if runtime.session.phase() == RuntimePhase::Faulted {
            break "faulted";
        }
        if runtime.completed() {
            break "completed";
        }
        if let Some(wait) = runtime.wait.clone() {
            if wait.kind == WaitKind::Void {
                if let Some(events) = runtime.await_pumps.pop_front() {
                    runtime.apply_key_events(&events)?;
                    runtime.clock = runtime.clock.saturating_add(1_000_000);
                    runtime.send(RuntimeMessage::AdvanceTime(AdvanceTime {
                        monotonic_time_ns: runtime.clock,
                    }))?;
                } else {
                    blocks.push("AWAIT requires another declared awaitPumps batch".into());
                    break "blocked";
                }
            } else if let Some(input) = inputs.pop_front() {
                runtime.clock = runtime.clock.saturating_add(1);
                runtime.send(RuntimeMessage::Input(FrontendInput {
                    wait_id: wait.wait_id,
                    token: wait.submission_token,
                    monotonic_time_ns: runtime.clock,
                    intent: InputIntent::CommitText(input),
                    message_skip: false,
                }))?;
                runtime.wait = None;
            } else {
                break "waitingInput";
            }
        }
    };
    let mut watches = serde_json::Map::new();
    let mut value = Value::Null;
    // Fault stops permit authorized read-only inspection without resuming the script.
    if matches!(termination, "completed" | "waitingInput" | "faulted") {
        match runtime.pause() {
            Ok((grant, stop)) => {
                for watch in request["watch"].as_array().into_iter().flatten() {
                    let name = watch.as_str().ok_or("watch must be a string")?;
                    match runtime.read_watch(grant, stop, name) {
                        Ok(result) => {
                            watches.insert(name.into(), result);
                        }
                        Err(error) => {
                            required_observation_blocked = true;
                            blocks.push(format!("watch {name}: {error}"));
                        }
                    }
                }
                if request["op"] == "eval" && termination != "faulted" {
                    match runtime.read_watch(grant, stop, "RESULT:0") {
                        Ok(result) => value = result,
                        Err(error) => {
                            required_observation_blocked = true;
                            blocks.push(format!("expression result unavailable: {error}"));
                        }
                    }
                }
                runtime.debug_command(grant, DebugCommand::Continue { stop })?;
            }
            Err(error) => {
                required_observation_blocked = true;
                blocks.push(format!("debug observation unavailable: {error}"));
            }
        }
    } else if request["watch"]
        .as_array()
        .is_some_and(|watches| !watches.is_empty())
    {
        blocks.push("runtime debugger cannot pause a faulted/blocked VM to inspect watches".into());
    }
    if request["observePresentation"] == true {
        blocks.push("pixel/font glyph rectangles unavailable: no frontend renderer or measurement endpoint was negotiated; logical presentation recorded".into());
    }
    if !inputs.is_empty() || !runtime.await_pumps.is_empty() {
        required_observation_blocked = true;
        blocks.push("execution ended before consuming the complete declared input trace".into());
    }
    let lines: Vec<_> = runtime
        .lines
        .iter()
        .filter(|line| !line_text(line).contains(COMPLETE))
        .cloned()
        .collect();
    let output = lines.iter().map(line_text).collect::<Vec<_>>();
    let blocked = termination == "blocked" || required_observation_blocked;
    let mut observation = json!({"request": request, "status": if blocked { "blocked" } else { "executed" },
        "reason": if blocked { Some(blocks.join("; ")) } else { None },
        "result": {"ok": matches!(termination, "completed" | "waitingInput"), "termination": termination,
            "value": value, "watches": watches, "output": output, "diagnostics": runtime.diagnostics,
            "hostLogs": runtime.host_logs, "presentation": lines, "observationBlocks": blocks, "inputEvidence": runtime.input_evidence,
            "displayState": {"settings": runtime.settings, "resources": runtime.resources},
            "inputObservation": if inputs.is_empty() && runtime.await_pumps.is_empty() { "consumed" } else { "blocked" },
            "runtimePhase": runtime.session.phase(), "instructions": runtime.instructions}});
    if let Some(evidence) = &runtime.storage_evidence {
        observation["result"]["storageEvidence"] = json!(evidence);
    }
    Ok(observation)
}

fn line_text(line: &DisplayLine) -> String {
    fn text(run: &era_runtime_protocol::DisplayRun) -> String {
        use era_runtime_protocol::DisplayRun;
        match run {
            DisplayRun::Text { text, .. } | DisplayRun::TextLayout { text, .. } => text.clone(),
            DisplayRun::Button { runs, .. } => runs.iter().map(text).collect(),
            DisplayRun::ColumnCell { content, .. } => content.iter().map(text).collect(),
            DisplayRun::Separator { pattern, .. } => pattern.clone(),
            DisplayRun::Space { .. } => " ".into(),
            DisplayRun::HtmlDocument { .. }
            | DisplayRun::Image { .. }
            | DisplayRun::Shape { .. } => String::new(),
        }
    }
    line.runs.iter().map(text).collect()
}
