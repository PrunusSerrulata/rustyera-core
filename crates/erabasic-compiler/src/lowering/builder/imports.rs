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
        let classification = if extension {
            self.context
                .host_registry
                .classification(name)
                .cloned()
                .unwrap_or_else(|| ExecutionBinding::Host(extension_binding(name)))
        } else {
            self.context
                .host_registry
                .classification(name)
                .cloned()
                .unwrap_or(ExecutionBinding::Unsupported {
                    reason: "the callable has no execution catalog entry".into(),
                })
        };
        if let ExecutionBinding::Host(binding) = classification {
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
        } else if let ExecutionBinding::Native(contract) = classification {
            self.emit_native_call(name, parameters, result, contract, location);
        } else if let ExecutionBinding::Unsupported { reason } = classification {
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
    }

    pub(in super::super) fn emit_native_call(
        &mut self,
        name: &str,
        parameters: &[BytecodeType],
        result: Option<BytecodeType>,
        contract: erabasic_bytecode::OperationContract,
        location: SourceLocation,
    ) {
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
        if let Some(index) = self
            .imports
            .iter()
            .position(|import| import.kind == kind && import.key == key)
        {
            return u32::try_from(index).unwrap_or(u32::MAX);
        }
        let index = self.imports.len();
        self.imports.push(FunctionImport { kind, key });
        u32::try_from(index).unwrap_or(u32::MAX)
    }
}
