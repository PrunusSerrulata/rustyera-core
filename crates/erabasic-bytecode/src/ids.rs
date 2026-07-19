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

#[derive(Clone, Copy, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
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

impl Serialize for SymbolKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut encoded = [0u8; 32];
        for (index, byte) in self.0.into_iter().enumerate() {
            encoded[index * 2] = HEX[usize::from(byte >> 4)];
            encoded[index * 2 + 1] = HEX[usize::from(byte & 0x0f)];
        }
        let encoded = std::str::from_utf8(&encoded).expect("hex digits are valid UTF-8");
        serializer.serialize_str(encoded)
    }
}

impl<'de> Deserialize<'de> for SymbolKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        if encoded.len() != 32 {
            return Err(serde::de::Error::custom(
                "symbol key must contain 32 hexadecimal digits",
            ));
        }
        let mut bytes = [0; 16];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&encoded[index * 2..index * 2 + 2], 16)
                .map_err(serde::de::Error::custom)?;
        }
        Ok(Self(bytes))
    }
}
