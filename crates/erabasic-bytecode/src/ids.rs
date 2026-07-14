use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Digest(pub [u8; 32]);

impl Digest {
    #[must_use]
    pub fn hash(domain: &str, parts: &[&[u8]]) -> Self {
        let mut hasher = blake3::Hasher::new_derive_key(domain);
        for part in parts {
            hasher.update(&(part.len() as u64).to_le_bytes());
            hasher.update(part);
        }
        Self(*hasher.finalize().as_bytes())
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Digest({self})")
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SymbolKey(pub [u8; 16]);

impl SymbolKey {
    #[must_use]
    pub fn derive(domain: &str, identity: &[u8]) -> Self {
        let digest = Digest::hash(domain, &[identity]);
        let mut key = [0; 16];
        key.copy_from_slice(&digest.0[..16]);
        Self(key)
    }
}

impl fmt::Debug for SymbolKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "SymbolKey(")?;
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        write!(formatter, ")")
    }
}
