//! Finite source-call authorization, separate from static physical imports and parse symbols.
use crate::{
    BytecodeType, NATIVE_ABI_VERSION, OperationContract, RuntimeBuiltinSymbol,
    RuntimeCallableShape, RuntimeExpressionShape, RuntimeImport, SymbolKey,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeNativeAuthorization {
    pub key: SymbolKey,
    pub namespace: String,
    pub name: String,
    pub abi_version: u32,
    pub result: BytecodeType,
    pub shapes: Vec<RuntimeCallableShape>,
    pub contract: OperationContract,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BoundRuntimeNative {
    /// Provider/checkpoint/rollback identity; never the physical signature key.
    pub service_key: SymbolKey,
    pub import: RuntimeImport,
    pub omitted_arguments: Vec<usize>,
}
impl RuntimeNativeAuthorization {
    #[must_use]
    pub fn new(symbol: &RuntimeBuiltinSymbol, contract: OperationContract) -> Self {
        let mut value = Self {
            key: SymbolKey::default(),
            namespace: "rustyera.vm".into(),
            name: symbol.name.to_ascii_lowercase(),
            abi_version: NATIVE_ABI_VERSION,
            result: symbol.result,
            shapes: crate::canonical_native_source_shapes(symbol),
            contract,
        };
        value.key = value.canonical_key();
        value
    }
    /// # Panics
    /// Panics if the fixed identity tuple cannot be serialized as JSON. Its
    /// current field types have infallible JSON representations.
    #[must_use]
    pub fn canonical_key(&self) -> SymbolKey {
        let bytes = serde_json::to_vec(&(
            &self.namespace,
            &self.name,
            self.abi_version,
            self.result,
            &self.shapes,
            self.contract,
        ))
        .expect("Native family identity is serializable");
        SymbolKey::derive("rustyera.bytecode.runtime-native-family.v1", &bytes)
    }
    /// Source shapes are supplied by the existing type visitor, without evaluating arguments.
    #[must_use]
    pub fn bind(&self, actuals: &[Option<RuntimeExpressionShape>]) -> Option<BoundRuntimeNative> {
        let shape = self.shapes.iter().find(|shape| shape.accepts(actuals))?;
        if !crate::native_source_relations(&self.name, actuals) {
            return None;
        }
        let binding = crate::bind_runtime_source_arguments(shape, actuals, |index, actual| {
            let constraint = shape
                .arguments
                .get(index)
                .or_else(|| shape.arguments.last());
            constraint.is_some_and(|constraint| constraint.keeps_place(actual.value_type))
                || self.name == "regexpmatch" && actuals.len() == 4 && index == 2
        })?;
        Some(self.bind_physical(binding.parameters, binding.omitted_arguments))
    }

    /// Called only after a source-shape binding has been validated.
    ///
    /// # Panics
    /// Panics if the physical import identity tuple cannot be serialized as JSON.
    /// Its current field types have infallible JSON representations.
    #[must_use]
    pub fn bind_physical(
        &self,
        parameters: Vec<BytecodeType>,
        omitted_arguments: Vec<usize>,
    ) -> BoundRuntimeNative {
        let result = Some(self.result);
        let bytes = serde_json::to_vec(&(
            &self.namespace,
            &self.name,
            self.abi_version,
            &parameters,
            result,
        ))
        .expect("physical Native identity is serializable");
        BoundRuntimeNative {
            service_key: self.key,
            omitted_arguments,
            import: RuntimeImport {
                key: SymbolKey::derive("rustyera.bytecode.runtime-import.v1", &bytes),
                namespace: self.namespace.clone(),
                name: self.name.clone(),
                abi_version: self.abi_version,
                parameters,
                result,
            },
        }
    }
}
