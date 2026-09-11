use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use super::{AuditResult, OUTPUT_SCHEMA_VERSION};

const WATCHDOG_PUBLISH_INTERVAL: Duration = Duration::from_secs(4);

pub(super) struct JsonlSink {
    writer: Box<dyn Write>,
    terminal_emitted: bool,
    sequence: u64,
}

impl JsonlSink {
    pub(super) fn new(path: Option<&Path>) -> AuditResult<Self> {
        let writer: Box<dyn Write> = match path {
            Some(path) => {
                if let Some(parent) = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent)?;
                }
                Box::new(BufWriter::new(fs::File::create(path)?))
            }
            None => Box::new(BufWriter::new(std::io::stdout())),
        };
        Ok(Self {
            writer,
            terminal_emitted: false,
            sequence: 0,
        })
    }

    pub(super) fn emit(&mut self, event: &str, fields: Value) -> AuditResult<()> {
        let mut object = match fields {
            Value::Object(object) => object,
            _ => return Err("JSONL event fields must be an object".into()),
        };
        object.insert("schemaVersion".into(), Value::from(OUTPUT_SCHEMA_VERSION));
        object.insert("epoch".into(), Value::from(1));
        object.insert("sequence".into(), Value::from(self.sequence));
        object.insert("event".into(), Value::from(event));
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or("performance JSONL sequence overflow")?;
        serde_json::to_writer(&mut self.writer, &Value::Object(object))?;
        self.writer.write_all(b"\n")?;
        Ok(())
    }

    pub(super) fn terminal(&mut self, status: &str, fields: Value) -> AuditResult<()> {
        if self.terminal_emitted {
            return Err("terminal JSONL event was emitted more than once".into());
        }
        self.terminal_emitted = true;
        let mut object = match fields {
            Value::Object(object) => object,
            _ => Map::new(),
        };
        object.insert("status".into(), Value::from(status));
        self.emit("terminal", Value::Object(object))?;
        self.flush()
    }

    pub(super) fn flush(&mut self) -> AuditResult<()> {
        self.writer.flush()?;
        Ok(())
    }
}

pub(super) struct WatchdogThrottle {
    last_publish: Option<Instant>,
}

impl WatchdogThrottle {
    pub(super) const fn new() -> Self {
        Self { last_publish: None }
    }

    pub(super) fn publish(&mut self, state: Value, force: bool) -> AuditResult<()> {
        self.publish_with(force, || Ok(state))
    }

    pub(super) fn publish_with(
        &mut self,
        force: bool,
        build: impl FnOnce() -> AuditResult<Value>,
    ) -> AuditResult<()> {
        let now = Instant::now();
        if force || self.is_due(now) {
            super::super::watchdog::publish(build()?)?;
            self.last_publish = Some(now);
        }
        Ok(())
    }

    fn is_due(&self, now: Instant) -> bool {
        self.last_publish
            .is_none_or(|previous| now.duration_since(previous) >= WATCHDOG_PUBLISH_INTERVAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watchdog_throttle_is_strictly_below_supervisor_interval() {
        assert!(WATCHDOG_PUBLISH_INTERVAL < Duration::from_secs(5));
        let now = Instant::now();
        let mut throttle = WatchdogThrottle::new();
        assert!(throttle.is_due(now));
        throttle.last_publish = Some(now);
        assert!(!throttle.is_due(now + Duration::from_secs(3)));
        assert!(throttle.is_due(now + Duration::from_secs(4)));
    }

    #[test]
    fn throttled_snapshot_is_built_only_when_due() -> AuditResult<()> {
        let mut throttle = WatchdogThrottle::new();
        throttle.last_publish = Some(Instant::now());
        let mut built = false;
        throttle.publish_with(false, || {
            built = true;
            Ok(Value::Null)
        })?;
        assert!(!built);
        Ok(())
    }
}
