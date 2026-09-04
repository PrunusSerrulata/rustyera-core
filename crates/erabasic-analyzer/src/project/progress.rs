use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnalysisProgressStage {
    Parsing,
    DeclaringGlobals,
    IndexingFunctions,
    DeclaringLocals,
    Analyzing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnalysisProgress {
    pub stage: AnalysisProgressStage,
    pub completed: usize,
    pub total: usize,
}

pub(crate) struct ProgressCounter<'a> {
    stage: AnalysisProgressStage,
    total: usize,
    report_interval: usize,
    completed: AtomicUsize,
    reported_completed: AtomicUsize,
    callback_lock: Mutex<()>,
    callback: Option<&'a dyn AnalysisProgressCallback>,
}

impl<'a> ProgressCounter<'a> {
    pub(crate) fn new(
        stage: AnalysisProgressStage,
        total: usize,
        callback: Option<&'a dyn AnalysisProgressCallback>,
    ) -> Self {
        if let Some(callback) = callback {
            callback(AnalysisProgress {
                stage,
                completed: 0,
                total,
            });
        }
        Self {
            stage,
            total,
            report_interval: total.checked_div(64).unwrap_or(0).max(64),
            completed: AtomicUsize::new(0),
            reported_completed: AtomicUsize::new(0),
            callback_lock: Mutex::new(()),
            callback,
        }
    }

    pub(crate) fn advance(&self) {
        let completed = self.completed.fetch_add(1, Ordering::Relaxed) + 1;
        let Some(callback) = self.callback else {
            return;
        };
        let previous = self.reported_completed.load(Ordering::Relaxed);
        if !self.should_report(completed, previous) {
            return;
        }
        let _guard = self
            .callback_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Observe the latest completed count under the callback lock: parallel
        // workers must never publish an older count after a newer one.
        let completed = self.completed.load(Ordering::Relaxed);
        let previous = self.reported_completed.load(Ordering::Relaxed);
        // Percentage-only reporting can hide thousands of completed functions. Keep bounded
        // sub-percent updates without flooding browser hosts when a large project advances fast.
        if self.should_report(completed, previous) {
            self.reported_completed.store(completed, Ordering::Relaxed);
            callback(AnalysisProgress {
                stage: self.stage,
                completed,
                total: self.total,
            });
        }
    }

    fn should_report(&self, completed: usize, previous: usize) -> bool {
        let percent = completed
            .saturating_mul(100)
            .checked_div(self.total)
            .unwrap_or(100);
        let previous_percent = previous
            .saturating_mul(100)
            .checked_div(self.total)
            .unwrap_or(100);
        completed > previous
            && (percent > previous_percent
                || completed - previous >= self.report_interval
                || completed == self.total)
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub trait AnalysisProgressCallback: Fn(AnalysisProgress) + Sync {}

#[cfg(not(target_arch = "wasm32"))]
impl<T> AnalysisProgressCallback for T where T: Fn(AnalysisProgress) + Sync {}

#[cfg(target_arch = "wasm32")]
pub trait AnalysisProgressCallback: Fn(AnalysisProgress) {}

#[cfg(target_arch = "wasm32")]
impl<T> AnalysisProgressCallback for T where T: Fn(AnalysisProgress) {}
