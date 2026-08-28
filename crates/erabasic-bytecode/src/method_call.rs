//! Typed, unevaluated argument shapes shared by all late-bound user calls.

use serde::{Deserialize, Serialize};

use crate::{BytecodeType, SymbolKey, opcode};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MethodResult {
    Integer,
    String,
}

impl MethodResult {
    #[must_use]
    pub fn bytecode_type(self) -> BytecodeType {
        match self {
            Self::Integer => BytecodeType::Integer,
            Self::String => BytecodeType::String,
        }
    }
}

/// Target-kind checks and completion are resolved before any actual is evaluated.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum UserCallMode {
    MethodInteger = 0,
    MethodString = 1,
    Procedure = 2,
    /// Keep the caller (including its LOCAL/REF backing) until the callee returns.
    JumpProcedure = 3,
    /// CALLFORMF accepts either method return type and discards the result.
    MethodDiscard = 4,
}

impl UserCallMode {
    #[must_use]
    pub fn expected_result(self) -> Option<BytecodeType> {
        match self {
            Self::MethodInteger => Some(BytecodeType::Integer),
            Self::MethodString => Some(BytecodeType::String),
            Self::Procedure | Self::JumpProcedure | Self::MethodDiscard => None,
        }
    }

    #[must_use]
    pub fn is_method(self) -> bool {
        matches!(
            self,
            Self::MethodInteger | Self::MethodString | Self::MethodDiscard
        )
    }

    #[must_use]
    pub fn unwinds_caller(self) -> bool {
        self == Self::JumpProcedure
    }

    /// # Errors
    /// Returns an operand error when the tag is not defined by this ISA.
    pub fn decode(tag: u8) -> Result<Self, String> {
        match tag {
            0 => Ok(Self::MethodInteger),
            1 => Ok(Self::MethodString),
            2 => Ok(Self::Procedure),
            3 => Ok(Self::JumpProcedure),
            4 => Ok(Self::MethodDiscard),
            _ => Err("invalid user-call mode".into()),
        }
    }
}

impl From<MethodResult> for UserCallMode {
    fn from(result: MethodResult) -> Self {
        match result {
            MethodResult::Integer => Self::MethodInteger,
            MethodResult::String => Self::MethodString,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum UserArgumentAdvance {
    Omitted = 0,
    Discarded = 1,
}

impl UserArgumentAdvance {
    /// # Errors
    /// Returns an operand error when the tag is not defined by this ISA.
    pub fn decode(tag: u8) -> Result<Self, String> {
        match tag {
            0 => Ok(Self::Omitted),
            1 => Ok(Self::Discarded),
            _ => Err("invalid user-call argument advance reason".into()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum UserArgumentSpec {
    Omitted,
    Value(BytecodeType),
    /// Retain the variable identity before deciding whether to evaluate its indices.
    Variable(SymbolKey),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UserCallSpec {
    pub mode: UserCallMode,
    pub allow_missing: bool,
    pub missing_target: u32,
    pub arguments: Vec<UserArgumentSpec>,
}

impl UserCallSpec {
    /// Encode the mode, missing-target branch and ordered syntactic slots.
    ///
    /// # Panics
    /// Panics if the argument count exceeds the ISA's `u16` slot limit.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let count = u16::try_from(self.arguments.len()).expect("user-call slot count exceeds u16");
        let mut payload = Vec::with_capacity(8 + self.arguments.len());
        payload.push(self.mode as u8);
        payload.push(u8::from(self.allow_missing));
        payload.extend_from_slice(&self.missing_target.to_le_bytes());
        payload.extend_from_slice(&count.to_le_bytes());
        for argument in &self.arguments {
            match argument {
                UserArgumentSpec::Omitted => payload.push(0),
                UserArgumentSpec::Value(value_type) => {
                    payload.extend_from_slice(&[1, opcode::type_tag(*value_type)]);
                }
                UserArgumentSpec::Variable(key) => {
                    payload.push(2);
                    payload.extend_from_slice(&key.0);
                }
            }
        }
        payload
    }

    /// Decode a complete operand, rejecting malformed tags, truncation and trailing bytes.
    ///
    /// # Errors
    /// Returns an operand diagnostic for an invalid encoding.
    pub fn decode(payload: &[u8]) -> Result<Self, String> {
        let invalid = || "invalid user-call resolve operand".to_owned();
        let header = payload.get(..8).ok_or_else(invalid)?;
        let mode = UserCallMode::decode(header[0])?;
        let allow_missing = match header[1] {
            0 => false,
            1 => true,
            _ => return Err(invalid()),
        };
        let missing_target = u32::from_le_bytes(header[2..6].try_into().map_err(|_| invalid())?);
        if !allow_missing && missing_target != 0 {
            return Err(invalid());
        }
        let count = usize::from(u16::from_le_bytes([header[6], header[7]]));
        let mut remaining = &payload[8..];
        if remaining.len() < count {
            return Err(invalid());
        }
        let mut arguments = Vec::with_capacity(count);
        for _ in 0..count {
            let (&tag, rest) = remaining.split_first().ok_or_else(invalid)?;
            remaining = rest;
            arguments.push(match tag {
                0 => UserArgumentSpec::Omitted,
                1 => {
                    let (&tag, rest) = remaining.split_first().ok_or_else(invalid)?;
                    remaining = rest;
                    let value_type = match tag {
                        0 => BytecodeType::Integer,
                        1 => BytecodeType::String,
                        _ => return Err(invalid()),
                    };
                    UserArgumentSpec::Value(value_type)
                }
                2 => {
                    let key = remaining.get(..16).ok_or_else(invalid)?;
                    remaining = &remaining[16..];
                    UserArgumentSpec::Variable(SymbolKey(key.try_into().map_err(|_| invalid())?))
                }
                _ => return Err(invalid()),
            });
        }
        if !remaining.is_empty() {
            return Err(invalid());
        }
        Ok(Self {
            mode,
            allow_missing,
            missing_target,
            arguments,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_slots_round_trip_without_scalar_sentinels() {
        let spec = UserCallSpec {
            mode: UserCallMode::MethodString,
            allow_missing: true,
            missing_target: 31,
            arguments: vec![
                UserArgumentSpec::Omitted,
                UserArgumentSpec::Value(BytecodeType::Integer),
                UserArgumentSpec::Variable(SymbolKey([7; 16])),
                UserArgumentSpec::Value(BytecodeType::String),
            ],
        };
        let encoded = spec.encode();
        assert_eq!(UserCallSpec::decode(&encoded), Ok(spec));
        for length in 0..encoded.len() {
            assert!(UserCallSpec::decode(&encoded[..length]).is_err());
        }
        let mut trailing = encoded;
        trailing.push(0);
        assert!(UserCallSpec::decode(&trailing).is_err());
    }

    #[test]
    fn method_operand_rejects_non_scalar_types_and_unknown_tags() {
        let spec = UserCallSpec {
            mode: UserCallMode::MethodInteger,
            allow_missing: false,
            missing_target: 0,
            arguments: vec![UserArgumentSpec::Value(BytecodeType::Integer)],
        };
        for (offset, invalid_tag) in [(0, 5), (1, 2), (2, 1), (8, 3), (9, 2)] {
            let mut payload = spec.encode();
            payload[offset] = invalid_tag;
            assert!(UserCallSpec::decode(&payload).is_err());
        }
    }
    #[test]
    fn all_user_call_modes_round_trip_and_expose_completion_contract() {
        for (mode, result, method, unwind) in [
            (
                UserCallMode::MethodInteger,
                Some(BytecodeType::Integer),
                true,
                false,
            ),
            (
                UserCallMode::MethodString,
                Some(BytecodeType::String),
                true,
                false,
            ),
            (UserCallMode::Procedure, None, false, false),
            (UserCallMode::JumpProcedure, None, false, true),
            (UserCallMode::MethodDiscard, None, true, false),
        ] {
            let spec = UserCallSpec {
                mode,
                allow_missing: false,
                missing_target: 0,
                arguments: Vec::new(),
            };
            assert_eq!(UserCallSpec::decode(&spec.encode()), Ok(spec));
            assert_eq!(mode.expected_result(), result);
            assert_eq!(mode.is_method(), method);
            assert_eq!(mode.unwinds_caller(), unwind);
        }
    }

}
