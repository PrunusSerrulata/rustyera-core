//! MATCH's token capture and ordered scalar phases. Indices are intentionally absent.
use crate::{BytecodeType, SymbolKey};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MatchInput {
    Variable(SymbolKey),
    Name(String),
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MatchCallSpec {
    pub input: MatchInput,
    /// The reference replaces an indexed CONST token with its scalar during
    /// `Restructure`, then faults when MATCHALL later requires the token.
    pub input_restructured_to_scalar: bool,
    pub output: Option<SymbolKey>,
    pub needle: BytecodeType,
    pub begin_type: BytecodeType,
    pub end_type: Option<BytecodeType>,
}
impl MatchCallSpec {
    /// # Panics
    /// Panics only if a runtime variable name exceeds the `u32` wire limit.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut data = Vec::new();
        match &self.input {
            MatchInput::Variable(key) => {
                data.push(0);
                data.extend_from_slice(&key.0);
            }
            MatchInput::Name(name) => {
                data.push(1);
                let length = u32::try_from(name.len()).expect("MATCH name fits the wire limit");
                data.extend_from_slice(&length.to_le_bytes());
                data.extend_from_slice(name.as_bytes());
            }
        }
        data.push(u8::from(self.input_restructured_to_scalar));
        data.push(u8::from(self.output.is_some()));
        if let Some(key) = self.output {
            data.extend_from_slice(&key.0);
        }
        data.push(scalar_tag(self.needle));
        data.push(scalar_tag(self.begin_type));
        data.push(self.end_type.map_or(2, scalar_tag));
        data
    }
    /// # Errors
    /// Rejects truncated data, invalid tags, oversized names and trailing bytes.
    pub fn decode(data: &[u8]) -> Result<Self, String> {
        fn take<'a>(data: &'a [u8], cursor: &mut usize, length: usize) -> Result<&'a [u8], String> {
            let end = cursor
                .checked_add(length)
                .ok_or("MATCH payload size overflow")?;
            let value = data.get(*cursor..end).ok_or("MATCH payload is truncated")?;
            *cursor = end;
            Ok(value)
        }
        fn byte(data: &[u8], cursor: &mut usize) -> Result<u8, String> {
            Ok(take(data, cursor, 1)?[0])
        }
        fn key(data: &[u8], cursor: &mut usize) -> Result<SymbolKey, String> {
            take(data, cursor, 16)?
                .try_into()
                .map(SymbolKey)
                .map_err(|_| "MATCH symbol identity is truncated".into())
        }
        let mut cursor = 0;
        let input = match byte(data, &mut cursor)? {
            0 => MatchInput::Variable(key(data, &mut cursor)?),
            1 => {
                let length_bytes = take(data, &mut cursor, 4)?
                    .try_into()
                    .map_err(|_| "MATCH name length is truncated")?;
                let length = u32::from_le_bytes(length_bytes) as usize;
                if length > 1024 * 1024 {
                    return Err("MATCH name exceeds payload limit".into());
                }
                MatchInput::Name(
                    std::str::from_utf8(take(data, &mut cursor, length)?)
                        .map_err(|_| "MATCH name is not UTF-8")?
                        .to_owned(),
                )
            }
            _ => return Err("MATCH input tag is invalid".into()),
        };
        let input_restructured_to_scalar = match byte(data, &mut cursor)? {
            0 => false,
            1 => true,
            _ => return Err("MATCH input restructure tag is invalid".into()),
        };
        let output = match byte(data, &mut cursor)? {
            0 => None,
            1 => Some(key(data, &mut cursor)?),
            _ => return Err("MATCH output tag is invalid".into()),
        };
        let needle = scalar_type(byte(data, &mut cursor)?)?;
        let begin_type = scalar_type(byte(data, &mut cursor)?)?;
        let end_type = match byte(data, &mut cursor)? {
            2 => None,
            tag => Some(scalar_type(tag)?),
        };
        if cursor != data.len() {
            return Err("MATCH payload has trailing bytes".into());
        }
        Ok(Self {
            input,
            input_restructured_to_scalar,
            output,
            needle,
            begin_type,
            end_type,
        })
    }
}
fn scalar_tag(value: BytecodeType) -> u8 {
    match value {
        BytecodeType::Integer => 0,
        BytecodeType::String => 1,
        _ => 255,
    }
}
fn scalar_type(tag: u8) -> Result<BytecodeType, String> {
    match tag {
        0 => Ok(BytecodeType::Integer),
        1 => Ok(BytecodeType::String),
        _ => Err("MATCH operand is not scalar".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn match_wire_round_trip_and_rejects_truncation_and_unknown_scalar() {
        let spec = MatchCallSpec {
            input: MatchInput::Name("変数".into()),
            input_restructured_to_scalar: false,
            output: Some(SymbolKey([7; 16])),
            needle: BytecodeType::String,
            begin_type: BytecodeType::Integer,
            end_type: None,
        };
        let bytes = spec.encode();
        assert_eq!(MatchCallSpec::decode(&bytes).unwrap(), spec);
        for end in 0..bytes.len() {
            assert!(MatchCallSpec::decode(&bytes[..end]).is_err());
        }
        let mut invalid = bytes;
        let last = invalid.len() - 1;
        invalid[last] = 255;
        assert!(MatchCallSpec::decode(&invalid).is_err());
    }
}
