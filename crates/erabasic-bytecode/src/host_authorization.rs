//! Finite direct Host expression grants, separate from Native provider identities.
use crate::{
    BytecodeType, HostImport, RuntimeBuiltinSymbol, RuntimeCallableShape, RuntimeExpressionShape,
    RuntimeImport, SymbolKey,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RuntimeHostLowering {
    Eager,
    HtmlLength,
    HtmlLines,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RuntimeHostStage {
    Call,
    MeasureLength,
    LengthUnit,
    LinesBegin,
    LinesMore,
    LinesStep,
    LinesEnd,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeHostAuthorization {
    pub key: SymbolKey,
    /// Source identity and finite signatures. The physical import is derived only after binding.
    pub name: String,
    pub result: BytecodeType,
    pub shapes: Vec<RuntimeCallableShape>,
    pub prototype: HostImport,
    pub lowering: RuntimeHostLowering,
    /// Compiler-owned internal stages; never independently callable by source text.
    pub stages: Vec<(RuntimeHostStage, HostImport)>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BoundRuntimeHost {
    pub family_key: SymbolKey,
    pub import: RuntimeImport,
    pub omitted_arguments: Vec<usize>,
}
impl RuntimeHostAuthorization {
    #[must_use]
    pub fn new(
        symbol: &RuntimeBuiltinSymbol,
        shapes: Vec<RuntimeCallableShape>,
        prototype: HostImport,
        lowering: RuntimeHostLowering,
        stages: Vec<(RuntimeHostStage, HostImport)>,
    ) -> Self {
        let mut value = Self {
            key: SymbolKey::default(),
            name: symbol.name.to_ascii_lowercase(),
            result: symbol.result,
            shapes,
            prototype,
            lowering,
            stages,
        };
        value.key = value.canonical_key();
        value
    }
    /// # Panics
    /// Panics only if serialization of the scalar Host contract fails.
    #[must_use]
    pub fn canonical_key(&self) -> SymbolKey {
        let bytes = serde_json::to_vec(&(
            &self.name,
            self.result,
            &self.shapes,
            &self.prototype,
            self.lowering,
            &self.stages,
        ))
        .expect("Host family identity is serializable");
        SymbolKey::derive("rustyera.bytecode.runtime-host-family.v1", &bytes)
    }
    #[must_use]
    pub fn bind(&self, actuals: &[Option<RuntimeExpressionShape>]) -> Option<BoundRuntimeHost> {
        let shape = self.shapes.iter().find(|shape| shape.accepts(actuals))?;
        let binding = crate::bind_runtime_source_arguments(shape, actuals, |index, actual| {
            shape
                .arguments
                .get(index)
                .or_else(|| shape.arguments.last())
                .is_some_and(|constraint| constraint.keeps_place(actual.value_type))
        })?;
        let import = runtime_host_import(
            &self.prototype.import,
            binding.parameters,
            Some(self.result),
        );
        Some(BoundRuntimeHost {
            family_key: self.key,
            import,
            omitted_arguments: binding.omitted_arguments,
        })
    }
    /// The live continuation validates source binding before asking for a stage.
    #[must_use]
    pub fn stage_import(
        &self,
        bound: &BoundRuntimeHost,
        stage: RuntimeHostStage,
    ) -> Option<HostImport> {
        if bound.family_key != self.key {
            return None;
        }
        if stage == RuntimeHostStage::Call && self.lowering == RuntimeHostLowering::Eager {
            return Some(HostImport {
                import: bound.import.clone(),
                ..self.prototype.clone()
            });
        }
        self.stages
            .iter()
            .find(|(candidate, _)| *candidate == stage)
            .map(|(_, import)| import.clone())
    }
}
/// Same physical import identity domain and tuple as ordinary compiler lowering.
///
/// # Panics
/// Panics only if serialization of the scalar import signature fails.
#[must_use]
pub fn runtime_host_import(
    prototype: &RuntimeImport,
    parameters: Vec<BytecodeType>,
    result: Option<BytecodeType>,
) -> RuntimeImport {
    let bytes = serde_json::to_vec(&(
        &prototype.namespace,
        &prototype.name,
        prototype.abi_version,
        &parameters,
        result,
    ))
    .expect("physical Host identity is serializable");
    RuntimeImport {
        key: SymbolKey::derive("rustyera.bytecode.runtime-import.v1", &bytes),
        namespace: prototype.namespace.clone(),
        name: prototype.name.clone(),
        abi_version: prototype.abi_version,
        parameters,
        result,
    }
}
