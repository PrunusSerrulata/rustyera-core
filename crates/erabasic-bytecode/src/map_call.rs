//! Typed staged MAP extension signature shared by lowering, validation and VM.
use crate::BytecodeType;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MapCallKind {
    Values,
    RemoveIf,
    FindKey,
    ToString,
    FromString,
}
impl MapCallKind {
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "map_values" => Some(Self::Values),
            "map_removeif" => Some(Self::RemoveIf),
            "map_findkey" => Some(Self::FindKey),
            "map_tostring" => Some(Self::ToString),
            "map_fromstring" => Some(Self::FromString),
            _ => None,
        }
    }
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Values => "map_values",
            Self::RemoveIf => "map_removeif",
            Self::FindKey => "map_findkey",
            Self::ToString => "map_tostring",
            Self::FromString => "map_fromstring",
        }
    }
    #[must_use]
    pub const fn result_type(self) -> BytecodeType {
        match self {
            Self::RemoveIf | Self::FromString => BytecodeType::Integer,
            _ => BytecodeType::String,
        }
    }
    #[must_use]
    pub fn valid_parameters(self, parameters: &[BytecodeType]) -> bool {
        use BytecodeType::{Integer as I, String as S, StringPlace as P};
        match self {
            Self::Values => matches!(parameters, [S] | [S, I] | [S, P, I]),
            Self::RemoveIf | Self::FindKey => parameters == [S, S, S],
            Self::ToString => {
                (1..=3).contains(&parameters.len()) && parameters.iter().all(|p| *p == S)
            }
            Self::FromString => {
                (2..=4).contains(&parameters.len()) && parameters.iter().all(|p| *p == S)
            }
        }
    }
    /// Actual VM stack order, excluding the captured first name.
    #[must_use]
    pub fn materialized_parameters(self, parameters: &[BytecodeType]) -> Vec<BytecodeType> {
        if self == Self::Values && parameters.len() == 3 {
            vec![BytecodeType::Integer, BytecodeType::StringPlace]
        } else {
            parameters[1..].to_vec()
        }
    }
    #[must_use]
    pub fn implicit_places(self, arity: usize) -> &'static [&'static str] {
        match (self, arity) {
            (Self::Values, 2) => &["RESULT", "RESULTS"],
            (Self::Values, 3) | (Self::FindKey, _) => &["RESULT"],
            _ => &[],
        }
    }
}
