//! Named MAP bindings and leased object identity are deliberately separate.
use super::{
    BTreeMap, BTreeSet, Deserialize, ExecutionFailure, FaultCategory, OrderedMap, Serialize,
    StructuredState, SymbolKey, VmFaultCode, contract_failure,
};
use crate::{FiberId, FrameId, GenerationId};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub(crate) enum MapLeaseOrigin {
    Bytecode { begin: usize },
    RuntimeForm { instruction: usize, slot: u64 },
}
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub(crate) struct MapLeaseOwner {
    pub fiber: FiberId,
    pub frame: FrameId,
    pub generation: GenerationId,
    pub function: SymbolKey,
    pub origin: MapLeaseOrigin,
}
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub(crate) struct MapLease {
    id: u64,
    object: u64,
    pub owner: MapLeaseOwner,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum MapLocation {
    Named(String),
    Detached(OrderedMap),
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct MapLeaseBook {
    next: u64,
    pub(super) revision: u64,
    bindings: BTreeMap<String, u64>,
    objects: BTreeMap<u64, MapLocation>,
    active: BTreeMap<u64, MapLease>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MapLeaseStamp {
    next: u64,
    revision: u64,
}

impl Default for MapLeaseBook {
    fn default() -> Self {
        Self {
            next: 1,
            revision: 0,
            bindings: BTreeMap::new(),
            objects: BTreeMap::new(),
            active: BTreeMap::new(),
        }
    }
}
fn resource(message: &str) -> ExecutionFailure {
    ExecutionFailure::classified(FaultCategory::ResourceLimit, VmFaultCode::Native, message)
}
impl StructuredState {
    pub(crate) fn map_lease_stamp(&self) -> Result<MapLeaseStamp, ExecutionFailure> {
        if self.map_leases.revision == u64::MAX {
            return Err(resource("MAP lease revision exhausted"));
        }
        Ok(MapLeaseStamp {
            next: self.map_leases.next,
            revision: self.map_leases.revision,
        })
    }
    pub(crate) fn all_map_leases(&self) -> BTreeSet<MapLease> {
        self.map_leases.active.values().copied().collect()
    }
    pub(super) fn bump_map_revision(&mut self) -> Result<(), ExecutionFailure> {
        self.map_leases.revision = self
            .map_leases
            .revision
            .checked_add(1)
            .ok_or_else(|| resource("MAP revision exhausted"))?;
        Ok(())
    }
    pub(crate) fn capture_map(
        &mut self,
        name: &str,
        owner: MapLeaseOwner,
    ) -> Result<Option<MapLease>, ExecutionFailure> {
        if !self.maps.contains_key(name) {
            return Ok(None);
        }
        if self
            .map_leases
            .active
            .values()
            .any(|lease| lease.owner == owner)
        {
            return Err(contract_failure("MAP owner already has an active capture"));
        }
        let new_object = !self.map_leases.bindings.contains_key(name);
        let next = self
            .map_leases
            .next
            .checked_add(if new_object { 2 } else { 1 })
            .ok_or_else(|| resource("MAP lease identity exhausted"))?;
        let revision = self
            .map_leases
            .revision
            .checked_add(1)
            .ok_or_else(|| resource("MAP lease revision exhausted"))?;
        let object = if let Some(object) = self.map_leases.bindings.get(name) {
            *object
        } else {
            let object = self.map_leases.next;
            self.map_leases.bindings.insert(name.into(), object);
            self.map_leases
                .objects
                .insert(object, MapLocation::Named(name.into()));
            object
        };
        let lease = MapLease {
            id: next - 1,
            object,
            owner,
        };
        self.map_leases.active.insert(lease.id, lease);
        self.map_leases.next = next;
        self.map_leases.revision = revision;
        Ok(Some(lease))
    }
    pub(crate) fn release_map_lease(&mut self, lease: MapLease) -> Result<(), ExecutionFailure> {
        if self.map_leases.active.get(&lease.id) != Some(&lease) {
            return Err(contract_failure(
                "MAP lease is stale or belongs to another owner",
            ));
        }
        let revision = self
            .map_leases
            .revision
            .checked_add(1)
            .ok_or_else(|| resource("MAP lease revision exhausted"))?;
        self.map_leases.active.remove(&lease.id);
        if !self
            .map_leases
            .active
            .values()
            .any(|other| other.object == lease.object)
            && let Some(MapLocation::Named(name)) = self.map_leases.objects.remove(&lease.object)
        {
            self.map_leases.bindings.remove(&name);
        }
        self.map_leases.revision = revision;
        Ok(())
    }
    pub(crate) fn retain_map_leases(
        &mut self,
        live: &BTreeSet<MapLease>,
    ) -> Result<(), ExecutionFailure> {
        for lease in live {
            if self.map_leases.active.get(&lease.id) != Some(lease) {
                return Err(contract_failure(
                    "live MAP continuation references a missing lease",
                ));
            }
        }
        let retired = self
            .map_leases
            .active
            .values()
            .filter(|lease| !live.contains(lease))
            .copied()
            .collect::<Vec<_>>();
        if retired.is_empty() {
            return Ok(());
        }
        let revision = self
            .map_leases
            .revision
            .checked_add(1)
            .ok_or_else(|| resource("MAP lease revision exhausted"))?;
        let retired_objects = retired
            .iter()
            .map(|lease| lease.object)
            .collect::<BTreeSet<_>>();
        for lease in retired {
            self.map_leases.active.remove(&lease.id);
        }
        for object in retired_objects {
            if self
                .map_leases
                .active
                .values()
                .any(|lease| lease.object == object)
            {
                continue;
            }
            if let Some(MapLocation::Named(name)) = self.map_leases.objects.remove(&object) {
                self.map_leases.bindings.remove(&name);
            }
        }
        self.map_leases.revision = revision;
        Ok(())
    }
    pub(crate) fn validate_map_lease_owners(
        &self,
        live: &BTreeSet<MapLease>,
    ) -> Result<(), ExecutionFailure> {
        self.validate_map_leases()?;
        if self
            .map_leases
            .active
            .values()
            .copied()
            .collect::<BTreeSet<_>>()
            != *live
        {
            return Err(contract_failure(
                "snapshot MAP leases and executable owners differ",
            ));
        }
        Ok(())
    }
    pub(super) fn retire_map_binding(&mut self, name: &str) {
        let Some(map) = self.maps.remove(name) else {
            return;
        };
        if let Some(object) = self.map_leases.bindings.remove(name) {
            // The active lease retains the live old object after release/recreate.
            self.map_leases
                .objects
                .insert(object, MapLocation::Detached(map));
        }
    }
    pub(super) fn replace_map_binding(
        &mut self,
        name: &str,
        map: OrderedMap,
    ) -> Result<(), ExecutionFailure> {
        let revision = self
            .map_leases
            .revision
            .checked_add(1)
            .ok_or_else(|| resource("MAP revision exhausted"))?;
        self.retire_map_binding(name);
        self.maps.insert(name.into(), map);
        self.map_leases.revision = revision;
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn leased_map(&self, lease: MapLease) -> Result<&OrderedMap, ExecutionFailure> {
        if self.map_leases.active.get(&lease.id) != Some(&lease) {
            return Err(contract_failure("MAP operation uses a stale lease"));
        }
        match self.map_leases.objects.get(&lease.object) {
            Some(MapLocation::Named(name)) => self
                .maps
                .get(name)
                .ok_or_else(|| contract_failure("MAP lease lost its named object")),
            Some(MapLocation::Detached(map)) => Ok(map),
            None => Err(contract_failure("MAP lease lost its object")),
        }
    }
    pub(super) fn leased_map_mut(
        &mut self,
        lease: MapLease,
    ) -> Result<&mut OrderedMap, ExecutionFailure> {
        if self.map_leases.active.get(&lease.id) != Some(&lease) {
            return Err(contract_failure("MAP mutation uses a stale lease"));
        }
        match self.map_leases.objects.get_mut(&lease.object) {
            Some(MapLocation::Named(name)) => self
                .maps
                .get_mut(name)
                .ok_or_else(|| contract_failure("MAP lease lost its named object")),
            Some(MapLocation::Detached(map)) => Ok(map),
            None => Err(contract_failure("MAP lease lost its object")),
        }
    }
    pub(super) fn validate_map_leases(&self) -> Result<(), ExecutionFailure> {
        let book = &self.map_leases;
        if book.next == 0 || book.revision == u64::MAX {
            return Err(contract_failure(
                "MAP lease timeline is exhausted or starts at zero",
            ));
        }
        let mut owners = BTreeSet::new();
        for (id, lease) in &book.active {
            if !owners.insert(lease.owner) || book.objects.contains_key(id) {
                return Err(contract_failure("MAP lease owner/identity is duplicated"));
            }
            if *id == 0
                || *id >= book.next
                || *id != lease.id
                || lease.object == 0
                || lease.object >= book.next
                || !book.objects.contains_key(&lease.object)
            {
                return Err(contract_failure(
                    "MAP lease has an invalid identity or object",
                ));
            }
        }
        for (object, location) in &book.objects {
            if !book.active.values().any(|lease| lease.object == *object) {
                return Err(contract_failure(
                    "unowned MAP object retained in lease book",
                ));
            }
            if let MapLocation::Named(name) = location
                && (!self.maps.contains_key(name) || book.bindings.get(name) != Some(object))
            {
                return Err(contract_failure(
                    "MAP object binding differs from lease identity",
                ));
            }
        }
        for (name, object) in &book.bindings {
            if !matches!(book.objects.get(object), Some(MapLocation::Named(bound)) if bound == name)
            {
                return Err(contract_failure("MAP name references another lease object"));
            }
        }
        Ok(())
    }
}
