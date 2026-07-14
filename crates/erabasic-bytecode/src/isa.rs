use serde::{Deserialize, Serialize};

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
    Jump = 16,
    JumpIfFalse = 17,
    Call = 32,
    Return = 33,
    CallNative = 34,
    CallHost = 35,
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
            16 => Self::Jump,
            17 => Self::JumpIfFalse,
            32 => Self::Call,
            33 => Self::Return,
            34 => Self::CallNative,
            35 => Self::CallHost,
            48 => Self::Yield,
            49 => Self::AwaitResume,
            255 => Self::Trap,
            unknown => return Err(unknown),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EncodedInstruction {
    pub opcode: u16,
    pub payload: Vec<u8>,
}

impl EncodedInstruction {
    #[must_use]
    pub fn new(opcode: Opcode, payload: Vec<u8>) -> Self {
        Self {
            opcode: opcode as u16,
            payload,
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
