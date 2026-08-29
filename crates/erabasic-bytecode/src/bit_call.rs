//! Bit-array calls capture their input backing before evaluating scalar operands.

use serde::{Deserialize, Serialize};

use crate::SymbolKey;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum BitOperation {
    Set = 0,
    Get = 1,
    Toggle = 2,
    IndexOfFirst = 3,
}

impl BitOperation {
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_uppercase().as_str() {
            "BITSET" => Some(Self::Set),
            "BITGET" => Some(Self::Get),
            "BITTOGGLE" => Some(Self::Toggle),
            "BITINDEXOFFIRST" => Some(Self::IndexOfFirst),
            _ => None,
        }
    }

    #[must_use]
    pub const fn maximum_tail(self) -> u8 {
        match self {
            Self::Set => 3,
            _ => 1,
        }
    }
}

/// Presence is independent of the integer value: `i64::MIN` never means omitted.
/// GET/TOGGLE's missing index is retained until execution after backing capture.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BitCallSpec {
    pub operation: BitOperation,
    pub input: SymbolKey,
    pub tail_count: u8,
    pub present: u8,
}

impl BitCallSpec {
    #[must_use]
    pub fn encode(self) -> [u8; 19] {
        let mut payload = [0; 19];
        payload[0] = self.operation as u8;
        payload[1..17].copy_from_slice(&self.input.0);
        payload[17] = self.tail_count;
        payload[18] = self.present;
        payload
    }

    /// # Errors
    /// Rejects undefined operations, trailing bytes and noncanonical presence bits.
    pub fn decode(payload: &[u8]) -> Result<Self, String> {
        if payload.len() != 19 {
            return Err("invalid bit-call payload length".into());
        }
        let operation = match payload[0] {
            0 => BitOperation::Set,
            1 => BitOperation::Get,
            2 => BitOperation::Toggle,
            3 => BitOperation::IndexOfFirst,
            _ => return Err("invalid bit-call operation".into()),
        };
        let tail_count = payload[17];
        let present = payload[18];
        if tail_count > operation.maximum_tail()
            || present >> tail_count != 0
            || operation == BitOperation::Set && (tail_count == 0 || present & 1 == 0)
        {
            return Err("invalid bit-call argument presence".into());
        }
        let input = payload[1..17]
            .try_into()
            .map_err(|_| "invalid bit-call input identity")?;
        Ok(Self {
            operation,
            input: SymbolKey(input),
            tail_count,
            present,
        })
    }

    #[must_use]
    pub const fn evaluated_arguments(self) -> usize {
        self.present.count_ones() as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omission_has_a_wire_identity_separate_from_explicit_integer_operands() {
        let key = SymbolKey::derive("bit-call-test", b"array");
        let omitted = BitCallSpec {
            operation: BitOperation::Set,
            input: key,
            tail_count: 3,
            present: 1,
        };
        let present = BitCallSpec {
            present: 7,
            ..omitted
        };
        assert_eq!(BitCallSpec::decode(&omitted.encode()), Ok(omitted));
        assert_eq!(BitCallSpec::decode(&present.encode()), Ok(present));
        assert_eq!(omitted.evaluated_arguments(), 1);
        assert_eq!(present.evaluated_arguments(), 3);
        let mut invalid = omitted.encode();
        invalid[18] = 8;
        assert!(BitCallSpec::decode(&invalid).is_err());
        invalid[18] = 0;
        assert!(BitCallSpec::decode(&invalid).is_err());
        let missing_get = BitCallSpec {
            operation: BitOperation::Get,
            input: key,
            tail_count: 0,
            present: 0,
        };
        assert_eq!(BitCallSpec::decode(&missing_get.encode()), Ok(missing_get));
    }
}
