use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use era_runtime_protocol::{
    SqlProviderHandleV1, SqlRequestV1, SqlResponseV1, StorageRequest, StorageResponse,
};

use crate::{SqlStorage, engine::Engine, policy::Control};

#[cfg(test)]
mod tests;

const TRANSPORT_BUDGET: Duration = Duration::from_secs(30);
const SHUTDOWN_BUDGET: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

enum Lifecycle {
    Register(SqlProviderHandleV1, crate::ProviderRole),
    Retire(SqlProviderHandleV1),
    PromoteCandidate(SqlProviderHandleV1),
    Reset,
}

enum Command {
    Execute(Box<SqlRequestV1>, u16, SyncSender<Reply>),
    Lifecycle(Lifecycle, SyncSender<crate::Result<()>>),
}

enum Reply {
    Storage(StorageRequest, SyncSender<StorageResponse>),
    Complete(SqlResponseV1),
}

/// Safe cross-thread cancellation. Cancellation permanently retires this owner;
/// construct a new provider after shutdown instead of clearing cancellation.
#[derive(Clone)]
pub struct CancellationHandle {
    control: Control,
}

impl CancellationHandle {
    pub fn cancel(&self) {
        self.control.cancel();
    }
}

/// `SQLite` handles never leave their owner thread. Only owned protocol values cross this queue.
pub struct NativeSqlProvider {
    commands: SyncSender<Command>,
    control: Control,
    completion: Receiver<()>,
    owner: Option<JoinHandle<()>>,
    completed: bool,
}

impl NativeSqlProvider {
    /// Create the fixed owner thread. A stopped worker is never replaced mid-request.
    ///
    /// # Errors
    /// Returns the operating system error if the owner thread cannot be created.
    pub fn new() -> std::io::Result<Self> {
        let (commands, receiver) = mpsc::sync_channel(1);
        let (done, completion) = mpsc::sync_channel(1);
        let control = Control::default();
        let worker_control = control.clone();
        let owner = thread::Builder::new()
            .name("era-sql".into())
            .spawn(move || {
                // This guard outlives run's engine, including during unwinding.
                let _completion = Completion(done);
                run(&receiver, &worker_control);
            })?;
        Ok(Self {
            commands,
            control,
            completion,
            owner: Some(owner),
            completed: false,
        })
    }

    #[must_use]
    pub fn cancellation_handle(&self) -> CancellationHandle {
        CancellationHandle {
            control: self.control.clone(),
        }
    }

    /// Execute one request, servicing storage on the calling host thread.
    ///
    /// The host MUST bound each `SqlStorage::handle` callback, including filesystem
    /// operations and publication, within the request's remaining 30-second transport
    /// budget. This synchronous callback cannot be preempted by Rust cancellation;
    /// hosts must supply their own I/O deadlines. Cancellation cannot undo a callback
    /// already entered or a publication it has committed. No new callback or returned
    /// continuation is accepted after cancellation/expiry is observed.
    ///
    /// # Errors
    /// Owner/transport failures permanently cancel the provider. SQL failures remain
    /// structured responses. The independent SQL execution limit remains five seconds.
    pub fn handle(
        &mut self,
        request: SqlRequestV1,
        minor: u16,
        storage: &mut dyn SqlStorage,
    ) -> std::result::Result<SqlResponseV1, String> {
        let result = (|| {
            self.start_request()?;
            let (sender, receiver) = mpsc::sync_channel(1);
            self.commands
                .try_send(Command::Execute(Box::new(request), minor, sender))
                .map_err(|_| "native SQL owner stopped".to_owned())?;
            loop {
                match receive(&receiver, &self.control)? {
                    Reply::Storage(request, response) => {
                        self.control.checkpoint().map_err(|error| error.message)?;
                        let value = storage.handle(request);
                        self.control.checkpoint().map_err(|error| error.message)?;
                        response
                            .try_send(value)
                            .map_err(|_| "native SQL storage continuation stopped".to_owned())?;
                    }
                    Reply::Complete(response) => return Ok(response),
                }
            }
        })();
        self.finish_request(result)
    }

    /// Register a host-issued provider identity and wait for the owner's ACK.
    ///
    /// # Errors
    /// Rejects retired identifiers, occupied roles, and unavailable owner transport.
    pub fn register(
        &mut self,
        handle: SqlProviderHandleV1,
        role: crate::ProviderRole,
    ) -> crate::Result<()> {
        self.lifecycle(Lifecycle::Register(handle, role))
    }

    /// Release a registered provider without disturbing another live or candidate provider.
    ///
    /// # Errors
    /// Rejects unregistered identifiers and unavailable owner transport.
    pub fn retire(&mut self, handle: SqlProviderHandleV1) -> crate::Result<()> {
        self.lifecycle(Lifecycle::Retire(handle))
    }

    /// Promote the registered candidate and release its predecessor.
    ///
    /// # Errors
    /// Rejects identifiers that are not the candidate, or unavailable owner transport.
    pub fn promote_candidate(&mut self, handle: SqlProviderHandleV1) -> crate::Result<()> {
        self.lifecycle(Lifecycle::PromoteCandidate(handle))
    }

    /// Clear all handles while preserving registration high-water marks and cancellation.
    ///
    /// # Errors
    /// Returns an error if the owner transport cannot acknowledge completion.
    pub fn reset(&mut self) -> std::result::Result<(), String> {
        self.lifecycle(Lifecycle::Reset)
            .map_err(|error| error.message)
    }

    fn lifecycle(&mut self, operation: Lifecycle) -> crate::Result<()> {
        let result = (|| {
            self.start_request()?;
            let (sender, receiver) = mpsc::sync_channel(1);
            self.commands
                .try_send(Command::Lifecycle(operation, sender))
                .map_err(|_| "native SQL owner stopped".to_owned())?;
            receive(&receiver, &self.control)
        })();
        self.finish_request(result).map_err(|message| {
            crate::ProviderError::new(era_runtime_protocol::SqlErrorCodeV1::InvalidState, message)
        })?
    }

    fn start_request(&self) -> std::result::Result<(), String> {
        self.control
            .begin_request(Instant::now() + TRANSPORT_BUDGET)
            .map_err(|error| error.message)
    }

    fn finish_request<T>(
        &self,
        result: std::result::Result<T, String>,
    ) -> std::result::Result<T, String> {
        let result = result.and_then(|value| {
            self.control
                .finish_request()
                .map_err(|error| error.message)?;
            Ok(value)
        });
        if result.is_err() {
            self.control.cancel();
        }
        result
    }

    /// Cancel permanently and wait at most five seconds for actual thread completion.
    /// Success confirms that all owner connections/readers have been destroyed. On
    /// timeout the retained join handle permits a later shutdown call to confirm exit.
    ///
    /// # Errors
    /// Reports an unconfirmed shutdown deadline or an owner thread panic.
    pub fn shutdown(&mut self) -> std::result::Result<(), String> {
        self.control.cancel();
        let deadline = Instant::now() + SHUTDOWN_BUDGET;
        while self
            .owner
            .as_ref()
            .is_some_and(|owner| !owner.is_finished())
        {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(
                    "native SQL shutdown completion was not confirmed within five seconds".into(),
                );
            }
            if self.completed {
                thread::sleep(remaining.min(POLL_INTERVAL));
            } else {
                match self.completion.recv_timeout(remaining.min(POLL_INTERVAL)) {
                    Ok(()) | Err(RecvTimeoutError::Disconnected) => self.completed = true,
                    Err(RecvTimeoutError::Timeout) => {}
                }
            }
        }
        if let Some(owner) = self.owner.take() {
            self.completed = true;
            owner
                .join()
                .map_err(|_| "native SQL owner panicked during shutdown".to_owned())?;
        }
        Ok(())
    }
}

impl Drop for NativeSqlProvider {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

struct Completion(SyncSender<()>);

impl Drop for Completion {
    fn drop(&mut self) {
        let _ = self.0.try_send(());
    }
}

fn receive<T>(receiver: &Receiver<T>, control: &Control) -> std::result::Result<T, String> {
    loop {
        control.checkpoint().map_err(|error| error.message)?;
        match receiver.recv_timeout(POLL_INTERVAL) {
            Ok(value) => {
                control.checkpoint().map_err(|error| error.message)?;
                return Ok(value);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err("native SQL owner channel stopped".into());
            }
        }
    }
}

struct ContinuationStorage {
    replies: SyncSender<Reply>,
    control: Control,
}

impl SqlStorage for ContinuationStorage {
    fn handle(&mut self, request: StorageRequest) -> StorageResponse {
        let request_id = request.request_id;
        let (sender, receiver) = mpsc::sync_channel(1);
        if !self.control.expired()
            && self
                .replies
                .try_send(Reply::Storage(request, sender))
                .is_ok()
            && let Ok(response) = receive(&receiver, &self.control)
        {
            return response;
        }
        self.control.cancel();
        StorageResponse {
            request_id,
            result: era_runtime_protocol::StorageResult::Error {
                error: era_runtime_protocol::FrontendIoError {
                    kind: era_runtime_protocol::FrontendIoErrorKind::Other,
                    message: "native SQL storage continuation stopped".into(),
                    platform_code: None,
                },
            },
        }
    }
}

fn run(receiver: &Receiver<Command>, control: &Control) {
    let mut engine = Engine::new(control.clone());
    while let Ok(command) = receive(receiver, control) {
        match command {
            Command::Lifecycle(operation, sender) => {
                let result = match operation {
                    Lifecycle::Register(handle, role) => engine.register(handle, role),
                    Lifecycle::Retire(handle) => engine.retire(handle),
                    Lifecycle::PromoteCandidate(handle) => engine.promote_candidate(handle),
                    Lifecycle::Reset => {
                        engine.reset();
                        Ok(())
                    }
                };
                if control.expired() || sender.try_send(result).is_err() {
                    break;
                }
            }
            Command::Execute(request, minor, sender) => {
                let response = engine.handle(
                    &request,
                    minor,
                    &mut ContinuationStorage {
                        replies: sender.clone(),
                        control: control.clone(),
                    },
                );
                if control.expired() || sender.try_send(Reply::Complete(response)).is_err() {
                    break;
                }
            }
        }
    }
    control.cancel();
}
