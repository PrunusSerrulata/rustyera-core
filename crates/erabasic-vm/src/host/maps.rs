//! Trusted VM-only access to the shared MAP object bundle.
use super::{
    BTreeSet, ExecutionFailure, NativeCallRequest, NativeReady, NativeServiceRegistry,
    StructuredState, SymbolKey, bundle_key, native_contract_failure,
};
use crate::structured::{MapLease, MapLeaseOwner, MapOperation};
impl NativeServiceRegistry {
    pub(crate) fn capture_map(
        &self,
        name: &str,
        owner: MapLeaseOwner,
    ) -> Result<Option<MapLease>, ExecutionFailure> {
        self.structured
            .as_ref()
            .ok_or_else(|| native_contract_failure("MAP bundle is not registered"))?
            .lock()
            .map_err(|_| native_contract_failure("MAP state lock is poisoned"))?
            .capture_map(name, owner)
    }
    pub(crate) fn apply_map(
        &self,
        operation: MapOperation,
        lease: MapLease,
        request: &NativeCallRequest,
        budget: &mut crate::compat_text::TextBudget,
    ) -> Result<NativeReady, ExecutionFailure> {
        self.structured
            .as_ref()
            .ok_or_else(|| native_contract_failure("MAP bundle is not registered"))?
            .lock()
            .map_err(|_| native_contract_failure("MAP state lock is poisoned"))?
            .call_leased_map(operation, lease, request, budget)
    }
    pub(crate) fn release_map(&self, lease: MapLease) -> Result<(), ExecutionFailure> {
        self.structured
            .as_ref()
            .ok_or_else(|| native_contract_failure("MAP bundle is not registered"))?
            .lock()
            .map_err(|_| native_contract_failure("MAP state lock is poisoned"))?
            .release_map_lease(lease)
    }
    pub(crate) fn retain_map_leases(
        &self,
        live: &std::collections::BTreeSet<MapLease>,
    ) -> Result<(), ExecutionFailure> {
        let live = live.union(&self.protected_map_leases).copied().collect();
        if let Some(state) = &self.structured {
            state
                .lock()
                .map_err(|_| native_contract_failure("MAP state lock is poisoned"))?
                .retain_map_leases(&live)
        } else if live.is_empty() {
            Ok(())
        } else {
            Err(native_contract_failure("live MAP leases have no bundle"))
        }
    }
    pub(crate) fn validate_map_snapshot(
        bytes: &[(SymbolKey, Vec<u8>)],
        live: &std::collections::BTreeSet<MapLease>,
    ) -> Result<(), String> {
        match bytes.iter().find(|(key, _)| *key == bundle_key()) {
            Some((_, state)) => StructuredState::decode(state)?
                .validate_map_lease_owners(live)
                .map_err(|error| error.to_string()),
            None if live.is_empty() => Ok(()),
            None => Err("snapshot MAP leases have no structured bundle".into()),
        }
    }
}

impl NativeServiceRegistry {
    pub(crate) fn map_lease_stamp(
        &self,
    ) -> Result<Option<crate::structured::MapLeaseStamp>, String> {
        self.structured
            .as_ref()
            .map(|state| {
                state
                    .lock()
                    .map_err(|_| "MAP state lock poisoned".to_owned())?
                    .map_lease_stamp()
                    .map_err(|failure| failure.to_string())
            })
            .transpose()
    }
    pub(crate) fn validate_map_lease_stamp(
        &self,
        expected: Option<crate::structured::MapLeaseStamp>,
    ) -> Result<(), String> {
        if self.map_lease_stamp()? != expected {
            return Err("prepared state belongs to a stale MAP lease timeline".into());
        }
        Ok(())
    }
    pub(crate) fn protect_map_roots(&mut self, roots: BTreeSet<MapLease>) -> Result<(), String> {
        self.validate_map_roots(&roots)?;
        self.protected_map_leases = roots;
        Ok(())
    }
    pub(crate) fn validate_map_roots(&self, roots: &BTreeSet<MapLease>) -> Result<(), String> {
        match &self.structured {
            Some(state) => state
                .lock()
                .map_err(|_| "MAP state lock poisoned".to_owned())?
                .validate_map_lease_owners(roots)
                .map_err(|failure| failure.to_string()),
            None if roots.is_empty() => Ok(()),
            None => Err("MAP owners have no structured bundle".into()),
        }
    }
    pub(crate) fn candidate_map_roots(&self, live: &BTreeSet<MapLease>) -> BTreeSet<MapLease> {
        live.union(&self.protected_map_leases).copied().collect()
    }
    pub(crate) fn protected_map_roots(&self) -> BTreeSet<MapLease> {
        self.protected_map_leases.clone()
    }
    pub(crate) fn prepare_map_lease_cleanup(
        &self,
        prepared: Option<&[u8]>,
        live: &BTreeSet<MapLease>,
    ) -> Result<Option<Vec<u8>>, String> {
        let Some(structured) = &self.structured else {
            return if live.is_empty() && self.protected_map_leases.is_empty() {
                Ok(None)
            } else {
                Err("live MAP leases have no structured bundle".into())
            };
        };
        let mut candidate = if let Some(bytes) = prepared {
            StructuredState::decode(bytes)?
        } else {
            structured
                .lock()
                .map_err(|_| "MAP state lock poisoned".to_owned())?
                .clone()
        };
        let retained = live.union(&self.protected_map_leases).copied().collect();
        candidate
            .retain_map_leases(&retained)
            .map_err(|failure| failure.to_string())?;
        candidate.encode().map(Some)
    }
    pub(crate) fn finish_map_candidate(
        &mut self,
        roots: &BTreeSet<MapLease>,
        inherited: BTreeSet<MapLease>,
    ) -> Result<(), String> {
        if self.protected_map_leases != *roots {
            return Err("candidate MAP parent roots changed".into());
        }
        self.validate_map_roots(roots)?;
        if !inherited.is_subset(roots) {
            return Err("candidate inherited MAP roots are not parent roots".into());
        }
        self.protected_map_leases = inherited;
        Ok(())
    }
}

impl NativeServiceRegistry {
    pub(crate) fn staged_map_provider(&self, key: SymbolKey) -> bool {
        self.staged_map_keys.contains(&key) && self.contains(key)
    }
}
