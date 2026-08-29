//! Parse-only expression contracts. These never grant service execution permission.
use crate::BytecodeType;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RuntimeArgumentConstraint {
    Integer,
    String,
    Any,
    MutableInteger,
    MutableString,
    MutableAny,
    ReferenceAny,
    ReferenceOrString,
    MutableReferenceOrString,
    IntegerOrReference,
    IntegerOrMutableString,
    Formatted,
    Raw,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeCallableShape {
    pub minimum: usize,
    pub maximum: Option<usize>,
    #[serde(with = "portable_omitted_from")]
    pub omitted_from: usize,
    pub arguments: Vec<RuntimeArgumentConstraint>,
    pub allow_omitted: bool,
}

mod portable_omitted_from {
    use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

    // Serde's `with` contract passes the field by reference even for Copy scalars.
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub fn serialize<S>(value: &usize, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let value = if *value == usize::MAX {
            u64::MAX
        } else {
            u64::try_from(*value).expect("usize always fits in u64")
        };
        serializer.serialize_u64(value)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<usize, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = u64::deserialize(deserializer)?;
        if value == u64::MAX {
            return Ok(usize::MAX);
        }
        usize::try_from(value).map_err(|_| D::Error::custom("omission boundary is not addressable"))
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeBuiltinSymbol {
    pub name: String,
    pub result: BytecodeType,
    pub shapes: Vec<RuntimeCallableShape>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeExpressionShape {
    pub value_type: BytecodeType,
    pub variable: bool,
    pub mutable: bool,
}
impl RuntimeCallableShape {
    #[must_use]
    pub fn accepts(&self, actuals: &[Option<RuntimeExpressionShape>]) -> bool {
        use RuntimeArgumentConstraint as C;
        if actuals.len() < self.minimum || self.maximum.is_some_and(|n| actuals.len() > n) {
            return false;
        }
        actuals.iter().enumerate().all(|(index, actual)| {
            let Some(actual) = actual else {
                return self.allow_omitted || index >= self.omitted_from;
            };
            let Some(constraint) = self.arguments.get(index).or_else(|| {
                self.maximum
                    .is_none()
                    .then(|| self.arguments.last())
                    .flatten()
            }) else {
                return false;
            };
            let integer = actual.value_type == BytecodeType::Integer;
            let string = actual.value_type == BytecodeType::String;
            match constraint {
                C::Integer | C::IntegerOrReference => integer,
                C::String => string,
                C::Any | C::Formatted | C::Raw => integer || string,
                C::MutableInteger => integer && actual.variable && actual.mutable,
                C::MutableString => string && actual.variable && actual.mutable,
                C::MutableAny => actual.variable && actual.mutable,
                C::ReferenceAny => actual.variable,
                C::ReferenceOrString => actual.variable || string,
                C::MutableReferenceOrString => {
                    if actual.variable {
                        actual.mutable
                    } else {
                        string
                    }
                }
                C::IntegerOrMutableString => {
                    integer || (string && actual.variable && actual.mutable)
                }
            }
        })
    }
}

/// Fixed token metadata required for source reconstruction (including private REF).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeVariableSymbol {
    pub key: crate::SymbolKey,
    pub reference: bool,
    pub match_name_rejection: Option<MatchNameRejectionKind>,
    pub character_disposal: CharacterArrayDisposal,
    pub reference_semantics: RuntimeReferenceSemantics,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeReferenceSemantics {
    pub is_const: bool,
    pub can_restructure: bool,
}

impl RuntimeArgumentConstraint {
    /// Exactly the analyzer's builtin value/place conversion, not a REF permission grant.
    #[must_use]
    pub fn keeps_place(self, value_type: BytecodeType) -> bool {
        matches!(
            self,
            Self::MutableInteger
                | Self::MutableString
                | Self::MutableAny
                | Self::ReferenceAny
                | Self::ReferenceOrString
                | Self::MutableReferenceOrString
        ) || self == Self::IntegerOrMutableString && value_type == BytecodeType::String
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variadic_omission_boundary_uses_a_host_neutral_sentinel() {
        let shape = RuntimeCallableShape {
            minimum: 1,
            maximum: None,
            omitted_from: usize::MAX,
            arguments: vec![RuntimeArgumentConstraint::Integer],
            allow_omitted: false,
        };
        let encoded = serde_json::to_value(&shape).unwrap();
        assert_eq!(encoded["omitted_from"], serde_json::json!(u64::MAX));
        assert_eq!(
            serde_json::from_value::<RuntimeCallableShape>(encoded).unwrap(),
            shape
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MatchNameRejectionKind {
    Script,
    Internal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CharacterArrayDisposal {
    Preserve,
    ClearSparse,
}
