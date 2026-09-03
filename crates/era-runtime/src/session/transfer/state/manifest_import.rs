//! Decode the manifest a file at a time, without retaining a second complete CBOR payload.

use era_protocol::decode_canonical_nested;
use era_runtime_protocol::{CompatibilityIdentity, ProjectManifest, SubmittedFile};

use crate::RuntimeError;

#[derive(Debug, Default)]
enum Phase {
    #[default]
    Map,
    Key,
    Value(u64),
    Files(u64),
    Done,
    Invalid,
}

#[derive(Debug, Default)]
pub(in super::super::super::super) struct ManifestImportDecoder {
    pending: Vec<u8>,
    phase: Phase,
    fields_left: u64,
    previous_key: Option<u64>,
    revision: Option<u64>,
    files: Option<Vec<SubmittedFile>>,
    compatibility: Option<CompatibilityIdentity>,
    pub(in super::super::super::super) received: u64,
}

impl ManifestImportDecoder {
    pub(in super::super::super::super) fn push(
        &mut self,
        bytes: &[u8],
    ) -> Result<(), RuntimeError> {
        self.received += bytes.len() as u64;
        if matches!(self.phase, Phase::Invalid) {
            return Ok(());
        }
        self.pending
            .try_reserve(bytes.len())
            .map_err(|_| RuntimeError::ResourceLimit("manifest file allocation failed"))?;
        self.pending.extend_from_slice(bytes);
        let mut consumed = 0;
        loop {
            match self.advance(consumed) {
                Ok(Some(length)) => consumed += length,
                Ok(None) => break,
                Err(()) => {
                    // Preserve the protocol's commit-time rejection, but discard invalid data now.
                    self.phase = Phase::Invalid;
                    self.pending = Vec::new();
                    self.files = None;
                    return Ok(());
                }
            }
        }
        self.pending.drain(..consumed);
        Ok(())
    }

    fn advance(&mut self, offset: usize) -> Result<Option<usize>, ()> {
        let bytes = &self.pending[offset..];
        match self.phase {
            Phase::Map => {
                let Some((fields, length)) = header(bytes, 5)? else {
                    return Ok(None);
                };
                self.fields_left = fields;
                self.phase = if fields == 0 { Phase::Done } else { Phase::Key };
                Ok(Some(length))
            }
            Phase::Key => {
                let Some((key, length)) = header(bytes, 0)? else {
                    return Ok(None);
                };
                if key > u64::from(u32::MAX) || self.previous_key.is_some_and(|last| last >= key) {
                    return Err(());
                }
                self.previous_key = Some(key);
                self.phase = Phase::Value(key);
                Ok(Some(length))
            }
            Phase::Value(1) => {
                let Some((count, length)) = header(bytes, 4)? else {
                    return Ok(None);
                };
                // Do not allocate from an untrusted declared item count.
                self.files = Some(Vec::new());
                if count == 0 {
                    self.end_field();
                } else {
                    self.phase = Phase::Files(count);
                }
                Ok(Some(length))
            }
            Phase::Value(key) => {
                let Some(length) = item_length(bytes)? else {
                    return Ok(None);
                };
                let value = &bytes[..length];
                match key {
                    0 => self.revision = Some(decode_canonical_nested(value, 1).map_err(|_| ())?),
                    2 => {
                        self.compatibility =
                            Some(decode_canonical_nested(value, 1).map_err(|_| ())?);
                    }
                    _ => {
                        decode_canonical_nested::<IgnoredValue>(value, 1).map_err(|_| ())?;
                    }
                }
                self.end_field();
                Ok(Some(length))
            }
            Phase::Files(left) => {
                let Some(length) = item_length(bytes)? else {
                    return Ok(None);
                };
                let file = decode_canonical_nested(&bytes[..length], 2).map_err(|_| ())?;
                self.files.as_mut().ok_or(())?.push(file);
                if left == 1 {
                    self.end_field();
                } else {
                    self.phase = Phase::Files(left - 1);
                }
                Ok(Some(length))
            }
            Phase::Done if bytes.is_empty() => Ok(None),
            Phase::Done | Phase::Invalid => Err(()),
        }
    }

    fn end_field(&mut self) {
        self.fields_left -= 1;
        self.phase = if self.fields_left == 0 {
            Phase::Done
        } else {
            Phase::Key
        };
    }

    pub(in super::super::super::super) fn finish(self) -> Result<ProjectManifest, ()> {
        if !matches!(self.phase, Phase::Done) || !self.pending.is_empty() {
            return Err(());
        }
        Ok(ProjectManifest {
            project_revision: self.revision.ok_or(())?,
            files: self.files.ok_or(())?,
            compatibility: self.compatibility.ok_or(())?,
        })
    }
}

// A container header cannot be passed alone to decode_canonical; enforce the same shortest
// definite-length representation here. Keys are unsigned field indices, as in the derived codec.
fn header(bytes: &[u8], major: u8) -> Result<Option<(u64, usize)>, ()> {
    let Some(&initial) = bytes.first() else {
        return Ok(None);
    };
    if initial >> 5 != major {
        return Err(());
    }
    let additional = initial & 31;
    let (width, minimum) = match additional {
        0..=23 => return Ok(Some((u64::from(additional), 1))),
        24 => (1, 24),
        25 => (2, 256),
        26 => (4, 65_536),
        27 => (8, 4_294_967_296),
        _ => return Err(()),
    };
    let Some(argument) = bytes.get(1..=width) else {
        return Ok(None);
    };
    let value = argument
        .iter()
        .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte));
    if value < minimum {
        return Err(());
    }
    Ok(Some((value, width + 1)))
}

fn item_length(bytes: &[u8]) -> Result<Option<usize>, ()> {
    let mut decoder = minicbor::Decoder::new(bytes);
    match decoder.skip() {
        Ok(()) => Ok(Some(decoder.position())),
        Err(error) if error.is_end_of_input() => Ok(None),
        Err(_) => Err(()),
    }
}

// Unknown fields remain forward compatible, while retaining canonical validation of their value.
struct IgnoredValue;

impl<'b> minicbor::Decode<'b, ()> for IgnoredValue {
    fn decode(
        decoder: &mut minicbor::Decoder<'b>,
        (): &mut (),
    ) -> Result<Self, minicbor::decode::Error> {
        decoder.skip()?;
        Ok(Self)
    }
}

impl minicbor::Encode<()> for IgnoredValue {
    fn encode<W: minicbor::encode::Write>(
        &self,
        encoder: &mut minicbor::Encoder<W>,
        (): &mut (),
    ) -> Result<(), minicbor::encode::Error<W::Error>> {
        encoder.null()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
