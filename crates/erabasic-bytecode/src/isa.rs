use std::{fmt, ops::Deref};

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{SeqAccess, Visitor},
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BytecodeType {
    Integer,
    String,
    IntegerPlace,
    StringPlace,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum Opcode {
    Nop = 0,
    PushInteger = 1,
    PushString = 2,
    LoadVariable = 3,
    StoreVariable = 4,
    Unary = 5,
    Binary = 6,
    ToString = 7,
    Concat = 8,
    MakePlace = 9,
    Pop = 10,
    Dup = 11,
    StorePlace = 12,
    Jump = 16,
    JumpIfFalse = 17,
    ForStart = 18,
    ForNext = 19,
    SelectStart = 20,
    SelectCompare = 21,
    SelectEnd = 22,
    ForBreak = 23,
    Call = 32,
    Return = 33,
    CallNative = 34,
    CallHost = 35,
    ResolveFunction = 36,
    InvokeDynamic = 37,
    JumpDynamicLabel = 38,
    InvokeEvent = 39,
    Yield = 48,
    AwaitResume = 49,
    Trap = 255,
}

impl TryFrom<u16> for Opcode {
    type Error = u16;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Ok(match value {
            0 => Self::Nop,
            1 => Self::PushInteger,
            2 => Self::PushString,
            3 => Self::LoadVariable,
            4 => Self::StoreVariable,
            5 => Self::Unary,
            6 => Self::Binary,
            7 => Self::ToString,
            8 => Self::Concat,
            9 => Self::MakePlace,
            10 => Self::Pop,
            11 => Self::Dup,
            12 => Self::StorePlace,
            16 => Self::Jump,
            17 => Self::JumpIfFalse,
            18 => Self::ForStart,
            19 => Self::ForNext,
            20 => Self::SelectStart,
            21 => Self::SelectCompare,
            22 => Self::SelectEnd,
            23 => Self::ForBreak,
            32 => Self::Call,
            33 => Self::Return,
            34 => Self::CallNative,
            35 => Self::CallHost,
            36 => Self::ResolveFunction,
            37 => Self::InvokeDynamic,
            38 => Self::JumpDynamicLabel,
            39 => Self::InvokeEvent,
            48 => Self::Yield,
            49 => Self::AwaitResume,
            255 => Self::Trap,
            unknown => return Err(unknown),
        })
    }
}

const INLINE_PAYLOAD_BYTES: usize = 19;

/// Compact instruction operand storage.
///
/// `EraBasic`'s common encoded operands are at most 19 bytes. Keeping those bytes in the
/// instruction avoids millions of individual heap allocations when a persisted artifact is
/// deserialized, while the custom serde representation remains the same byte sequence used by
/// existing containers and artifact identities.
#[derive(Clone, Eq, PartialEq)]
pub struct InstructionPayload(InstructionPayloadStorage);

#[derive(Clone, Eq, PartialEq)]
enum InstructionPayloadStorage {
    Inline {
        bytes: [u8; INLINE_PAYLOAD_BYTES],
        length: u8,
    },
    Heap(Box<[u8]>),
}

impl InstructionPayload {
    #[must_use]
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        match &self.0 {
            InstructionPayloadStorage::Inline { bytes, length } => &bytes[..usize::from(*length)],
            InstructionPayloadStorage::Heap(bytes) => bytes,
        }
    }

    fn from_slice(value: &[u8]) -> Self {
        if value.len() <= INLINE_PAYLOAD_BYTES {
            let mut bytes = [0; INLINE_PAYLOAD_BYTES];
            bytes[..value.len()].copy_from_slice(value);
            Self(InstructionPayloadStorage::Inline {
                bytes,
                length: u8::try_from(value.len()).expect("inline payload length fits in u8"),
            })
        } else {
            Self(InstructionPayloadStorage::Heap(value.into()))
        }
    }
}

impl From<Vec<u8>> for InstructionPayload {
    fn from(value: Vec<u8>) -> Self {
        if value.len() <= INLINE_PAYLOAD_BYTES {
            let mut bytes = [0; INLINE_PAYLOAD_BYTES];
            bytes[..value.len()].copy_from_slice(&value);
            Self(InstructionPayloadStorage::Inline {
                bytes,
                length: u8::try_from(value.len()).expect("inline payload length fits in u8"),
            })
        } else {
            Self(InstructionPayloadStorage::Heap(value.into_boxed_slice()))
        }
    }
}

impl Default for InstructionPayload {
    fn default() -> Self {
        Self::from(Vec::new())
    }
}

impl Deref for InstructionPayload {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl AsRef<[u8]> for InstructionPayload {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl fmt::Debug for InstructionPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(self.as_slice()).finish()
    }
}

impl Serialize for InstructionPayload {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(self.as_slice())
    }
}

impl<'de> Deserialize<'de> for InstructionPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PayloadVisitor;

        impl<'de> Visitor<'de> for PayloadVisitor {
            type Value = InstructionPayload;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an instruction operand byte sequence")
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut inline = [0; INLINE_PAYLOAD_BYTES];
                let mut length = 0;
                while let Some(byte) = sequence.next_element()? {
                    if length < INLINE_PAYLOAD_BYTES {
                        inline[length] = byte;
                        length += 1;
                        continue;
                    }
                    let mut heap = Vec::with_capacity(
                        sequence
                            .size_hint()
                            .map_or(length + 1, |remaining| length + remaining + 1),
                    );
                    heap.extend_from_slice(&inline);
                    heap.push(byte);
                    while let Some(byte) = sequence.next_element()? {
                        heap.push(byte);
                    }
                    return Ok(InstructionPayload(InstructionPayloadStorage::Heap(
                        heap.into_boxed_slice(),
                    )));
                }
                Ok(InstructionPayload(InstructionPayloadStorage::Inline {
                    bytes: inline,
                    length: u8::try_from(length).expect("inline payload length fits in u8"),
                }))
            }

            fn visit_bytes<E>(self, value: &[u8]) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(InstructionPayload::from_slice(value))
            }

            fn visit_borrowed_bytes<E>(self, value: &'de [u8]) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(InstructionPayload::from_slice(value))
            }

            fn visit_byte_buf<E>(self, value: Vec<u8>) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(InstructionPayload::from(value))
            }
        }

        deserializer.deserialize_bytes(PayloadVisitor)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EncodedInstruction {
    pub opcode: u16,
    pub payload: InstructionPayload,
}

impl EncodedInstruction {
    #[must_use]
    pub fn new(opcode: Opcode, payload: Vec<u8>) -> Self {
        Self {
            opcode: opcode as u16,
            payload: payload.into(),
        }
    }

    #[must_use]
    pub fn encoded_len(&self) -> u64 {
        2 + 4 + self.payload.len() as u64
    }
}

/// Canonical payload constructors shared by the compiler and tests.
pub mod opcode {
    use crate::{BytecodeType, EncodedInstruction, Opcode, SymbolKey};

    #[must_use]
    pub fn push_integer(value: i64) -> EncodedInstruction {
        EncodedInstruction::new(Opcode::PushInteger, value.to_le_bytes().to_vec())
    }

    #[must_use]
    #[allow(clippy::missing_panics_doc)]
    pub fn push_string(value: &str) -> EncodedInstruction {
        let mut payload = Vec::with_capacity(4 + value.len());
        let length = u32::try_from(value.len()).expect("one bytecode string exceeds 4 GiB");
        payload.extend_from_slice(&length.to_le_bytes());
        payload.extend_from_slice(value.as_bytes());
        EncodedInstruction::new(Opcode::PushString, payload)
    }

    #[must_use]
    pub fn variable(
        opcode: Opcode,
        key: SymbolKey,
        indices: u16,
        operation: u8,
    ) -> EncodedInstruction {
        let mut payload = Vec::with_capacity(19);
        payload.extend_from_slice(&key.0);
        payload.extend_from_slice(&indices.to_le_bytes());
        payload.push(operation);
        EncodedInstruction::new(opcode, payload)
    }

    #[must_use]
    pub fn unary(operation: u8) -> EncodedInstruction {
        EncodedInstruction::new(Opcode::Unary, vec![operation])
    }

    #[must_use]
    pub fn binary(operation: u8) -> EncodedInstruction {
        EncodedInstruction::new(Opcode::Binary, vec![operation])
    }

    #[must_use]
    pub fn jump(opcode: Opcode, instruction: u32) -> EncodedInstruction {
        EncodedInstruction::new(opcode, instruction.to_le_bytes().to_vec())
    }

    #[must_use]
    pub fn call(
        opcode: Opcode,
        import: u32,
        arguments: u16,
        result: Option<BytecodeType>,
    ) -> EncodedInstruction {
        let mut payload = Vec::with_capacity(7);
        payload.extend_from_slice(&import.to_le_bytes());
        payload.extend_from_slice(&arguments.to_le_bytes());
        payload.push(result.map_or(u8::MAX, type_tag));
        EncodedInstruction::new(opcode, payload)
    }

    #[must_use]
    pub fn return_value(has_value: bool) -> EncodedInstruction {
        EncodedInstruction::new(Opcode::Return, vec![u8::from(has_value)])
    }

    #[must_use]
    pub fn resolve_function(
        missing_target: u32,
        allow_missing: bool,
        method: bool,
    ) -> EncodedInstruction {
        let mut payload = missing_target.to_le_bytes().to_vec();
        payload.push(u8::from(allow_missing));
        payload.push(u8::from(method));
        EncodedInstruction::new(Opcode::ResolveFunction, payload)
    }

    #[must_use]
    pub fn invoke_dynamic(arguments: u16, tail: bool) -> EncodedInstruction {
        let mut payload = arguments.to_le_bytes().to_vec();
        payload.push(u8::from(tail));
        EncodedInstruction::new(Opcode::InvokeDynamic, payload)
    }

    #[must_use]
    pub fn jump_dynamic_label(missing_target: u32) -> EncodedInstruction {
        EncodedInstruction::new(
            Opcode::JumpDynamicLabel,
            missing_target.to_le_bytes().to_vec(),
        )
    }

    #[must_use]
    pub fn invoke_event() -> EncodedInstruction {
        EncodedInstruction::new(Opcode::InvokeEvent, Vec::new())
    }

    #[must_use]
    pub fn concat(parts: u16) -> EncodedInstruction {
        EncodedInstruction::new(Opcode::Concat, parts.to_le_bytes().to_vec())
    }

    #[must_use]
    pub fn type_tag(value_type: BytecodeType) -> u8 {
        match value_type {
            BytecodeType::Integer => 0,
            BytecodeType::String => 1,
            BytecodeType::IntegerPlace => 2,
            BytecodeType::StringPlace => 3,
        }
    }

    #[must_use]
    pub fn decode_type(tag: u8) -> Option<BytecodeType> {
        Some(match tag {
            0 => BytecodeType::Integer,
            1 => BytecodeType::String,
            2 => BytecodeType::IntegerPlace,
            3 => BytecodeType::StringPlace,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SymbolKey;

    #[test]
    fn compact_payload_preserves_the_existing_wire_shape() {
        let short_json =
            r#"{"opcode":3,"payload":[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18]}"#;
        let short: EncodedInstruction = serde_json::from_str(short_json).unwrap();
        assert_eq!(short.payload.as_slice(), &(0_u8..19).collect::<Vec<_>>());
        assert_eq!(serde_json::to_string(&short).unwrap(), short_json);

        let long_bytes = (0_u8..32).collect::<Vec<_>>();
        let long = EncodedInstruction::new(Opcode::Trap, long_bytes.clone());
        let round_trip: EncodedInstruction =
            serde_json::from_slice(&serde_json::to_vec(&long).unwrap()).unwrap();
        assert_eq!(round_trip.payload.as_slice(), long_bytes);
    }

    #[test]
    fn common_instruction_payloads_remain_inline_sized() {
        assert!(std::mem::size_of::<InstructionPayload>() <= 24);
        assert!(std::mem::size_of::<EncodedInstruction>() <= 32);
        assert_eq!(
            opcode::variable(Opcode::LoadVariable, SymbolKey([7; 16]), 1, 0)
                .payload
                .len(),
            INLINE_PAYLOAD_BYTES
        );
    }
}
