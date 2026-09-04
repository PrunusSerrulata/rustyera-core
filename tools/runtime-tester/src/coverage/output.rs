//! Incremental raw/encoded digests. A gzip trailer and destination flush must succeed first.

use flate2::{Compression, write::GzEncoder};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::{
    fs::File,
    io::{self, Read, Write},
    path::Path,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize)]
pub(super) struct Digest {
    pub bytes: u64,
    pub blake3: String,
    pub sha256: String,
}

struct Digests {
    bytes: u64,
    blake3: blake3::Hasher,
    sha256: Sha256,
}
impl Default for Digests {
    fn default() -> Self {
        Self {
            bytes: 0,
            blake3: blake3::Hasher::new(),
            sha256: Sha256::new(),
        }
    }
}
impl Digests {
    fn update(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| io::Error::other("report byte count overflow"))?;
        self.blake3.update(bytes);
        self.sha256.update(bytes);
        Ok(())
    }
    fn finish(self) -> Digest {
        Digest {
            bytes: self.bytes,
            blake3: self.blake3.finalize().to_hex().to_string(),
            sha256: format!("{:x}", self.sha256.finalize()),
        }
    }
}

struct Hashed<W> {
    inner: W,
    hashes: Digests,
}
impl<W: Write> Write for Hashed<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(bytes)?;
        self.hashes.update(&bytes[..written])?;
        Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

enum Encoding<W: Write> {
    Plain(Hashed<W>),
    Gzip(GzEncoder<Hashed<W>>),
}
impl<W: Write> Write for Encoding<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(writer) => writer.write(bytes),
            Self::Gzip(writer) => writer.write(bytes),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(writer) => writer.flush(),
            Self::Gzip(writer) => writer.flush(),
        }
    }
}

pub(super) struct ReportOutput<W: Write> {
    inner: Hashed<Encoding<W>>,
    gzip: bool,
    last_progress: Option<Instant>,
    progress_bytes: u64,
}
impl<W: Write> ReportOutput<W> {
    pub fn new(destination: W, gzip: bool) -> Self {
        let encoded = Hashed {
            inner: destination,
            hashes: Digests::default(),
        };
        let inner = if gzip {
            Encoding::Gzip(GzEncoder::new(encoded, Compression::default()))
        } else {
            Encoding::Plain(encoded)
        };
        Self {
            inner: Hashed {
                inner,
                hashes: Digests::default(),
            },
            gzip,
            last_progress: None,
            progress_bytes: 0,
        }
    }
    pub fn finish(self) -> io::Result<(W, serde_json::Value)> {
        let raw = self.inner.hashes.finish();
        let mut encoded = match self.inner.inner {
            Encoding::Plain(writer) => writer,
            Encoding::Gzip(writer) => writer.finish()?,
        };
        encoded.flush()?;
        let stored = encoded.hashes.finish();
        Ok((
            encoded.inner,
            serde_json::json!({"version": 1, "status": "complete", "encoding": if self.gzip { "gzip" } else { "identity" },
            "raw_json": raw, "stored_file": stored, "completion_policy": "only_after_json_close_gzip_finish_and_destination_flush"}),
        ))
    }
}
impl<W: Write> Write for ReportOutput<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(bytes)?;
        let completed = self.inner.hashes.bytes;
        if written > 0
            && (self.last_progress.is_none()
                || completed.saturating_sub(self.progress_bytes) >= 65536)
            && self
                .last_progress
                .is_none_or(|time| time.elapsed() >= Duration::from_secs(1))
        {
            let encoded = match &self.inner.inner {
                Encoding::Plain(writer) => writer.hashes.bytes,
                Encoding::Gzip(writer) => writer.get_ref().hashes.bytes,
            };
            crate::watchdog::publish(
                serde_json::json!({"phase": "coverage_write_report", "pending": "streaming_json",
                "raw_bytes_written": completed, "encoded_bytes_written": encoded, "lastFullResponse": null}),
            )?;
            self.last_progress = Some(Instant::now());
            self.progress_bytes = completed;
        }
        Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

pub(super) fn hash_path(path: &Path, maximum: u64) -> io::Result<Digest> {
    let mut input = File::open(path)?;
    if input.metadata()?.len() > maximum {
        return Err(io::Error::other("input exceeds configured byte limit"));
    }
    let mut hash = Digests::default();
    let mut bytes = [0; 65536];
    let mut last_progress = None::<Instant>;
    loop {
        let count = input.read(&mut bytes)?;
        if count == 0 {
            return Ok(hash.finish());
        }
        hash.update(&bytes[..count])?;
        if hash.bytes > maximum {
            return Err(io::Error::other("input grew beyond configured byte limit"));
        }
        if last_progress.is_none_or(|time| time.elapsed() >= Duration::from_secs(1)) {
            crate::watchdog::publish(
                serde_json::json!({"phase": "coverage_hash_input", "case": path,
                "pending": "streaming_bytes", "bytes_completed": hash.bytes, "lastFullResponse": null}),
            )?;
            last_progress = Some(Instant::now());
        }
    }
}
