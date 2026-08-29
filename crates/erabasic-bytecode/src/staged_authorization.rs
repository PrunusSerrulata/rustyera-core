//! Authority for existing VM stages. These are neither Host requests nor Native providers.
use crate::{
    BitOperation, BytecodeType, RuntimeBuiltinSymbol, RuntimeCallableShape, RuntimeExpressionShape,
    SymbolKey,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RuntimeStagedKind {
    Bit(BitOperation),
    MatchAll,
    MatchAllEx,
}
impl RuntimeStagedKind {
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        if let Some(operation) = BitOperation::from_name(name) {
            return Some(Self::Bit(operation));
        }
        match name.to_ascii_uppercase().as_str() {
            "MATCHALL" => Some(Self::MatchAll),
            "MATCHALLEX" => Some(Self::MatchAllEx),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeStagedAuthorization {
    pub key: SymbolKey,
    pub name: String,
    pub kind: RuntimeStagedKind,
    pub result: BytecodeType,
    pub shapes: Vec<RuntimeCallableShape>,
}
impl RuntimeStagedAuthorization {
    #[must_use]
    pub fn new(symbol: &RuntimeBuiltinSymbol, kind: RuntimeStagedKind) -> Self {
        let mut value = Self {
            key: SymbolKey::default(),
            name: symbol.name.to_ascii_lowercase(),
            kind,
            result: symbol.result,
            shapes: symbol.shapes.clone(),
        };
        value.key = value.canonical_key();
        value
    }
    /// # Panics
    /// The fixed string/enum/shape identity has an infallible JSON representation.
    #[must_use]
    pub fn canonical_key(&self) -> SymbolKey {
        let bytes = serde_json::to_vec(&(&self.name, self.kind, self.result, &self.shapes))
            .expect("staged authorization identity is serializable");
        SymbolKey::derive("rustyera.bytecode.runtime-staged-family.v1", &bytes)
    }
    #[must_use]
    pub fn accepts(&self, shapes: &[Option<RuntimeExpressionShape>]) -> bool {
        self.shapes.iter().any(|shape| shape.accepts(shapes))
    }
}
