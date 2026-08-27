//! Typed, unevaluated argument shapes for late-bound expression methods.

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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MethodArgumentSpec {
    Omitted,
    Value(BytecodeType),
    /// Retain the variable identity before deciding whether to evaluate its indices.
    Variable(SymbolKey),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MethodCallSpec {
    pub result: MethodResult,
    pub allow_missing: bool,
    pub missing_target: u32,
    pub arguments: Vec<MethodArgumentSpec>,
}

impl MethodCallSpec {
    /// Encode the result, missing-target branch and ordered syntactic slots.
    ///
    /// # Panics
    /// Panics if the argument count exceeds the ISA's `u16` slot limit.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let count = u16::try_from(self.arguments.len()).expect("method slot count exceeds u16");
        let mut payload = Vec::with_capacity(8 + self.arguments.len());
        payload.push(opcode::type_tag(self.result.bytecode_type()));
        payload.push(u8::from(self.allow_missing));
        payload.extend_from_slice(&self.missing_target.to_le_bytes());
        payload.extend_from_slice(&count.to_le_bytes());
        for argument in &self.arguments {
            match argument {
                MethodArgumentSpec::Omitted => payload.push(0),
                MethodArgumentSpec::Value(value_type) => {
                    payload.extend_from_slice(&[1, opcode::type_tag(*value_type)]);
                }
                MethodArgumentSpec::Variable(key) => {
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
        let invalid = || "invalid expression-method resolve operand".to_owned();
        let header = payload.get(..8).ok_or_else(invalid)?;
        let result = match header[0] {
            0 => MethodResult::Integer,
            1 => MethodResult::String,
            _ => return Err(invalid()),
        };
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
                0 => MethodArgumentSpec::Omitted,
                1 => {
                    let (&tag, rest) = remaining.split_first().ok_or_else(invalid)?;
                    remaining = rest;
                    let value_type = match tag {
                        0 => BytecodeType::Integer,
                        1 => BytecodeType::String,
                        _ => return Err(invalid()),
                    };
                    MethodArgumentSpec::Value(value_type)
                }
                2 => {
                    let key = remaining.get(..16).ok_or_else(invalid)?;
                    remaining = &remaining[16..];
                    MethodArgumentSpec::Variable(SymbolKey(key.try_into().map_err(|_| invalid())?))
                }
                _ => return Err(invalid()),
            });
        }
        if !remaining.is_empty() {
            return Err(invalid());
        }
        Ok(Self {
            result,
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
        let spec = MethodCallSpec {
            result: MethodResult::String,
            allow_missing: true,
            missing_target: 31,
            arguments: vec![
                MethodArgumentSpec::Omitted,
                MethodArgumentSpec::Value(BytecodeType::Integer),
                MethodArgumentSpec::Variable(SymbolKey([7; 16])),
                MethodArgumentSpec::Value(BytecodeType::String),
            ],
        };
        let encoded = spec.encode();
        assert_eq!(MethodCallSpec::decode(&encoded), Ok(spec));
        for length in 0..encoded.len() {
            assert!(MethodCallSpec::decode(&encoded[..length]).is_err());
        }
        let mut trailing = encoded;
        trailing.push(0);
        assert!(MethodCallSpec::decode(&trailing).is_err());
    }

    #[test]
    fn method_operand_rejects_non_scalar_types_and_unknown_tags() {
        let spec = MethodCallSpec {
            result: MethodResult::Integer,
            allow_missing: false,
            missing_target: 0,
            arguments: vec![MethodArgumentSpec::Value(BytecodeType::Integer)],
        };
        for (offset, invalid_tag) in [(0, 2), (1, 2), (2, 1), (8, 3), (9, 2)] {
            let mut payload = spec.encode();
            payload[offset] = invalid_tag;
            assert!(MethodCallSpec::decode(&payload).is_err());
        }
    }
}
