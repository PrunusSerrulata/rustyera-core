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
    pub omitted_from: usize,
    pub arguments: Vec<RuntimeArgumentConstraint>,
    pub allow_omitted: bool,
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
