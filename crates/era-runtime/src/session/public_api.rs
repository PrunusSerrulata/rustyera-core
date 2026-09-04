#[derive(Clone, Copy, Debug)]
pub struct RuntimeOptions {
    pub session_id: SessionId,
    pub limits: RuntimeLimits,
    pub wire_limits: WireLimits,
    pub vm_config: VmConfig,
    /// Creator-owned upper bound for [`DebugScope`] discriminants.
    pub debug_scope_mask: u64,
    /// Keep complete project file payloads in the reload snapshot after a successful build.
    ///
    /// Constrained hosts that can rematerialize an authorized project may disable this and submit
    /// a complete payload set for each reload. Paths, hashes, resource descriptors, and revisions
    /// remain available in the compact snapshot.
    pub retain_project_source_payloads: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectProgressStage {
    Scanning,
    Normalizing,
    LoadingData,
    Parsing,
    Analyzing,
    Compiling,
    Validating,
    Finalizing,
    Preparing,
    Packaging,
    CacheParsing,
    CacheDecoding,
    CacheValidating,
    InitializingMemory,
    IndexingProgram,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectProgress {
    pub stage: ProjectProgressStage,
    pub completed: u64,
    pub total: u64,
}

#[derive(Clone)]
pub struct ProjectProgressReporter {
    #[cfg(not(target_arch = "wasm32"))]
    callback: Arc<dyn Fn(ProjectProgress) + Send + Sync>,
    #[cfg(not(target_arch = "wasm32"))]
    gate: Arc<std::sync::Mutex<ProjectProgressGate>>,
    #[cfg(not(target_arch = "wasm32"))]
    started_at: Instant,
    #[cfg(target_arch = "wasm32")]
    callback: std::rc::Rc<dyn Fn(ProjectProgress)>,
    #[cfg(target_arch = "wasm32")]
    gate: std::rc::Rc<std::cell::RefCell<ProjectProgressGate>>,
    #[cfg(target_arch = "wasm32")]
    elapsed: std::rc::Rc<dyn Fn() -> Duration>,
}

#[derive(Default)]
struct ProjectProgressGate {
    last: Option<ProjectProgress>,
    last_emitted_at: Option<Duration>,
}

impl ProjectProgressGate {
    // Limit intermediate updates to ten per second while keeping stage boundaries immediate.
    const INTERVAL: Duration = Duration::from_millis(100);

    fn accepts(&mut self, progress: ProjectProgress, now: Duration) -> bool {
        if self.last == Some(progress) {
            return false;
        }
        let boundary = progress.completed == 0 || progress.completed >= progress.total;
        let segment_changed = self.last.is_none_or(|previous| {
            previous.stage != progress.stage || previous.total != progress.total
        });
        if !segment_changed
            && self
                .last
                .is_some_and(|previous| progress.completed < previous.completed)
        {
            return false;
        }
        let interval_elapsed = self
            .last_emitted_at
            .is_none_or(|previous| now.saturating_sub(previous) >= Self::INTERVAL);
        let accepts = segment_changed || boundary || interval_elapsed;
        if accepts {
            self.last = Some(progress);
            self.last_emitted_at = Some(now);
        }
        accepts
    }
}

impl ProjectProgressReporter {
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn new(callback: impl Fn(ProjectProgress) + Send + Sync + 'static) -> Self {
        Self {
            callback: Arc::new(callback),
            gate: Arc::new(std::sync::Mutex::new(ProjectProgressGate::default())),
            started_at: Instant::now(),
        }
    }

    /// Create a reporter with a host clock, retaining the native monotonic clock off WebAssembly.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn new_with_elapsed(
        callback: impl Fn(ProjectProgress) + Send + Sync + 'static,
        _elapsed: impl Fn() -> Duration + 'static,
    ) -> Self {
        Self::new(callback)
    }

    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub fn new(callback: impl Fn(ProjectProgress) + 'static) -> Self {
        Self::new_with_elapsed(callback, || Duration::ZERO)
    }

    /// Create a WebAssembly reporter with a host-provided monotonic elapsed clock.
    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub fn new_with_elapsed(
        callback: impl Fn(ProjectProgress) + 'static,
        elapsed: impl Fn() -> Duration + 'static,
    ) -> Self {
        Self {
            callback: std::rc::Rc::new(callback),
            gate: std::rc::Rc::new(std::cell::RefCell::new(ProjectProgressGate::default())),
            elapsed: std::rc::Rc::new(elapsed),
        }
    }

    pub(crate) fn report(&self, progress: ProjectProgress) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut gate = self
                .gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if gate.accepts(progress, self.started_at.elapsed()) {
                // Serialize the callback with acceptance so concurrent producers cannot
                // reorder accepted updates while crossing an FFI or IPC boundary.
                (self.callback)(progress);
            }
        }
        #[cfg(target_arch = "wasm32")]
        if self.gate.borrow_mut().accepts(progress, (self.elapsed)()) {
            (self.callback)(progress);
        }
    }
}

#[cfg(test)]
mod progress_reporter_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, Mutex};

    use super::*;

    #[test]
    fn project_progress_coalesces_duplicates_and_keeps_stage_boundaries() {
        let reports = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&reports);
        let reporter = ProjectProgressReporter::new(move |progress| {
            observed.lock().unwrap().push(progress);
        });
        for completed in 0..=1_000 {
            let progress = ProjectProgress {
                stage: ProjectProgressStage::Compiling,
                completed,
                total: 1_000,
            };
            reporter.report(progress);
            reporter.report(progress);
        }
        reporter.report(ProjectProgress {
            stage: ProjectProgressStage::Finalizing,
            completed: 0,
            total: 10,
        });
        reporter.report(ProjectProgress {
            stage: ProjectProgressStage::Finalizing,
            completed: 10,
            total: 10,
        });

        let reports = reports.lock().unwrap();
        assert_eq!(reports.first().unwrap().completed, 0);
        assert_eq!(
            reports[reports.len() - 2].stage,
            ProjectProgressStage::Finalizing
        );
        assert_eq!(reports[reports.len() - 2].completed, 0);
        assert_eq!(reports.last().unwrap().completed, 10);
        assert!(reports.len() <= 4);
        assert!(reports.windows(2).all(|pair| pair[0] != pair[1]));
    }

    #[test]
    fn project_progress_gate_uses_time_and_preserves_boundaries() {
        let mut gate = ProjectProgressGate::default();
        let compiling = |completed, total| ProjectProgress {
            stage: ProjectProgressStage::Compiling,
            completed,
            total,
        };

        assert!(gate.accepts(compiling(0, 100), Duration::ZERO));
        assert!(!gate.accepts(compiling(1, 100), Duration::from_millis(99)));
        assert!(gate.accepts(compiling(2, 100), Duration::from_millis(100)));
        assert!(!gate.accepts(compiling(1, 100), Duration::from_millis(101)));
        assert!(gate.accepts(compiling(100, 100), Duration::from_millis(101)));
        assert!(!gate.accepts(compiling(100, 100), Duration::from_millis(102)));
        assert!(gate.accepts(compiling(0, 300), Duration::from_millis(102)));
        assert!(!gate.accepts(compiling(1, 300), Duration::from_millis(201)));
        assert!(gate.accepts(compiling(2, 300), Duration::from_millis(202)));

        let zero_total = ProjectProgress {
            stage: ProjectProgressStage::Preparing,
            completed: 0,
            total: 0,
        };
        assert!(gate.accepts(zero_total, Duration::from_millis(203)));
        assert!(!gate.accepts(zero_total, Duration::from_millis(303)));
    }

    #[test]
    fn project_progress_callback_is_serialized_across_threads() {
        const THREADS: usize = 8;
        let active = Arc::new(AtomicUsize::new(0));
        let maximum_active = Arc::new(AtomicUsize::new(0));
        let observed = Arc::new(Mutex::new(Vec::new()));
        let callback_active = Arc::clone(&active);
        let callback_maximum = Arc::clone(&maximum_active);
        let callback_observed = Arc::clone(&observed);
        let reporter = Arc::new(ProjectProgressReporter::new(move |progress| {
            let current = callback_active.fetch_add(1, Ordering::SeqCst) + 1;
            callback_maximum.fetch_max(current, Ordering::SeqCst);
            std::thread::yield_now();
            callback_observed.lock().unwrap().push(progress);
            callback_active.fetch_sub(1, Ordering::SeqCst);
        }));
        let barrier = Arc::new(Barrier::new(THREADS));
        let handles = (0..THREADS)
            .map(|index| {
                let reporter = Arc::clone(&reporter);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    reporter.report(ProjectProgress {
                        stage: if index % 2 == 0 {
                            ProjectProgressStage::Parsing
                        } else {
                            ProjectProgressStage::Analyzing
                        },
                        completed: 0,
                        total: 1,
                    });
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(maximum_active.load(Ordering::SeqCst), 1);
        assert!(!observed.lock().unwrap().is_empty());
    }

    #[test]
    fn project_progress_reporter_allows_an_intermediate_after_the_interval() {
        let reports = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&reports);
        let reporter = ProjectProgressReporter::new(move |progress| {
            observed.lock().unwrap().push(progress);
        });
        reporter.report(ProjectProgress {
            stage: ProjectProgressStage::Compiling,
            completed: 0,
            total: 100,
        });
        std::thread::sleep(ProjectProgressGate::INTERVAL);
        reporter.report(ProjectProgress {
            stage: ProjectProgressStage::Compiling,
            completed: 1,
            total: 100,
        });
        assert_eq!(reports.lock().unwrap().len(), 2);
    }
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            session_id: SessionId { high: 0, low: 1 },
            limits: RuntimeLimits {
                maximum_envelope_bytes: 1024 * 1024 * 1024,
                maximum_payload_bytes: 1023 * 1024 * 1024,
                maximum_pending_requests: 1024,
                maximum_journal_entries: 4096,
                maximum_drive_instructions: 100_000,
                maximum_transfer_bytes: 1024 * 1024 * 1024,
                maximum_journal_bytes: 64 * 1024 * 1024,
            },
            wire_limits: WireLimits::default(),
            vm_config: VmConfig::default(),
            debug_scope_mask: 0,
            retain_project_source_payloads: true,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RuntimeDriveBudget {
    pub maximum_vm_instructions: u64,
    pub maximum_runtime_transitions: u32,
}

impl Default for RuntimeDriveBudget {
    fn default() -> Self {
        Self {
            maximum_vm_instructions: 100_000,
            maximum_runtime_transitions: 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeDriveState {
    Idle,
    MoreWork,
    OutputReady,
    Stopped,
    Faulted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraditionalSaveInspection {
    pub description: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TraditionalSaveValidationError {
    ProjectUnavailable,
    Invalid(String),
    DifferentGame,
    DifferentVersion,
    Incompatible(String),
}

impl fmt::Display for TraditionalSaveValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProjectUnavailable => formatter.write_str("no compiled project is available"),
            Self::Invalid(message) => write!(formatter, "traditional save is invalid: {message}"),
            Self::DifferentGame => {
                formatter.write_str("traditional save belongs to a different game")
            }
            Self::DifferentVersion => {
                formatter.write_str("traditional save belongs to an incompatible game version")
            }
            Self::Incompatible(message) => {
                write!(formatter, "traditional save is incompatible: {message}")
            }
        }
    }
}

impl std::error::Error for TraditionalSaveValidationError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeDriveReport {
    pub state: RuntimeDriveState,
    pub vm_instructions: u64,
    pub runtime_transitions: u32,
    pub queued_envelopes: u32,
    /// Whether this drive advanced one single-threaded background-work quantum.
    pub cooperative_background_work: bool,
}

#[derive(Debug)]
pub enum RuntimeError {
    Protocol(ProtocolError),
    InvalidSequence {
        expected: u64,
        actual: u64,
    },
    SessionMismatch,
    ResourceLimit(&'static str),
    Busy(&'static str),
    /// A trusted script domain/read failure; only the Host dispatch boundary catches it.
    Script {
        kind: erabasic_vm::ScriptFaultKind,
        message: String,
    },
    Internal(String),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol(error) => error.fmt(formatter),
            Self::InvalidSequence { expected, actual } => {
                write!(formatter, "expected sequence {expected}, received {actual}")
            }
            Self::SessionMismatch => formatter.write_str("runtime session identity differs"),
            Self::ResourceLimit(message) | Self::Busy(message) => formatter.write_str(message),
            Self::Script { message, .. } | Self::Internal(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for RuntimeError {}

impl From<ProtocolError> for RuntimeError {
    fn from(value: ProtocolError) -> Self {
        Self::Protocol(value)
    }
}

