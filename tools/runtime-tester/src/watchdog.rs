//! A separate parent process supervises blocking audit work; stdout remains the result channel.

use std::{
    error::Error,
    fs, io,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

const CHILD_SNAPSHOT: &str = "ERA_AUDIT_CHILD_SNAPSHOT";
const INTERVAL: Duration = Duration::from_secs(5);
static PUBLISH_LOCK: Mutex<()> = Mutex::new(());

/// Replace the complete observed state. No-op outside a supervised CLI execution.
pub(super) fn publish(state: Value) -> io::Result<()> {
    let Some(path) = std::env::var_os(CHILD_SNAPSHOT).map(PathBuf::from) else {
        return Ok(());
    };
    let _lock = PUBLISH_LOCK
        .lock()
        .map_err(|_| io::Error::other("watchdog snapshot lock poisoned"))?;
    let temporary = path.with_extension("next");
    fs::write(&temporary, serde_json::to_vec(&state)?)?;
    fs::rename(temporary, path)
}

/// Use this from callbacks that cannot return an I/O error. Losing observation is fatal.
pub(super) fn publish_or_exit(state: Value) {
    if let Err(error) = publish(state) {
        eprintln!("audit observation failed: {error}");
        std::process::exit(2);
    }
}

#[derive(Default)]
struct Comparison {
    previous: Option<Value>,
    identical_samples: usize,
}

impl Comparison {
    fn sample(&mut self, state: &Value) -> bool {
        let state = comparison_state(state);
        let unchanged = self.previous.as_ref() == Some(&state);
        self.identical_samples = if unchanged {
            self.identical_samples.saturating_add(1)
        } else {
            1
        };
        let stalled = self.identical_samples >= required_identical_samples(&state);
        self.previous = Some(state);
        stalled
    }
}

fn required_identical_samples(state: &Value) -> usize {
    let observed = state.get("observed").unwrap_or(state);
    let phase = &observed["phase"];
    // Only explicit project-loading stages receive the user-authorized grace.
    // Report assembly/writes, execution and input waits retain the strict rule.
    let loading = matches!(
        phase.as_str(),
        Some(
            "inventory_directory"
                | "hash_file"
                | "coverage_read_input"
                | "coverage_hash_input"
                | "appearance_parse"
                | "appearance_sort"
                | "appearance_sort_complete"
                | "csv_load"
                | "analysis"
                | "analysis_returned"
                | "hir_validation"
                | "coverage_symbol_projection"
                | "compile"
                | "loading_project"
                | "reloading"
        )
    ) || phase.get("projectStage").is_some_and(|stage| {
        serde_json::from_value::<era_runtime::ProjectProgressStage>(stage.clone()).is_ok()
    });
    if loading { 4 } else { 2 }
}

fn comparison_state(state: &Value) -> Value {
    let mut state = state.clone();
    let observed = if state.get("observed").is_some() {
        &mut state["observed"]
    } else {
        &mut state
    };
    if let Some(object) = observed.as_object_mut() {
        object.remove("reportMetadata");
        for key in ["request", "pending", "lastFullResponse"] {
            if let Some(value) = object.get_mut(key) {
                normalize_protocol_metadata(value);
            }
        }
    }
    state
}

/// Visit known protocol boundaries only. Never walk result/watches/script dictionaries.
fn normalize_protocol_metadata(value: &mut Value) {
    if let Some(items) = value.as_array_mut() {
        for item in items {
            normalize_protocol_metadata(item);
        }
        return;
    }
    let Some(object) = value.as_object_mut() else {
        return;
    };
    for name in [
        "id",
        "request_id",
        "requestId",
        "sequence",
        "acknowledged_sequence",
    ] {
        object.remove(name);
    }
    // ObservationSession's last response is {channel, message}; pending entries are
    // typed protocol messages. Future raw envelope inclusion uses an explicit key.
    for key in ["message", "envelope"] {
        if let Some(nested) = object.get_mut(key) {
            normalize_protocol_metadata(nested);
        }
    }
    if object.contains_key("type")
        && let Some(payload) = object.get_mut("value").and_then(Value::as_object_mut)
    {
        for name in ["request_id", "requestId", "acknowledged_sequence"] {
            payload.remove(name);
        }
    }
}

struct ProcessGuard(Child, bool);

impl ProcessGuard {
    fn stop(&mut self) {
        if self.1 {
            return;
        }
        self.1 = true;
        #[cfg(unix)]
        {
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{}", self.0.id())])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        #[cfg(windows)]
        {
            let _ = Command::new("taskkill")
                .args(["/PID", &self.0.id().to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Drop for ProcessGuard {
    fn drop(&mut self) {
        self.stop();
    }
}

fn observed(path: &Path, command: &str, process_id: u32) -> io::Result<Value> {
    let state = match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            json!({"phase": "starting", "case": command, "pending": command, "lastFullResponse": null})
        }
        Err(error) => return Err(error),
    };
    // These are actual process properties. Sampling time and poll counts are absent.
    Ok(
        json!({"command": command, "process": {"pid": process_id, "state": "running"}, "observed": state}),
    )
}

pub(super) fn supervise(
    command: &str,
    run: fn() -> Result<(), Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    if std::env::var_os(CHILD_SNAPSHOT).is_some() {
        publish(
            json!({"case": command, "phase": "started", "pending": null, "lastFullResponse": null}),
        )?;
        return run();
    }
    let seconds = std::env::var("ERA_AUDIT_BUDGET_SECONDS")
        .unwrap_or_else(|_| "3600".into())
        .parse::<f64>()?;
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err("ERA_AUDIT_BUDGET_SECONDS must be finite and positive".into());
    }
    let started = Instant::now();
    let budget = Duration::try_from_secs_f64(seconds)?;
    let directory = std::env::temp_dir().join(format!(
        "rustyera-audit-watchdog-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    fs::create_dir(&directory)?;
    let snapshot = directory.join("state.json");
    let mut child = Command::new(std::env::current_exe()?);
    child
        .args(std::env::args_os().skip(1))
        .env(CHILD_SNAPSHOT, &snapshot)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        child.process_group(0);
    }
    let mut child = ProcessGuard(child.spawn()?, false);
    let mut comparison = Comparison::default();
    let mut next_sample = started + INTERVAL;
    loop {
        if let Some(status) = child.0.try_wait()? {
            let state = fs::read(&snapshot)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
            eprintln!(
                "{}",
                json!({"auditWatchdog": {"command": command, "process": {"pid": child.0.id(), "state": "exited", "code": status.code()}, "observed": state}})
            );
            if status.success() {
                let _ = fs::remove_file(&snapshot);
                let _ = fs::remove_dir(&directory);
                return Ok(());
            }
            return Err(format!(
                "{command} exited {status}; last observation retained at {}",
                snapshot.display()
            )
            .into());
        }
        let now = Instant::now();
        if now.duration_since(started) >= budget {
            let state = observed(&snapshot, command, child.0.id())?;
            eprintln!(
                "{}",
                json!({"auditWatchdog": state, "failure": "wall_clock_budget_exhausted"})
            );
            child.stop();
            return Err(format!(
                "{command} exceeded {seconds}s; observation retained at {}",
                snapshot.display()
            )
            .into());
        }
        if now >= next_sample {
            let state = observed(&snapshot, command, child.0.id())?;
            let stalled = comparison.sample(&state);
            let required = required_identical_samples(&state);
            // Policy counters are report metadata, never observed progress.
            eprintln!(
                "{}",
                json!({"auditWatchdog": state, "watchdogPolicy": {
                "identicalSamples": comparison.identical_samples, "requiredIdenticalSamples": required}})
            );
            if stalled {
                child.stop();
                return Err(format!("{command}: unchanged complete observations at {required} consecutive 5s samples; retained {}", snapshot.display()).into());
            }
            next_sample += INTERVAL;
        }
        thread::sleep(Duration::from_millis(100).min(budget.saturating_sub(started.elapsed())));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_state_fails_without_counting_samples_as_progress() {
        let mut comparison = Comparison::default();
        let state =
            json!({"phase": "running", "pending": "source.erb", "completed": 4, "diagnostics": []});
        assert!(!comparison.sample(&state));
        assert!(comparison.sample(&state));
        let changed =
            json!({"phase": "running", "pending": "source.erb", "completed": 5, "diagnostics": []});
        assert!(!comparison.sample(&changed));
    }

    #[test]
    fn loading_requires_four_identical_samples_and_real_progress_resets_the_run() {
        for phase in [
            json!("inventory_directory"),
            json!("hash_file"),
            json!("coverage_read_input"),
            json!("coverage_hash_input"),
            json!("appearance_parse"),
            json!("appearance_sort"),
            json!("appearance_sort_complete"),
            json!("csv_load"),
            json!("analysis"),
            json!("analysis_returned"),
            json!("coverage_symbol_projection"),
            json!("compile"),
            json!("loading_project"),
            json!("reloading"),
            json!({"projectStage": "finalizing"}),
            json!({"projectStage": "initializing_memory"}),
        ] {
            let mut comparison = Comparison::default();
            let mut state = json!({"observed": {"phase": phase, "completed": 42,
                "pending": {"request_id": 1}}});
            assert!(!comparison.sample(&state));
            state["observed"]["pending"]["request_id"] = json!(2);
            assert!(!comparison.sample(&state));
            assert!(!comparison.sample(&state));
            state["observed"]["completed"] = json!(43);
            assert!(!comparison.sample(&state));
            assert!(!comparison.sample(&state));
            assert!(!comparison.sample(&state));
            assert!(comparison.sample(&state), "phase={phase}");
        }
    }

    #[test]
    fn leaving_loading_restores_two_samples_and_unknown_stages_are_not_exempt() {
        for phase in [
            json!("coverage_prepare_report"),
            json!("coverage_reference_graph"),
            json!("coverage_rows"),
            json!("coverage_write_report"),
            json!("running"),
            json!("waiting_input"),
            json!("waiting_external"),
            json!("starting"),
            json!("unknown"),
            json!({"projectStage": "unknown"}),
        ] {
            let mut comparison = Comparison::default();
            let loading = json!({"phase": "analysis", "completed": 42});
            for _ in 0..3 {
                assert!(!comparison.sample(&loading));
            }
            let state = json!({"phase": phase, "completed": 42});
            assert!(!comparison.sample(&state));
            assert!(comparison.sample(&state), "phase={phase}");
        }
    }

    #[test]
    #[ignore = "real 20-second parent/child watchdog termination check"]
    fn frozen_loading_process_is_stopped_on_the_fourth_sample() {
        fn frozen_loading() -> Result<(), Box<dyn Error>> {
            publish(
                json!({"phase": "coverage_symbol_projection", "pending": "functions",
                "symbols_completed": 112727, "symbols_total": 112727}),
            )?;
            loop {
                thread::park();
            }
        }
        let error = supervise("watchdog_loading_stall", frozen_loading).unwrap_err();
        assert!(error.to_string().contains("at 4 consecutive 5s samples"));
        eprintln!("expected watchdog termination: {error}");
    }

    #[test]
    fn full_response_changes_are_observable() {
        let mut comparison = Comparison::default();
        assert!(
            !comparison
                .sample(&json!({"lastFullResponse": {"output": ["a"], "watches": {"FLAG:0": 1}}}))
        );
        assert!(
            !comparison
                .sample(&json!({"lastFullResponse": {"output": ["a"], "watches": {"FLAG:0": 2}}}))
        );
    }

    #[test]
    fn envelope_ids_do_not_mask_a_stall_or_remove_script_ids() {
        let mut comparison = Comparison::default();
        let first = json!({"observed": {"pending": {"id": 1, "op": "run"}, "lastFullResponse": {"id": 1, "result": {"watches": {"id": 7}}}}});
        let mut second = json!({"observed": {"pending": {"id": 2, "op": "run"}, "lastFullResponse": {"id": 2, "result": {"watches": {"id": 7}}}}});
        assert!(!comparison.sample(&first));
        assert!(comparison.sample(&second));
        assert_eq!(second["observed"]["lastFullResponse"]["id"], 2);
        second["observed"]["lastFullResponse"]["result"]["watches"]["id"] = json!(8);
        assert!(!comparison.sample(&second));
    }

    #[test]
    fn runtime_message_correlation_is_not_observed_progress() {
        let mut comparison = Comparison::default();
        let first = json!({"lastFullResponse": {"channel": "runtime", "message": {"type": "service_response", "value": {"request_id": 1, "result": {"id": 7}}}}});
        let second = json!({"lastFullResponse": {"channel": "runtime", "message": {"type": "service_response", "value": {"request_id": 2, "result": {"id": 7}}}}});
        assert!(!comparison.sample(&first));
        assert!(comparison.sample(&second));
    }

    #[cfg(unix)]
    #[test]
    fn stop_interrupts_a_blocked_worker_process() {
        use std::os::unix::process::CommandExt;
        let mut command = Command::new("sh");
        command.args(["-c", "exec sleep 60"]).process_group(0);
        let mut child = ProcessGuard(command.spawn().expect("spawn blocked worker"), false);
        child.stop();
        assert!(child.0.try_wait().expect("read worker exit").is_some());
    }
}
