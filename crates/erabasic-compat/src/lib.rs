//! Versioned language identity shared by every stage of the runtime pipeline.
//!
//! A requested dialect is separate from the policies actually implemented. The initial snake
//! profile is experimental and intentionally retains the current reference execution policies.

use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Decode,
    Encode,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
    Serialize,
    Deserialize,
)]
#[cbor(index_only)]
pub enum CompatibilityProfileId {
    #[default]
    #[n(0)]
    #[serde(rename = "emuera.em")]
    EmueraEm,
    #[n(1)]
    #[serde(rename = "emuera.skia.snake")]
    EmueraSkiaSnake,
}

impl CompatibilityProfileId {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EmueraEm => "emuera.em",
            Self::EmueraSkiaSnake => "emuera.skia.snake",
        }
    }
}

impl std::fmt::Display for CompatibilityProfileId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::str::FromStr for CompatibilityProfileId {
    type Err = CompatibilityError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "emuera.em" => Ok(Self::EmueraEm),
            "emuera.skia.snake" => Ok(Self::EmueraSkiaSnake),
            _ => Err(CompatibilityError(format!(
                "unknown compatibility profile {value:?}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct CompatibilityServiceContract {
    #[n(0)]
    pub name: String,
    #[n(1)]
    pub version: u32,
}

/// Exact implemented policies. Unknown versions are rejected, never silently downgraded.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct CompatibilityIdentity {
    #[n(0)]
    pub profile: CompatibilityProfileId,
    #[n(1)]
    pub semantic_version: u32,
    #[n(2)]
    pub policy_version: u32,
    #[n(3)]
    pub arithmetic: String,
    #[n(4)]
    pub rng_algorithm: String,
    #[n(5)]
    pub rng_state_version: u32,
    #[n(6)]
    pub layout: String,
    #[n(7)]
    pub save_codec: String,
    #[n(8)]
    pub services: Vec<CompatibilityServiceContract>,
}

impl Default for CompatibilityIdentity {
    fn default() -> Self {
        Self::reference()
    }
}

impl CompatibilityIdentity {
    #[must_use]
    pub fn reference() -> Self {
        Self::for_profile(CompatibilityProfileId::EmueraEm)
    }

    #[must_use]
    pub fn for_profile(profile: CompatibilityProfileId) -> Self {
        let version = match profile {
            CompatibilityProfileId::EmueraEm => 1,
            CompatibilityProfileId::EmueraSkiaSnake => 2,
        };
        Self {
            profile,
            semantic_version: version,
            policy_version: version,
            arithmetic: "wrapping_i64_v1".into(),
            rng_algorithm: "sfmt19937".into(),
            rng_state_version: 1,
            layout: "unicode_column_v1".into(),
            save_codec: match profile {
                CompatibilityProfileId::EmueraEm => "emuera1808",
                CompatibilityProfileId::EmueraSkiaSnake => "rustyera_envelope_v1:emuera1808",
            }
            .into(),
            services: Vec::new(),
        }
    }

    #[must_use]
    pub const fn is_experimental(&self) -> bool {
        matches!(self.profile, CompatibilityProfileId::EmueraSkiaSnake)
    }

    /// User ERD aliases and the snake built-in alias recovery rules arrived in policy v2.
    #[must_use]
    pub const fn uses_snake_alias_rules(&self) -> bool {
        matches!(self.profile, CompatibilityProfileId::EmueraSkiaSnake) && self.policy_version >= 2
    }

    /// Validate the complete policy, including semantic service versions.
    ///
    /// # Errors
    /// Returns an error for any identity not implemented by this runtime build.
    pub fn validate(&self) -> Result<(), CompatibilityError> {
        if self != &Self::for_profile(self.profile) {
            return Err(CompatibilityError(format!(
                "unsupported compatibility identity for {} (semantic {}, policy {})",
                self.profile, self.semantic_version, self.policy_version
            )));
        }
        Ok(())
    }

    /// Canonical CBOR map order and a domain-separated BLAKE3 hash define identity v1.
    ///
    /// # Panics
    /// Panics only if encoding the fixed in-memory identity into a Vec fails.
    #[must_use]
    pub fn digest(&self) -> [u8; 32] {
        let encoded = minicbor::to_vec(self).expect("compatibility identity encodes into memory");
        let mut hasher = blake3::Hasher::new_derive_key("rustyera.compatibility.identity.v1");
        hasher.update(&encoded);
        *hasher.finalize().as_bytes()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompatibilityError(pub String);

impl std::fmt::Display for CompatibilityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for CompatibilityError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_are_explicit_and_validate_all_policy_fields() {
        let reference = CompatibilityIdentity::reference();
        let snake = CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake);
        assert_ne!(reference.digest(), snake.digest());
        assert_eq!(reference.arithmetic, snake.arithmetic);
        assert_eq!(reference.rng_algorithm, snake.rng_algorithm);
        assert!(snake.is_experimental());
        assert!(snake.uses_snake_alias_rules());
        assert!(!reference.uses_snake_alias_rules());
        let mut previous_snake = snake.clone();
        previous_snake.semantic_version = 1;
        previous_snake.policy_version = 1;
        assert!(previous_snake.validate().is_err());
        assert!(reference.validate().is_ok());
        assert!(snake.validate().is_ok());
        let mut unsupported = snake;
        unsupported.rng_state_version += 1;
        assert!(unsupported.validate().is_err());
        assert!("emuera.snake".parse::<CompatibilityProfileId>().is_err());
    }
}
