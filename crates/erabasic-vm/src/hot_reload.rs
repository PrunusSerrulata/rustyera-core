use std::collections::BTreeMap;

use erabasic_bytecode::{BytecodeArtifact, BytecodePatch, BytecodeStorage, Digest, apply_patch};
use erabasic_validator::{ValidatedArtifact, ValidationContext, validate_bytecode};

use crate::{GenerationId, ProgramGeneration, Vm, VmError};

#[derive(Clone, Debug)]
pub struct HotReloadPlan {
    pub(crate) base_artifact_id: Digest,
    pub(crate) target: ValidatedArtifact,
    pub added_variables: usize,
    pub removed_variables: usize,
    pub resized_variables: usize,
}

impl HotReloadPlan {
    #[must_use]
    pub fn base_artifact_id(&self) -> Digest {
        self.base_artifact_id
    }

    #[must_use]
    pub fn target_artifact_id(&self) -> Digest {
        self.target.artifact().manifest.artifact_id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotReloadReport {
    pub old_generation: GenerationId,
    pub new_generation: GenerationId,
    pub retained_generations: usize,
    pub added_variables: usize,
    pub removed_variables: usize,
    pub resized_variables: usize,
}

impl Vm {
    /// Reconstruct and validate a patch before it can affect scheduler-visible state.
    /// A failed preparation leaves the current program and all state untouched.
    ///
    /// # Errors
    ///
    /// Returns an error for a stale patch, failed bytecode validation, incompatible
    /// state migration, or an already-pending plan.
    pub fn prepare_hot_reload(
        &mut self,
        patch: &BytecodePatch,
        context: &ValidationContext,
    ) -> Result<&HotReloadPlan, VmError> {
        if self.pending_reload.is_some() {
            return Err(VmError::HotReload(
                "another hot-reload plan is already pending".into(),
            ));
        }
        let target = apply_patch(self.artifact(), patch)
            .map_err(|error| VmError::HotReload(error.to_string()))?;
        let report = validate_bytecode(target.into_unvalidated(), context);
        let target = report.value.ok_or_else(|| {
            VmError::HotReload(
                report
                    .diagnostics
                    .into_iter()
                    .map(|diagnostic| diagnostic.message)
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        })?;
        self.prepare_hot_reload_artifact(target)
    }

    /// Prepare an already validated full artifact. This is useful when the runtime
    /// receives a complete build rather than an incremental `.erbc` patch.
    ///
    /// # Errors
    ///
    /// Returns an error for an incompatible state migration or pending plan.
    pub fn prepare_hot_reload_artifact(
        &mut self,
        target: ValidatedArtifact,
    ) -> Result<&HotReloadPlan, VmError> {
        if self.pending_reload.is_some() {
            return Err(VmError::HotReload(
                "another hot-reload plan is already pending".into(),
            ));
        }
        let plan = plan_migration(self.artifact(), target)?;
        self.pending_reload = Some(plan);
        self.pending_reload
            .as_ref()
            .ok_or_else(|| VmError::HotReload("prepared plan was not retained".into()))
    }

    #[must_use]
    pub fn pending_hot_reload(&self) -> Option<&HotReloadPlan> {
        self.pending_reload.as_ref()
    }

    pub fn abort_hot_reload(&mut self) {
        self.pending_reload = None;
    }

    /// Commit only between interpreter slices. All migration work is performed on
    /// clones first, so an incompatibility or resource-limit error is atomic.
    ///
    /// # Errors
    ///
    /// Returns an error if there is no current plan, its base changed, or retaining
    /// another program generation would exceed the configured limit.
    pub fn commit_hot_reload(&mut self) -> Result<HotReloadReport, VmError> {
        self.reclaim_generations();
        let plan = self
            .pending_reload
            .as_ref()
            .ok_or_else(|| VmError::HotReload("no hot-reload plan is pending".into()))?;
        if self.artifact_id() != plan.base_artifact_id {
            return Err(VmError::HotReload(
                "the current artifact changed after hot reload was prepared".into(),
            ));
        }
        if self.generations.len() >= self.config.maximum_retained_generations {
            return Err(VmError::ResourceLimit("retained program generations"));
        }
        let target = plan.target.artifact().clone();
        let old_generation = self.current_generation;
        let old_artifact = self.artifact().clone();
        let new_generation = GenerationId(self.next_generation);
        let mut migrated = self.memory.clone();
        migrated.migrate(old_generation, &old_artifact, &target);

        self.memory = migrated;
        self.generations
            .insert(new_generation, ProgramGeneration { artifact: target });
        self.current_generation = new_generation;
        self.next_generation = self.next_generation.saturating_add(1);
        let report = HotReloadReport {
            old_generation,
            new_generation,
            retained_generations: self.generations.len(),
            added_variables: plan.added_variables,
            removed_variables: plan.removed_variables,
            resized_variables: plan.resized_variables,
        };
        self.pending_reload = None;
        self.reclaim_generations();
        Ok(HotReloadReport {
            retained_generations: self.generations.len(),
            ..report
        })
    }
}

fn plan_migration(
    base: &BytecodeArtifact,
    target: ValidatedArtifact,
) -> Result<HotReloadPlan, VmError> {
    let base_by_key: BTreeMap<_, _> = base
        .globals
        .iter()
        .map(|definition| (definition.key, definition))
        .collect();
    let target_by_key: BTreeMap<_, _> = target
        .artifact()
        .globals
        .iter()
        .map(|definition| (definition.key, definition))
        .collect();
    let mut resized = 0;
    for (key, old) in &base_by_key {
        let Some(new) = target_by_key.get(key) else {
            continue;
        };
        if old.value_type != new.value_type || old.storage != new.storage || old.owner != new.owner
        {
            return Err(VmError::HotReload(format!(
                "variable {} changed type, storage class, or owner",
                old.name
            )));
        }
        if old.dimensions != new.dimensions && old.storage != BytecodeStorage::FunctionLocal {
            resized += 1;
        }
    }
    Ok(HotReloadPlan {
        base_artifact_id: base.manifest.artifact_id,
        added_variables: target_by_key
            .keys()
            .filter(|key| !base_by_key.contains_key(key))
            .count(),
        removed_variables: base_by_key
            .keys()
            .filter(|key| !target_by_key.contains_key(key))
            .count(),
        resized_variables: resized,
        target,
    })
}
