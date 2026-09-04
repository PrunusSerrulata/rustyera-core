#[allow(clippy::wildcard_imports)]
use super::*;
#[derive(Clone)]
pub(super) struct CharacterWidthModeHandle(Arc<AtomicU8>);

impl Default for CharacterWidthModeHandle {
    fn default() -> Self {
        Self(Arc::new(AtomicU8::new(0)))
    }
}

impl CharacterWidthModeHandle {
    pub(super) fn get(&self) -> crate::CharacterWidthMode {
        match self.0.load(Ordering::Relaxed) {
            1 => crate::CharacterWidthMode::AmbiguousNarrow,
            2 => crate::CharacterWidthMode::AmbiguousWide,
            _ => crate::CharacterWidthMode::Automatic,
        }
    }

    fn set(&self, mode: crate::CharacterWidthMode) {
        let value = match mode {
            crate::CharacterWidthMode::Automatic => 0,
            crate::CharacterWidthMode::AmbiguousNarrow => 1,
            crate::CharacterWidthMode::AmbiguousWide => 2,
        };
        self.0.store(value, Ordering::Relaxed);
    }
}

type SymbolServiceMap =
    HashMap<SymbolKey, Box<dyn NativeService>, BuildHasherDefault<SymbolKeyHasher>>;
type SymbolKeySet = HashSet<SymbolKey, BuildHasherDefault<SymbolKeyHasher>>;

#[derive(Default)]
pub struct NativeServiceRegistry {
    pub(super) services: SymbolServiceMap,
    pub(super) path_memo_safe_keys: SymbolKeySet,
    pub(super) random: Option<Arc<Mutex<Sfmt19937>>>,
    pub(super) structured: Option<Arc<Mutex<StructuredState>>>,
    pub(super) structured_keys: SymbolKeySet,
    pub(super) staged_map_keys: SymbolKeySet,
    /// Parent VM roots retained only while this registry belongs to an isolated candidate.
    pub(super) protected_map_leases: BTreeSet<crate::structured::MapLease>,
    pub(super) extensions: erabasic_data::ExtensionData,
    pub(super) character_width_mode: CharacterWidthModeHandle,
}

type PreparedStructuredImport = (Option<Vec<u8>>, BTreeSet<(u8, String)>);

impl NativeServiceRegistry {
    pub(crate) fn fork_for_artifact(&self, artifact: &BytecodeArtifact) -> Result<Self, String> {
        let snapshots = self.snapshots()?;
        let mut fork = Self::for_artifact(artifact);
        fork.restore_snapshots(&snapshots)?;
        fork.set_character_width_mode(self.character_width_mode.get());
        Ok(fork)
    }

    /// Register the small VM-native services emitted directly by the compiler.
    /// Project-specific builtins remain explicit services and fail closed when absent.
    #[must_use]
    pub fn for_artifact(artifact: &BytecodeArtifact) -> Self {
        Self::for_artifact_with_seed(artifact, 0)
    }

    #[must_use]
    pub fn for_artifact_with_seed(artifact: &BytecodeArtifact, seed: u64) -> Self {
        let mut registry = Self {
            extensions: artifact.project_data.static_data.extensions.clone(),
            ..Self::default()
        };
        let random = Arc::new(Mutex::new(Sfmt19937::new(seed)));
        let structured = Arc::new(Mutex::new(StructuredState::default()));
        registry.random = Some(Arc::clone(&random));
        let services = artifact
            .native_imports
            .iter()
            .map(|native| (native.import.key, native.import.name.as_str()))
            .chain(
                artifact
                    .runtime_native_authorizations
                    .iter()
                    .map(|family| (family.key, family.name.as_str())),
            );
        for (service_key, name) in services {
            if is_structured_name(name) {
                registry.structured = Some(Arc::clone(&structured));
                registry.structured_keys.insert(service_key);
                registry.register(
                    service_key,
                    StructuredNative::new(name, Arc::clone(&structured)),
                );
                if erabasic_bytecode::MapCallKind::from_name(name).is_some() {
                    registry.staged_map_keys.insert(service_key);
                }
            } else if matches!(name, "format_integer" | "format_string" | "times")
                || name.starts_with("control_")
            {
                registry.register(
                    service_key,
                    CompilerNative {
                        name: name.into(),
                        character_width_mode: registry.character_width_mode.clone(),
                    },
                );
                if compiler_native_path_memo_safe(name) {
                    registry.path_memo_safe_keys.insert(service_key);
                }
            } else if matches!(name, "rand" | "randomize" | "initrand" | "dumprand") {
                registry.register(
                    service_key,
                    RandomNative {
                        name: name.into(),
                        state: Arc::clone(&random),
                    },
                );
            } else if core_native_name(name) {
                registry.register(
                    service_key,
                    CoreNative::new(
                        name.into(),
                        artifact.project_data.static_data.legacy_encoding,
                    )
                    .with_compatibility(&artifact.manifest.compatibility),
                );
                registry.path_memo_safe_keys.insert(service_key);
            }
        }
        registry
    }

    pub fn register(&mut self, key: SymbolKey, service: impl NativeService + 'static) -> bool {
        self.staged_map_keys.remove(&key);
        self.path_memo_safe_keys.remove(&key);
        self.services.insert(key, Box::new(service)).is_none()
    }

    pub(crate) fn set_character_width_mode(&mut self, mode: crate::CharacterWidthMode) {
        self.character_width_mode.set(mode);
    }

    pub(crate) fn character_width_mode(&self) -> crate::CharacterWidthMode {
        self.character_width_mode.get()
    }

    pub(crate) fn call(
        &mut self,
        key: SymbolKey,
        request: NativeCallRequest,
    ) -> Result<NativeReady, ExecutionFailure> {
        if request.service_key != key
            || request
                .omitted_arguments
                .iter()
                .any(|index| *index >= request.arguments.len())
        {
            return Err(native_contract_failure(
                "Native request service key differs from registry key",
            ));
        }
        self.services
            .get_mut(&key)
            .ok_or_else(|| {
                native_contract_failure(format!("native service {key:?} is not registered"))
            })?
            .call(request)
    }

    pub(crate) fn contains(&self, key: SymbolKey) -> bool {
        self.services.contains_key(&key)
    }

    pub(crate) fn path_memo_safe(&self, key: SymbolKey) -> bool {
        self.path_memo_safe_keys.contains(&key)
    }

    pub(crate) fn implicit_place_names(
        &self,
        key: SymbolKey,
    ) -> Result<&'static [&'static str], String> {
        self.services
            .get(&key)
            .map(|service| service.implicit_place_names())
            .ok_or_else(|| format!("native service {key:?} is not registered"))
    }

    pub(crate) fn checkpoint(&self, key: SymbolKey) -> Result<Option<Vec<u8>>, String> {
        let service = self
            .services
            .get(&key)
            .ok_or_else(|| format!("native service {key:?} is not registered"))?;
        if service.requires_rollback_checkpoint() {
            service.snapshot()
        } else {
            Ok(None)
        }
    }

    pub(crate) fn rollback(&mut self, key: SymbolKey, state: &[u8]) -> Result<(), String> {
        self.services
            .get_mut(&key)
            .ok_or_else(|| format!("native service {key:?} is not registered"))?
            .restore(state)
    }

    pub(crate) fn prepare_structured_transaction(
        &self,
        transaction: &crate::VmRuntimeStateTransaction,
    ) -> Result<Option<Vec<u8>>, String> {
        let Some(structured) = &self.structured else {
            return Ok(None);
        };
        let mut candidate = structured
            .lock()
            .map_err(|_| "structured native state lock is poisoned".to_owned())?
            .clone();
        candidate
            .clear_for_transaction(&self.extensions, transaction)
            .map_err(|failure| failure.to_string())?;
        candidate.encode().map(Some)
    }

    pub(crate) fn prepare_structured_import(
        &self,
        transaction: &crate::VmRuntimeStateTransaction,
        scope: StructuredScope,
        values: &[StructuredExtension],
    ) -> Result<PreparedStructuredImport, String> {
        let Some(structured) = &self.structured else {
            return Ok((None, BTreeSet::new()));
        };
        let mut candidate = structured
            .lock()
            .map_err(|_| "structured native state lock is poisoned".to_owned())?
            .clone();
        candidate
            .clear_for_transaction(&self.extensions, transaction)
            .map_err(|failure| failure.to_string())?;
        let imported = candidate.import_extensions(&self.extensions, scope, values)?;
        Ok((Some(candidate.encode()?), imported))
    }

    pub(crate) fn structured_extensions(
        &self,
        scope: StructuredScope,
    ) -> Result<Vec<StructuredExtension>, String> {
        self.structured.as_ref().map_or_else(
            || Ok(Vec::new()),
            |structured| {
                let structured = structured
                    .lock()
                    .map_err(|_| "structured native state lock is poisoned".to_owned())?;
                Ok(structured.export_extensions(&self.extensions, scope))
            },
        )
    }

    pub(crate) fn column_identity_stamp(&self) -> Result<Option<ColumnIdentityStamp>, String> {
        self.structured
            .as_ref()
            .map(|state| {
                state
                    .lock()
                    .map(|state| state.column_identity_stamp())
                    .map_err(|_| "structured native state lock is poisoned".to_owned())
            })
            .transpose()
    }

    pub(crate) fn validate_column_identity_stamp(
        &self,
        expected: Option<ColumnIdentityStamp>,
    ) -> Result<(), String> {
        if self.column_identity_stamp()? != expected {
            return Err("structured state belongs to a stale column identity timeline".into());
        }
        Ok(())
    }

    pub(crate) fn commit_structured_state(
        &mut self,
        bytes: &[u8],
        expected: Option<ColumnIdentityStamp>,
        expected_maps: Option<crate::structured::MapLeaseStamp>,
    ) -> Result<(), String> {
        let decoded = StructuredState::decode(bytes)?;
        let mut state = self
            .structured
            .as_ref()
            .ok_or_else(|| "structured native bundle is not registered".to_owned())?
            .lock()
            .map_err(|_| "structured native state lock is poisoned".to_owned())?;
        if Some(state.column_identity_stamp()) != expected {
            return Err("structured state belongs to a stale column identity timeline".into());
        }
        if Some(
            state
                .map_lease_stamp()
                .map_err(|failure| failure.to_string())?,
        ) != expected_maps
        {
            return Err("structured state belongs to a stale MAP timeline".into());
        }
        *state = decoded;
        Ok(())
    }

    pub(crate) fn random_values(&self) -> Result<Vec<i64>, String> {
        self.random
            .as_ref()
            .ok_or_else(|| "random native service is not registered".to_owned())?
            .lock()
            .map(|state| state.era_values())
            .map_err(|_| "SFMT state lock is poisoned".into())
    }

    pub(crate) fn set_random_values(&mut self, values: &[i64]) -> Result<(), String> {
        self.set_random_values_execution(values)
            .map_err(|error| error.message)
    }

    pub(crate) fn set_random_values_execution(
        &mut self,
        values: &[i64],
    ) -> Result<(), crate::ExecutionFailure> {
        let candidate = Sfmt19937::from_era_values(values).map_err(|message| {
            crate::ExecutionFailure::script(
                crate::ScriptFaultKind::Argument,
                crate::VmFaultCode::Native,
                message,
            )
        })?;
        let internal = |message: &str| {
            crate::ExecutionFailure::classified(
                crate::FaultCategory::InternalInvariant,
                crate::VmFaultCode::Native,
                message,
            )
        };
        let mut state = self
            .random
            .as_ref()
            .ok_or_else(|| internal("random native service is not registered"))?
            .lock()
            .map_err(|_| internal("SFMT state lock is poisoned"))?;
        *state = candidate;
        Ok(())
    }

    pub(crate) fn snapshots(&self) -> Result<BTreeMap<SymbolKey, Vec<u8>>, String> {
        let mut snapshots = self
            .services
            .iter()
            .map(|(key, service)| {
                service
                    .snapshot()?
                    .map(|state| (*key, state))
                    .ok_or_else(|| format!("native service {key:?} is not snapshot-capable"))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        if let Some(structured) = &self.structured {
            snapshots.insert(
                bundle_key(),
                structured
                    .lock()
                    .map_err(|_| "structured native state lock is poisoned".to_owned())?
                    .encode()?,
            );
        }
        Ok(snapshots)
    }

    pub(crate) fn restore_snapshots(
        &mut self,
        states: &BTreeMap<SymbolKey, Vec<u8>>,
    ) -> Result<(), String> {
        let previous = self.snapshots()?;
        for (key, state) in states {
            let outcome = if *key == bundle_key() {
                self.structured.as_ref().map_or_else(
                    || Err("structured native bundle is not registered".into()),
                    |structured| {
                        let decoded = StructuredState::decode(state)?;
                        *structured
                            .lock()
                            .map_err(|_| "structured native state lock is poisoned".to_owned())? =
                            decoded;
                        Ok(())
                    },
                )
            } else {
                self.services.get_mut(key).map_or_else(
                    || Err(format!("native service {key:?} is not registered")),
                    |service| service.restore(state),
                )
            };
            if let Err(error) = outcome {
                for (rollback_key, rollback) in &previous {
                    if *rollback_key == bundle_key() {
                        if let (Some(structured), Ok(decoded)) =
                            (&self.structured, StructuredState::decode(rollback))
                            && let Ok(mut state) = structured.lock()
                        {
                            *state = decoded;
                        }
                    } else if let Some(service) = self.services.get_mut(rollback_key) {
                        let _ = service.restore(rollback);
                    }
                }
                return Err(error);
            }
        }
        Ok(())
    }

    /// Build the registry required by a replacement artifact while retaining every
    /// service state whose stable import identity still exists. New services start
    /// from their deterministic default; removed services are dropped only after the
    /// VM has accepted the replacement generation.
    pub(crate) fn migrated_for_artifact(
        &self,
        artifact: &BytecodeArtifact,
    ) -> Result<Self, String> {
        let previous = self.snapshots()?;
        let mut target = Self::for_artifact(artifact);
        let active_maps = self
            .structured
            .as_ref()
            .map(|state| {
                state
                    .lock()
                    .map(|state| !state.all_map_leases().is_empty())
                    .map_err(|_| "MAP state lock poisoned".to_owned())
            })
            .transpose()?
            .unwrap_or(false);
        if active_maps && !self.staged_map_keys.is_subset(&target.staged_map_keys) {
            return Err(
                "hot reload removes a MAP provider still owned by an active continuation".into(),
            );
        }
        if target.structured.is_none()
            && let Some(state) = &self.structured
        {
            let state = state
                .lock()
                .map_err(|_| "MAP state lock poisoned".to_owned())?;
            if !state.all_map_leases().is_empty() {
                target.structured = Some(Arc::new(Mutex::new(state.clone())));
            }
        }
        let retained = previous
            .into_iter()
            .filter(|(key, _)| {
                target.services.contains_key(key)
                    || (*key == bundle_key() && target.structured.is_some())
            })
            .collect();
        target.restore_snapshots(&retained)?;
        target
            .protected_map_leases
            .clone_from(&self.protected_map_leases);
        target.set_character_width_mode(self.character_width_mode.get());
        Ok(target)
    }
}

pub(super) fn compiler_native_path_memo_safe(name: &str) -> bool {
    matches!(name, "format_integer" | "format_string")
}
