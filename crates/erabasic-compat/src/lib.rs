//! Versioned language identity shared by every stage of the runtime pipeline.
//!
//! A requested dialect is separate from the policies actually implemented. The snake profile
//! remains experimental while its arithmetic policy is shared by analysis and execution.

use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

mod calls;
mod integer;
mod ordinal_casing;
pub use ordinal_casing::OrdinalCasing;

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
            CompatibilityProfileId::EmueraEm => 3,
            CompatibilityProfileId::EmueraSkiaSnake => 15,
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

    const fn supports_snake_policy(&self, minimum_version: u32) -> bool {
        self.is_experimental() && self.policy_version >= minimum_version
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
        self.supports_snake_policy(3)
    }

    /// Original v3 and snake v13 count legacy strings by round-tripped UTF-16 units.
    #[must_use]
    pub const fn uses_utf16_legacy_counting(&self) -> bool {
        (matches!(self.profile, CompatibilityProfileId::EmueraEm) && self.policy_version >= 3)
            || self.supports_snake_policy(13)
    }

    /// Snake v14 clamps invalid ordinary integer RAND arguments without taking a sample.
    #[must_use]
    pub const fn clamps_integer_rand(&self) -> bool {
        self.supports_snake_policy(14)
    }

    /// Snake v15 merges preset ERD names and prices before reverse lookup construction.
    #[must_use]
    pub const fn supports_snake_preset_erd(&self) -> bool {
        self.supports_snake_policy(15)
    }

    /// Snake v13 shares the original save-version check and result contract.
    #[must_use]
    pub const fn uses_save_check_version(&self) -> bool {
        matches!(self.profile, CompatibilityProfileId::EmueraEm) || self.supports_snake_policy(13)
    }

    /// Complete call text and checked forms share the v4 execution contract.
    #[must_use]
    pub const fn supports_call_text(&self) -> bool {
        self.supports_snake_policy(4)
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
        self.supports_snake_policy(6)
    }

    #[must_use]
    pub const fn supports_map_extensions(&self) -> bool {
        self.supports_snake_data_apis()
    }

    /// Final script-fault hooks are part of the v7 snake execution policy.
    #[must_use]
    pub const fn supports_fault_hooks(&self) -> bool {
        self.supports_snake_policy(7)
    }

    /// Normalized history display state and logical animation timers are part of policy v8.
    #[must_use]
    pub const fn supports_snake_display_state(&self) -> bool {
        self.supports_snake_policy(8)
    }

    /// Runtime-owned input control, device latches, and environment queries are policy v9.
    #[must_use]
    pub const fn supports_snake_input(&self) -> bool {
        self.supports_snake_policy(9)
    }

    /// Safe SQL catalog and service identity are part of snake policy v10.
    #[must_use]
    pub const fn supports_safe_sql(&self) -> bool {
        self.supports_snake_policy(10)
    }

    /// Whole-project snake source convergence semantics are fixed by policy v11.
    #[must_use]
    pub const fn supports_snake_compile_convergence(&self) -> bool {
        self.supports_snake_policy(11)
    }

    /// Policy for non-variadic user calls; builtin signatures remain exact.
    #[must_use]
    pub const fn user_call_argument_policy(&self, strict: bool) -> UserCallArgumentPolicy {
        if self.supports_snake_policy(4) && !strict {
            UserCallArgumentPolicy::WarnAndIgnoreExcess
        } else {
            UserCallArgumentPolicy::RejectExcess
        }
    }

    /// User ERD aliases and the snake built-in alias recovery rules arrived in policy v2.
    #[must_use]
    pub const fn uses_snake_alias_rules(&self) -> bool {
        self.supports_snake_policy(2)
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
    fn original_upgrade_rejects_previous_identity() {
        let current = CompatibilityIdentity::reference();
        assert_eq!((current.semantic_version, current.policy_version), (3, 3));
        assert!(current.validate().is_ok());
        assert!(current.uses_utf16_legacy_counting());
        for version in [1, 2] {
            let mut previous = current.clone();
            previous.semantic_version = version;
            previous.policy_version = version;
            assert!(previous.validate().is_err());
            assert!(!previous.uses_utf16_legacy_counting());
            assert_ne!(previous.digest(), current.digest());
        }
        let snake = CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake);
        assert_eq!((snake.semantic_version, snake.policy_version), (15, 15));
        assert!(snake.validate().is_ok());
        assert!(snake.uses_utf16_legacy_counting());
    }

    #[test]
    fn snake_upstream_upgrade_rejects_previous_identity() {
        let current = CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake);
        assert!(current.uses_utf16_legacy_counting());
        assert!(current.uses_save_check_version());
        assert!(CompatibilityIdentity::reference().uses_save_check_version());
        for version in 1..13 {
            let mut previous = current.clone();
            previous.semantic_version = version;
            previous.policy_version = version;
            assert!(previous.validate().is_err());
            assert!(!previous.uses_utf16_legacy_counting());
            assert!(!previous.uses_save_check_version());
            assert_ne!(previous.digest(), current.digest());
        }
    }

    #[test]
    fn integer_rand_policy_has_an_explicit_identity_boundary() {
        let mut snake = CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake);
        assert!(snake.clamps_integer_rand());
        assert!(!CompatibilityIdentity::reference().clamps_integer_rand());
        snake.semantic_version = 14;
        snake.policy_version = 14;
        assert!(snake.clamps_integer_rand());
        snake.semantic_version = 13;
        snake.policy_version = 13;
        assert!(!snake.clamps_integer_rand());
        assert!(snake.validate().is_err());
    }

    #[test]
    fn preset_erd_policy_has_an_independent_v15_boundary() {
        let current = CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake);
        assert_eq!((current.semantic_version, current.policy_version), (15, 15));
        assert!(current.supports_snake_preset_erd());
        assert!(current.validate().is_ok());
        assert!(!CompatibilityIdentity::reference().supports_snake_preset_erd());
        for (semantic, policy) in [(14, 14), (14, 15), (15, 14)] {
            let mut old = current.clone();
            old.semantic_version = semantic;
            old.policy_version = policy;
            if semantic == 14 && policy == 14 {
                assert!(!old.supports_snake_preset_erd());
            }
            assert!(old.validate().is_err());
            assert_ne!(old.digest(), current.digest());
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn identities_are_explicit_and_validate_all_policy_fields() {
        let reference = CompatibilityIdentity::reference();
        let snake = CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake);
        assert_ne!(reference.digest(), snake.digest());
        assert_ne!(reference.arithmetic, snake.arithmetic);
        assert_eq!(reference.rng_algorithm, snake.rng_algorithm);
        assert!(snake.is_experimental());
        assert_eq!(snake.semantic_version, 15);
        assert_eq!(snake.policy_version, 15);
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
