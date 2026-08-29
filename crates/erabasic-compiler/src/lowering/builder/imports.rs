use crate::HostBinding;

use super::super::{
    BytecodeType, CompilerDiagnostic, CompilerDiagnosticCode, EncodedInstruction, ExecutionBinding,
    FunctionImport, HostImport, ImportKind, NATIVE_ABI_VERSION, NativeImport, Opcode,
    SourceLocation, SymbolKey, extension_binding, opcode, runtime_import,
};
use super::Builder;

impl Builder<'_> {
    pub(in super::super) fn emit_runtime_call(
        &mut self,
        name: &str,
        parameters: &[BytecodeType],
        result: Option<BytecodeType>,
        extension: bool,
        location: SourceLocation,
    ) {
        let registry = self.context.host_registry;
        match registry.classification(name) {
            Some(ExecutionBinding::Host(binding)) => {
                self.emit_host_call(name, parameters, result, binding, location);
            }
            Some(ExecutionBinding::Native(contract)) => {
                self.emit_native_call(name, parameters, result, *contract, location);
            }
            Some(
                ExecutionBinding::BitArray
                | ExecutionBinding::ArrayMatch
                | ExecutionBinding::ExpressionMethod { .. },
            ) => {
                self.diagnostics.push(CompilerDiagnostic::at(
                    CompilerDiagnosticCode::InvalidHir,
                    location,
                    "expression methods require lazy typed lowering",
                ));
                self.emit(
                    EncodedInstruction::new(Opcode::Trap, b"invalid eager method call".to_vec()),
                    location,
                );
            }
            Some(ExecutionBinding::Unsupported { reason }) => {
                self.emit_unsupported_call(name, reason, location);
            }
            None if extension => {
                let binding = extension_binding(name);
                self.emit_host_call(name, parameters, result, &binding, location);
            }
            None => self.emit_unsupported_call(
                name,
                "the callable has no execution catalog entry",
                location,
            ),
        }
    }

    pub(super) fn emit_host_call(
        &mut self,
        name: &str,
        parameters: &[BytecodeType],
        result: Option<BytecodeType>,
        binding: &HostBinding,
        location: SourceLocation,
    ) {
        if binding.contract.portability
            == erabasic_bytecode::OperationPortability::FrontendObservation
        {
            self.diagnostics.push(CompilerDiagnostic::notice_at(
                CompilerDiagnosticCode::FrontendObservation,
                location,
                format!(
                    "{name} observes the authoritative frontend environment and may vary across clients"
                ),
            ));
        }
        let key = if let Some(key) = self
            .host_imports
            .iter()
            .find(|value| {
                value.import.namespace == binding.namespace
                    && value.import.name == binding.name
                    && value.import.abi_version == binding.abi_version
                    && value.import.parameters == parameters
                    && value.import.result == result
            })
            .map(|value| value.import.key)
        {
            key
        } else {
            let import = runtime_import(
                &binding.namespace,
                &binding.name,
                binding.abi_version,
                parameters,
                result,
            );
            let key = import.key;
            if let Err(index) = self
                .host_imports
                .binary_search_by_key(&key, |value| value.import.key)
            {
                self.host_imports.insert(
                    index,
                    HostImport {
                        import,
                        effect: binding.effect,
                        capability: binding.capability,
                        snapshot_capability: binding.snapshot_capability,
                        contract: binding.contract,
                    },
                );
            }
            key
        };
        let index = self.add_import(ImportKind::Host, key);
        self.emit(
            opcode::call(
                Opcode::CallHost,
                index,
                u16::try_from(parameters.len()).unwrap_or(u16::MAX),
                result,
            ),
            location,
        );
    }

    fn emit_unsupported_call(&mut self, name: &str, reason: &str, location: SourceLocation) {
        self.diagnostics.push(CompilerDiagnostic::at(
            CompilerDiagnosticCode::UnsupportedConstruct,
            location,
            format!("{name} is unsupported: {reason}"),
        ));
        self.emit(
            EncodedInstruction::new(Opcode::Trap, format!("unsupported {name}").into_bytes()),
            location,
        );
    }

    pub(in super::super) fn emit_native_call(
        &mut self,
        name: &str,
        parameters: &[BytecodeType],
        result: Option<BytecodeType>,
        contract: erabasic_bytecode::OperationContract,
        location: SourceLocation,
    ) {
        let key = if let Some(key) = self
            .native_imports
            .iter()
            .find(|value| {
                value.import.namespace == "rustyera.vm"
                    && value.import.name.eq_ignore_ascii_case(name)
                    && value.import.abi_version == NATIVE_ABI_VERSION
                    && value.import.parameters == parameters
                    && value.import.result == result
            })
            .map(|value| value.import.key)
        {
            key
        } else {
            let import = runtime_import(
                "rustyera.vm",
                &name.to_ascii_lowercase(),
                NATIVE_ABI_VERSION,
                parameters,
                result,
            );
            let key = import.key;
            if let Err(index) = self
                .native_imports
                .binary_search_by_key(&key, |value| value.import.key)
            {
                self.native_imports.insert(
                    index,
                    NativeImport {
                        import,
                        effect: contract.effect(),
                        contract,
                    },
                );
            }
            key
        };
        let index = self.add_import(ImportKind::Native, key);
        self.emit(
            opcode::call(
                Opcode::CallNative,
                index,
                u16::try_from(parameters.len()).unwrap_or(u16::MAX),
                result,
            ),
            location,
        );
    }

    pub(in super::super) fn add_import(&mut self, kind: ImportKind, key: SymbolKey) -> u32 {
        let kind_tag = match kind {
            ImportKind::Function => 0,
            ImportKind::Native => 1,
            ImportKind::Host => 2,
        };
        if let Some(index) = self.import_indices.get(&(kind_tag, key)) {
            return *index;
        }
        let index = u32::try_from(self.imports.len()).unwrap_or(u32::MAX);
        self.imports.push(FunctionImport { kind, key });
        self.import_indices.insert((kind_tag, key), index);
        index
    }
}
