//! Versioned language identity shared by every stage of the runtime pipeline.
//!
//! A requested dialect is separate from the policies actually implemented. The snake profile
//! remains experimental while its arithmetic policy is shared by analysis and execution.

use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

mod calls;
mod integer;

pub use calls::{UserCallArgumentPolicy, UserCallArityDecision, UserCallArityDiagnostic};

pub use integer::{
    IntegerArithmeticError, IntegerArithmeticOutcome, IntegerArithmeticPolicy,
    IntegerArithmeticWarning, IntegerOperation,
};

pub const SQL_SERVICE_CONTRACT_NAME: &str = "rustyera.sql";
pub const SQL_SERVICE_CONTRACT_VERSION: u16 = 1;
pub const SQL_LIMITS_CONTRACT_NAME: &str = "rustyera.sql.limits";
pub const SQL_LIMITS_CONTRACT_VERSION: u32 = 1;
pub const SCENE_CONTRACT_NAME: &str = "rustyera.scene";
pub const SCENE_CONTRACT_VERSION: u32 = 1;
pub const AUDIO_SERVICE_CONTRACT_NAME: &str = "rustyera.audio";
pub const AUDIO_SERVICE_CONTRACT_VERSION: u32 = 1;
pub const SNAKE_INTEROP_SAVE_CODEC: &str = "snake_emuera1808_interop_v1";

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
            CompatibilityProfileId::EmueraSkiaSnake => 12,
        };
        Self {
            profile,
            semantic_version: version,
            policy_version: version,
            arithmetic: match profile {
                CompatibilityProfileId::EmueraEm => "wrapping_i64_v1",
                CompatibilityProfileId::EmueraSkiaSnake => "snake_saturating_i64_v1",
            }
            .into(),
            rng_algorithm: "sfmt19937".into(),
            rng_state_version: 1,
            layout: "unicode_column_v1".into(),
            save_codec: match profile {
                CompatibilityProfileId::EmueraEm => "emuera1808",
                CompatibilityProfileId::EmueraSkiaSnake => SNAKE_INTEROP_SAVE_CODEC,
            }
            .into(),
            services: match profile {
                CompatibilityProfileId::EmueraEm => Vec::new(),
                CompatibilityProfileId::EmueraSkiaSnake => vec![
                    CompatibilityServiceContract {
                        name: SQL_SERVICE_CONTRACT_NAME.into(),
                        version: u32::from(SQL_SERVICE_CONTRACT_VERSION),
                    },
                    CompatibilityServiceContract {
                        name: SQL_LIMITS_CONTRACT_NAME.into(),
                        version: SQL_LIMITS_CONTRACT_VERSION,
                    },
                    CompatibilityServiceContract {
                        name: SCENE_CONTRACT_NAME.into(),
                        version: SCENE_CONTRACT_VERSION,
                    },
                    CompatibilityServiceContract {
                        name: AUDIO_SERVICE_CONTRACT_NAME.into(),
                        version: AUDIO_SERVICE_CONTRACT_VERSION,
                    },
                ],
            },
        }
    }

    #[must_use]
    pub const fn is_experimental(&self) -> bool {
        matches!(self.profile, CompatibilityProfileId::EmueraSkiaSnake)
    }

    /// Arithmetic selected by this identity; callers validate identities before use.
    #[must_use]
    pub const fn integer_arithmetic_policy(&self) -> IntegerArithmeticPolicy {
        match self.profile {
            CompatibilityProfileId::EmueraEm => IntegerArithmeticPolicy::ReferenceWrappingV1,
            CompatibilityProfileId::EmueraSkiaSnake => IntegerArithmeticPolicy::SnakeSaturatingV1,
        }
    }

    /// Snake policy v3 returns zero when TOINT's integer reader fails.
    #[must_use]
    pub const fn uses_snake_numeric_read_fallback(&self) -> bool {
        matches!(self.profile, CompatibilityProfileId::EmueraSkiaSnake) && self.policy_version >= 3
    }

    /// Complete call text and checked forms share the v4 execution contract.
    #[must_use]
    pub const fn supports_call_text(&self) -> bool {
        matches!(self.profile, CompatibilityProfileId::EmueraSkiaSnake) && self.policy_version >= 4
    }

    #[must_use]
    pub const fn supports_checked_runtime_forms(&self) -> bool {
        self.supports_call_text()
    }

    #[must_use]
    pub const fn supports_existvar_expression_probe(&self) -> bool {
        self.supports_call_text()
    }

    /// Deterministic data extensions share the v6 execution contract.
    #[must_use]
    pub const fn supports_snake_data_apis(&self) -> bool {
        matches!(self.profile, CompatibilityProfileId::EmueraSkiaSnake) && self.policy_version >= 6
    }

    #[must_use]
    pub const fn supports_map_extensions(&self) -> bool {
        self.supports_snake_data_apis()
    }

    /// Final script-fault hooks are part of the v7 snake execution policy.
    #[must_use]
    pub const fn supports_fault_hooks(&self) -> bool {
        matches!(self.profile, CompatibilityProfileId::EmueraSkiaSnake) && self.policy_version >= 7
    }

    /// Normalized history display state and logical animation timers are part of policy v8.
    #[must_use]
    pub const fn supports_snake_display_state(&self) -> bool {
        matches!(self.profile, CompatibilityProfileId::EmueraSkiaSnake) && self.policy_version >= 8
    }

    /// Runtime-owned input control, device latches, and environment queries are policy v9.
    #[must_use]
    pub const fn supports_snake_input(&self) -> bool {
        matches!(self.profile, CompatibilityProfileId::EmueraSkiaSnake) && self.policy_version >= 9
    }

    /// Safe SQL catalog and service identity are part of snake policy v10.
    #[must_use]
    pub const fn supports_safe_sql(&self) -> bool {
        matches!(self.profile, CompatibilityProfileId::EmueraSkiaSnake) && self.policy_version >= 10
    }

    /// Whole-project snake source convergence semantics are fixed by policy v11.
    #[must_use]
    pub const fn supports_snake_compile_convergence(&self) -> bool {
        matches!(self.profile, CompatibilityProfileId::EmueraSkiaSnake) && self.policy_version >= 11
    }

    /// Policy for non-variadic user calls; builtin signatures remain exact.
    #[must_use]
    pub const fn user_call_argument_policy(&self, strict: bool) -> UserCallArgumentPolicy {
        match self.profile {
            CompatibilityProfileId::EmueraSkiaSnake if self.policy_version >= 4 && !strict => {
                UserCallArgumentPolicy::WarnAndIgnoreExcess
            }
            _ => UserCallArgumentPolicy::RejectExcess,
        }
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
    #[allow(clippy::too_many_lines)]
    fn identities_are_explicit_and_validate_all_policy_fields() {
        let reference = CompatibilityIdentity::reference();
        let snake = CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake);
        assert_ne!(reference.digest(), snake.digest());
        assert_ne!(reference.arithmetic, snake.arithmetic);
        assert_eq!(reference.rng_algorithm, snake.rng_algorithm);
        assert!(snake.is_experimental());
        assert_eq!(snake.semantic_version, 12);
        assert_eq!(snake.policy_version, 12);
        assert_eq!(snake.save_codec, SNAKE_INTEROP_SAVE_CODEC);
        assert!(snake.uses_snake_alias_rules());
        assert!(snake.supports_safe_sql());
        assert!(!reference.uses_snake_alias_rules());
        assert!(!reference.supports_safe_sql());
        assert_eq!(
            snake.services,
            vec![
                CompatibilityServiceContract {
                    name: SQL_SERVICE_CONTRACT_NAME.into(),
                    version: u32::from(SQL_SERVICE_CONTRACT_VERSION),
                },
                CompatibilityServiceContract {
                    name: SQL_LIMITS_CONTRACT_NAME.into(),
                    version: SQL_LIMITS_CONTRACT_VERSION,
                },
                CompatibilityServiceContract {
                    name: SCENE_CONTRACT_NAME.into(),
                    version: SCENE_CONTRACT_VERSION,
                },
                CompatibilityServiceContract {
                    name: AUDIO_SERVICE_CONTRACT_NAME.into(),
                    version: AUDIO_SERVICE_CONTRACT_VERSION,
                },
            ]
        );
        for contract in [
            SQL_SERVICE_CONTRACT_NAME,
            SQL_LIMITS_CONTRACT_NAME,
            SCENE_CONTRACT_NAME,
            AUDIO_SERVICE_CONTRACT_NAME,
        ] {
            let mut different_service = snake.clone();
            different_service
                .services
                .iter_mut()
                .find(|service| service.name == contract)
                .expect("snake identity carries every registered service contract")
                .version += 1;
            assert_ne!(different_service.digest(), snake.digest());
            assert!(different_service.validate().is_err());
        }
        assert!(reference.validate().is_ok());
        assert!(snake.validate().is_ok());
        let mut unsupported = snake;
        unsupported.rng_state_version += 1;
        assert!(unsupported.validate().is_err());
        assert!("emuera.snake".parse::<CompatibilityProfileId>().is_err());
    }
}
