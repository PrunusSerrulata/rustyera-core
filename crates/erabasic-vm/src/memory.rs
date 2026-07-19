use std::collections::BTreeMap;

use erabasic_bytecode::{
    BytecodeArtifact, BytecodeConstant, BytecodeGlobal, BytecodeStorage, BytecodeType, SymbolKey,
};
use erabasic_data::{CharacterSelection, CharacterTemplate, RuntimeDefaults};
use serde::{Deserialize, Serialize};

use crate::{GenerationId, PlaceDescriptor, VmValue};

/// Dense variable storage is specialized by `EraBasic` value type.
///
/// Most game memory consists of large integer arrays, especially character
/// variables. Keeping every element in the public `VmValue` enum would retain
/// the enum's largest payload and waste two thirds of each integer allocation.
/// The VM converts at its boundary instead, preserving the public value model
/// and snapshot semantics while storing dense arrays in their native layout.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum VariableValues {
    Integers(Vec<i64>),
    Strings(Vec<String>),
    IntegerPlaces(Vec<PlaceDescriptor>),
    StringPlaces(Vec<PlaceDescriptor>),
}

impl VariableValues {
    fn with_default(value_type: BytecodeType, length: usize) -> Self {
        match value_type {
            BytecodeType::Integer => Self::Integers(vec![0; length]),
            BytecodeType::String => Self::Strings(vec![String::new(); length]),
            BytecodeType::IntegerPlace => {
                Self::IntegerPlaces(vec![PlaceDescriptor::default(); length])
            }
            BytecodeType::StringPlace => {
                Self::StringPlaces(vec![PlaceDescriptor::default(); length])
            }
        }
    }

    const fn value_type(&self) -> BytecodeType {
        match self {
            Self::Integers(_) => BytecodeType::Integer,
            Self::Strings(_) => BytecodeType::String,
            Self::IntegerPlaces(_) => BytecodeType::IntegerPlace,
            Self::StringPlaces(_) => BytecodeType::StringPlace,
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Integers(values) => values.len(),
            Self::Strings(values) => values.len(),
            Self::IntegerPlaces(values) | Self::StringPlaces(values) => values.len(),
        }
    }

    fn get(&self, index: usize) -> Option<VmValue> {
        match self {
            Self::Integers(values) => values.get(index).copied().map(VmValue::Integer),
            Self::Strings(values) => values.get(index).cloned().map(VmValue::String),
            Self::IntegerPlaces(values) => values
                .get(index)
                .cloned()
                .map(Box::new)
                .map(VmValue::IntegerPlace),
            Self::StringPlaces(values) => values
                .get(index)
                .cloned()
                .map(Box::new)
                .map(VmValue::StringPlace),
        }
    }

    fn set(&mut self, index: usize, value: VmValue) -> Result<(), String> {
        match (self, value) {
            (Self::Integers(values), VmValue::Integer(value)) => set_slot(values, index, value),
            (Self::Strings(values), VmValue::String(value)) => set_slot(values, index, value),
            (Self::IntegerPlaces(values), VmValue::IntegerPlace(value))
            | (Self::StringPlaces(values), VmValue::StringPlace(value)) => {
                set_slot(values, index, *value)
            }
            (values, value) => Err(format!(
                "variable expects {:?}, found {:?}",
                values.value_type(),
                value.value_type()
            )),
        }
    }

    fn fill(&mut self, value: VmValue) -> Result<(), String> {
        match (self, value) {
            (Self::Integers(values), VmValue::Integer(value)) => values.fill(value),
            (Self::Strings(values), VmValue::String(value)) => values.fill(value),
            (Self::IntegerPlaces(values), VmValue::IntegerPlace(value))
            | (Self::StringPlaces(values), VmValue::StringPlace(value)) => values.fill(*value),
            (values, value) => {
                return Err(format!(
                    "variable expects {:?}, found {:?}",
                    values.value_type(),
                    value.value_type()
                ));
            }
        }
        Ok(())
    }

    fn to_vm_values(&self) -> Vec<VmValue> {
        match self {
            Self::Integers(values) => values.iter().copied().map(VmValue::Integer).collect(),
            Self::Strings(values) => values.iter().cloned().map(VmValue::String).collect(),
            Self::IntegerPlaces(values) => values
                .iter()
                .cloned()
                .map(Box::new)
                .map(VmValue::IntegerPlace)
                .collect(),
            Self::StringPlaces(values) => values
                .iter()
                .cloned()
                .map(Box::new)
                .map(VmValue::StringPlace)
                .collect(),
        }
    }
}

fn set_slot<T>(values: &mut [T], index: usize, value: T) -> Result<(), String> {
    let slot = values
        .get_mut(index)
        .ok_or_else(|| "variable offset is outside its storage".to_owned())?;
    *slot = value;
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct VariableCell {
    pub value_type: BytecodeType,
    pub dimensions: Vec<u64>,
    values: VariableValues,
}

impl VariableCell {
    pub fn new(definition: &BytecodeGlobal) -> Self {
        let length = element_count(&definition.dimensions).unwrap_or(0);
        let mut values = VariableValues::with_default(definition.value_type, length);
        for (index, value) in definition.initial_values.iter().enumerate() {
            let value = match value {
                BytecodeConstant::Integer(value) => VmValue::Integer(*value),
                BytecodeConstant::String(value) => VmValue::String(value.clone()),
            };
            values
                .set(index, value)
                .expect("validated global initial value matches its declaration");
        }
        Self {
            value_type: definition.value_type,
            dimensions: definition.dimensions.clone(),
            values,
        }
    }

    pub fn read(&self, indices: &[u64]) -> Result<VmValue, String> {
        let offset = flatten(&self.dimensions, indices)?;
        self.values
            .get(offset)
            .ok_or_else(|| "variable offset is outside its storage".into())
    }

    pub fn write(&mut self, indices: &[u64], value: VmValue) -> Result<(), String> {
        let offset = flatten(&self.dimensions, indices)?;
        self.values.set(offset, value)
    }

    pub(crate) fn len(&self) -> usize {
        self.values.len()
    }

    pub(crate) fn first(&self) -> Option<VmValue> {
        self.values.get(0)
    }

    pub(crate) fn get(&self, index: usize) -> Option<VmValue> {
        self.values.get(index)
    }

    pub(crate) fn set(&mut self, index: usize, value: VmValue) -> Result<(), String> {
        self.values.set(index, value)
    }

    pub(crate) fn fill(&mut self, value: VmValue) -> Result<(), String> {
        self.values.fill(value)
    }

    pub(crate) fn to_values(&self) -> Vec<VmValue> {
        self.values.to_vm_values()
    }

    pub(crate) fn integers(&self) -> Option<&[i64]> {
        match &self.values {
            VariableValues::Integers(values) => Some(values),
            _ => None,
        }
    }

    pub(crate) fn integers_mut(&mut self) -> Option<&mut [i64]> {
        match &mut self.values {
            VariableValues::Integers(values) => Some(values),
            _ => None,
        }
    }

    pub(crate) fn replace_values(&mut self, values: Vec<VmValue>) -> Result<(), String> {
        if values.len() != self.len()
            || values
                .iter()
                .any(|value| value.value_type() != self.value_type)
        {
            return Err("array replacement differs from its storage shape or type".into());
        }
        let mut replacement = VariableValues::with_default(self.value_type, values.len());
        for (index, value) in values.into_iter().enumerate() {
            replacement.set(index, value)?;
        }
        self.values = replacement;
        Ok(())
    }

    pub(crate) fn replace_shape(
        &mut self,
        value_type: BytecodeType,
        dimensions: Vec<u64>,
        values: Vec<VmValue>,
    ) -> Result<(), String> {
        self.value_type = value_type;
        self.dimensions = dimensions;
        self.values = VariableValues::with_default(value_type, values.len());
        self.replace_values(values)
    }

    pub(crate) fn storage_is_valid(&self) -> bool {
        self.values.value_type() == self.value_type
    }

    pub fn migrate(&self, definition: &BytecodeGlobal) -> Self {
        let mut target = Self::new(definition);
        let target_len = target.len();
        for target_offset in 0..target_len {
            let coordinates = unflatten(&target.dimensions, target_offset);
            if coordinates
                .iter()
                .zip(&self.dimensions)
                .all(|(index, length)| index < length)
                && coordinates.len() == self.dimensions.len()
                && let Ok(source_offset) = flatten(&self.dimensions, &coordinates)
                && let Some(value) = self.get(source_offset)
            {
                target
                    .set(target_offset, value)
                    .expect("migration only copies identical variable types");
            }
        }
        target
    }

    pub fn overlay(&mut self, dimensions: &[u64], values: &[VmValue]) -> Result<(), String> {
        for (source_offset, value) in values.iter().enumerate() {
            let coordinates = unflatten(dimensions, source_offset);
            if coordinates.len() != self.dimensions.len()
                || !coordinates
                    .iter()
                    .zip(&self.dimensions)
                    .all(|(index, length)| index < length)
            {
                continue;
            }
            let target_offset = flatten(&self.dimensions, &coordinates)?;
            if value.value_type() != self.value_type {
                return Err("saved variable value type does not match its schema".into());
            }
            if target_offset < self.len() {
                self.set(target_offset, value.clone())?;
            }
        }
        Ok(())
    }
}

fn element_count(dimensions: &[u64]) -> Option<usize> {
    dimensions
        .iter()
        .copied()
        .try_fold(1u64, u64::checked_mul)
        .and_then(|length| usize::try_from(length).ok())
}

fn flatten(dimensions: &[u64], indices: &[u64]) -> Result<usize, String> {
    if indices.len() > dimensions.len() {
        return Err("too many variable indices".into());
    }
    let mut offset = 0u64;
    for (dimension, length) in dimensions.iter().enumerate() {
        let index = indices.get(dimension).copied().unwrap_or(0);
        if index >= *length {
            return Err(format!(
                "index {index} is outside dimension {dimension} of length {length}"
            ));
        }
        offset = offset
            .checked_mul(*length)
            .and_then(|value| value.checked_add(index))
            .ok_or_else(|| "variable offset overflow".to_owned())?;
    }
    usize::try_from(offset).map_err(|_| "variable offset exceeds this platform".into())
}

fn unflatten(dimensions: &[u64], mut offset: usize) -> Vec<u64> {
    let mut result = vec![0; dimensions.len()];
    for dimension in (0..dimensions.len()).rev() {
        let length = usize::try_from(dimensions[dimension]).unwrap_or(usize::MAX);
        if length != 0 {
            result[dimension] = (offset % length) as u64;
            offset /= length;
        }
    }
    result
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct LegacyMemory {
    pub shared: BTreeMap<SymbolKey, VariableCell>,
    pub statics: BTreeMap<SymbolKey, VariableCell>,
    pub characters: Vec<BTreeMap<SymbolKey, VariableCell>>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Memory {
    pub shared: BTreeMap<SymbolKey, VariableCell>,
    pub statics: BTreeMap<SymbolKey, VariableCell>,
    pub characters: Vec<BTreeMap<SymbolKey, VariableCell>>,
    pub legacy: BTreeMap<GenerationId, LegacyMemory>,
}

impl Memory {
    pub fn title(artifact: &BytecodeArtifact) -> Self {
        let mut result = Self::empty(artifact);
        // Emuera initializes ordinary variable defaults before SYSTEM_TITLE, but
        // ResetData and the initial CSV characters are deferred until the player
        // actually selects "new game" from the built-in title flow.
        result.apply_runtime_defaults(artifact, &artifact.project_data.new_game_seed().defaults);
        result
    }

    pub fn new_game(artifact: &BytecodeArtifact) -> Self {
        let mut result = Self::title(artifact);
        for selection in &artifact.project_data.new_game_seed().initial_characters {
            match selection {
                CharacterSelection::CsvNumber(number) => {
                    let template = artifact
                        .project_data
                        .static_data
                        .characters
                        .iter()
                        .find(|template| template.csv_no == *number);
                    result.push_character(artifact, template);
                }
            }
        }
        // Calculated variables are materialized as cells so bytecode can load
        // them normally. Initialization must therefore refresh CHARANUM just as
        // the native character mutation service does after ADDCHARA.
        result.set_named_integer(
            artifact,
            "CHARANUM",
            i64::try_from(result.characters.len()).unwrap_or(i64::MAX),
        );
        result
    }

    pub fn empty(artifact: &BytecodeArtifact) -> Self {
        let mut result = Self::default();
        for definition in &artifact.globals {
            match definition.storage {
                BytecodeStorage::Project
                | BytecodeStorage::Constant
                | BytecodeStorage::Calculated => {
                    result
                        .shared
                        .insert(definition.key, VariableCell::new(definition));
                }
                BytecodeStorage::FunctionStatic
                | BytecodeStorage::FunctionPersistent
                | BytecodeStorage::FunctionLocal
                | BytecodeStorage::Character => {}
            }
        }
        result
    }

    pub(crate) fn ensure_function_statics<'a>(
        &mut self,
        generation: GenerationId,
        definitions: impl IntoIterator<Item = &'a BytecodeGlobal>,
    ) {
        for definition in definitions {
            if self
                .legacy
                .get(&generation)
                .is_some_and(|memory| memory.statics.contains_key(&definition.key))
            {
                continue;
            }
            self.statics
                .entry(definition.key)
                .or_insert_with(|| VariableCell::new(definition));
        }
    }

    pub(crate) fn initialize_function_statics(&mut self, artifact: &BytecodeArtifact) {
        for definition in &artifact.globals {
            if matches!(
                definition.storage,
                BytecodeStorage::FunctionStatic | BytecodeStorage::FunctionPersistent
            ) {
                self.statics
                    .insert(definition.key, VariableCell::new(definition));
            }
        }
    }

    pub fn push_character(
        &mut self,
        artifact: &BytecodeArtifact,
        template: Option<&CharacterTemplate>,
    ) {
        let mut character: BTreeMap<_, _> = artifact
            .globals
            .iter()
            .filter(|definition| definition.storage == BytecodeStorage::Character)
            .map(|definition| (definition.key, VariableCell::new(definition)))
            .collect();
        if let Some(template) = template {
            initialize_character(artifact, &mut character, template);
        }
        self.characters.push(character);
    }

    fn apply_runtime_defaults(&mut self, artifact: &BytecodeArtifact, defaults: &RuntimeDefaults) {
        self.set_named_values(artifact, "ITEMPRICE", &defaults.item_prices);
        self.set_named_optional_strings(artifact, "STR", &defaults.str_values);
        self.set_named_values(artifact, "PALAMLV", &defaults.palam_levels);
        self.set_named_values(artifact, "EXPLV", &defaults.exp_levels);
        for (name, value) in [
            ("ASSI", defaults.assi_0),
            ("TARGET", defaults.target_0),
            ("PBAND", defaults.pband_0),
            ("EJAC", defaults.ejac_0),
            ("NOITEM", defaults.no_item_0),
            ("RELATION", defaults.relation_default),
            ("LASTLOAD_VERSION", defaults.last_load_version),
            ("LASTLOAD_NO", defaults.last_load_no),
        ] {
            self.set_named_integer(artifact, name, value);
        }
        self.set_named_string(artifact, "LASTLOAD_TEXT", &defaults.last_load_text);
    }

    pub(crate) fn set_last_load(
        &mut self,
        artifact: &BytecodeArtifact,
        version: i64,
        slot: i64,
        text: &str,
    ) {
        self.set_named_integer(artifact, "LASTLOAD_VERSION", version);
        self.set_named_integer(artifact, "LASTLOAD_NO", slot);
        self.set_named_string(artifact, "LASTLOAD_TEXT", text);
    }

    fn set_named_integer(&mut self, artifact: &BytecodeArtifact, name: &str, value: i64) {
        if let Some(definition) = find_definition(artifact, name)
            && let Some(cell) = self.shared.get_mut(&definition.key)
        {
            let _ = cell.set(0, VmValue::Integer(value));
        }
    }

    fn set_named_string(&mut self, artifact: &BytecodeArtifact, name: &str, value: &str) {
        if let Some(definition) = find_definition(artifact, name)
            && let Some(cell) = self.shared.get_mut(&definition.key)
        {
            let _ = cell.set(0, VmValue::String(value.into()));
        }
    }

    fn set_named_values(&mut self, artifact: &BytecodeArtifact, name: &str, values: &[i64]) {
        if let Some(definition) = find_definition(artifact, name)
            && let Some(cell) = self.shared.get_mut(&definition.key)
        {
            for (index, value) in values.iter().copied().take(cell.len()).enumerate() {
                let _ = cell.set(index, VmValue::Integer(value));
            }
        }
    }

    fn set_named_optional_strings(
        &mut self,
        artifact: &BytecodeArtifact,
        name: &str,
        values: &[Option<String>],
    ) {
        if let Some(definition) = find_definition(artifact, name)
            && let Some(cell) = self.shared.get_mut(&definition.key)
        {
            for (index, value) in values.iter().take(cell.len()).enumerate() {
                let _ = cell.set(index, VmValue::String(value.clone().unwrap_or_default()));
            }
        }
    }

    pub fn target_character(&self, artifact: &BytecodeArtifact, generation: GenerationId) -> usize {
        let Some(definition) = find_definition(artifact, "TARGET") else {
            return 0;
        };
        let cell = self
            .legacy
            .get(&generation)
            .and_then(|memory| memory.shared.get(&definition.key))
            .or_else(|| self.shared.get(&definition.key));
        match cell.and_then(VariableCell::first) {
            Some(VmValue::Integer(value)) => usize::try_from(value).unwrap_or(0),
            _ => 0,
        }
    }

    pub fn cell(
        &self,
        generation: GenerationId,
        definition: &BytecodeGlobal,
        character: usize,
    ) -> Option<&VariableCell> {
        let legacy = self.legacy.get(&generation);
        match definition.storage {
            BytecodeStorage::Project | BytecodeStorage::Constant | BytecodeStorage::Calculated => {
                legacy
                    .and_then(|memory| memory.shared.get(&definition.key))
                    .or_else(|| self.shared.get(&definition.key))
            }
            BytecodeStorage::FunctionStatic | BytecodeStorage::FunctionPersistent => legacy
                .and_then(|memory| memory.statics.get(&definition.key))
                .or_else(|| self.statics.get(&definition.key)),
            BytecodeStorage::Character => legacy
                .and_then(|memory| memory.characters.get(character))
                .and_then(|values| values.get(&definition.key))
                .or_else(|| {
                    self.characters
                        .get(character)
                        .and_then(|values| values.get(&definition.key))
                }),
            BytecodeStorage::FunctionLocal => None,
        }
    }

    pub fn cell_mut(
        &mut self,
        generation: GenerationId,
        definition: &BytecodeGlobal,
        character: usize,
    ) -> Option<&mut VariableCell> {
        let use_legacy =
            self.legacy
                .get(&generation)
                .is_some_and(|memory| match definition.storage {
                    BytecodeStorage::Project
                    | BytecodeStorage::Constant
                    | BytecodeStorage::Calculated => memory.shared.contains_key(&definition.key),
                    BytecodeStorage::FunctionStatic | BytecodeStorage::FunctionPersistent => {
                        memory.statics.contains_key(&definition.key)
                    }
                    BytecodeStorage::Character => memory
                        .characters
                        .get(character)
                        .is_some_and(|values| values.contains_key(&definition.key)),
                    BytecodeStorage::FunctionLocal => false,
                });
        if use_legacy {
            let memory = self.legacy.get_mut(&generation)?;
            return match definition.storage {
                BytecodeStorage::Project
                | BytecodeStorage::Constant
                | BytecodeStorage::Calculated => memory.shared.get_mut(&definition.key),
                BytecodeStorage::FunctionStatic | BytecodeStorage::FunctionPersistent => {
                    memory.statics.get_mut(&definition.key)
                }
                BytecodeStorage::Character => memory
                    .characters
                    .get_mut(character)
                    .and_then(|values| values.get_mut(&definition.key)),
                BytecodeStorage::FunctionLocal => None,
            };
        }
        match definition.storage {
            BytecodeStorage::Project | BytecodeStorage::Constant | BytecodeStorage::Calculated => {
                self.shared.get_mut(&definition.key)
            }
            BytecodeStorage::FunctionStatic | BytecodeStorage::FunctionPersistent => {
                self.statics.get_mut(&definition.key)
            }
            BytecodeStorage::Character => self
                .characters
                .get_mut(character)
                .and_then(|values| values.get_mut(&definition.key)),
            BytecodeStorage::FunctionLocal => None,
        }
    }

    pub fn migrate(
        &mut self,
        old_generation: GenerationId,
        old: &BytecodeArtifact,
        target: &BytecodeArtifact,
    ) {
        let target_definitions: BTreeMap<_, _> = target
            .globals
            .iter()
            .map(|definition| (definition.key, definition))
            .collect();
        let mut legacy = LegacyMemory {
            characters: vec![BTreeMap::new(); self.characters.len()],
            ..LegacyMemory::default()
        };
        for definition in &old.globals {
            if definition.storage == BytecodeStorage::FunctionLocal {
                continue;
            }
            let changed = target_definitions
                .get(&definition.key)
                .is_none_or(|target| target.dimensions != definition.dimensions);
            if !changed {
                continue;
            }
            match definition.storage {
                BytecodeStorage::Project
                | BytecodeStorage::Constant
                | BytecodeStorage::Calculated => {
                    if let Some(cell) = self.shared.get(&definition.key) {
                        legacy.shared.insert(definition.key, cell.clone());
                    }
                }
                BytecodeStorage::FunctionStatic | BytecodeStorage::FunctionPersistent => {
                    legacy.statics.insert(
                        definition.key,
                        self.statics
                            .get(&definition.key)
                            .map_or_else(|| VariableCell::new(definition), Clone::clone),
                    );
                }
                BytecodeStorage::Character => {
                    for (index, character) in self.characters.iter().enumerate() {
                        if let Some(cell) = character.get(&definition.key) {
                            legacy.characters[index].insert(definition.key, cell.clone());
                        }
                    }
                }
                BytecodeStorage::FunctionLocal => {}
            }
        }
        for definition in &target.globals {
            let old_definition = old.globals.iter().find(|old| old.key == definition.key);
            let changed = old_definition.is_none_or(|old| old.dimensions != definition.dimensions);
            if !changed || definition.storage == BytecodeStorage::FunctionLocal {
                continue;
            }
            match definition.storage {
                BytecodeStorage::Project
                | BytecodeStorage::Constant
                | BytecodeStorage::Calculated => {
                    let cell = self.shared.get(&definition.key).map_or_else(
                        || VariableCell::new(definition),
                        |cell| cell.migrate(definition),
                    );
                    self.shared.insert(definition.key, cell);
                }
                BytecodeStorage::FunctionStatic | BytecodeStorage::FunctionPersistent => {
                    if let Some(cell) = self.statics.get(&definition.key) {
                        self.statics
                            .insert(definition.key, cell.migrate(definition));
                    }
                }
                BytecodeStorage::Character => {
                    for character in &mut self.characters {
                        let cell = character.get(&definition.key).map_or_else(
                            || VariableCell::new(definition),
                            |cell| cell.migrate(definition),
                        );
                        character.insert(definition.key, cell);
                    }
                }
                BytecodeStorage::FunctionLocal => {}
            }
        }
        if !legacy.shared.is_empty()
            || !legacy.statics.is_empty()
            || legacy.characters.iter().any(|values| !values.is_empty())
        {
            self.legacy.insert(old_generation, legacy);
        }
    }

    pub fn reclaim_generation(&mut self, generation: GenerationId) {
        self.legacy.remove(&generation);
    }
}

fn find_definition<'a>(artifact: &'a BytecodeArtifact, name: &str) -> Option<&'a BytecodeGlobal> {
    artifact
        .globals
        .iter()
        .find(|definition| definition.name.eq_ignore_ascii_case(name))
}

fn initialize_character(
    artifact: &BytecodeArtifact,
    cells: &mut BTreeMap<SymbolKey, VariableCell>,
    template: &CharacterTemplate,
) {
    for (name, value) in [
        ("NO", VmValue::Integer(template.no)),
        ("NAME", VmValue::String(template.name.clone())),
        ("CALLNAME", VmValue::String(template.call_name.clone())),
        ("NICKNAME", VmValue::String(template.nick_name.clone())),
        ("MASTERNAME", VmValue::String(template.master_name.clone())),
    ] {
        if let Some(definition) = find_definition(artifact, name)
            && let Some(cell) = cells.get_mut(&definition.key)
        {
            let _ = cell.set(0, value);
        }
    }
    for (name, values) in [
        ("MAXBASE", &template.max_base),
        ("BASE", &template.max_base),
        ("MARK", &template.mark),
        ("EXP", &template.exp),
        ("ABL", &template.abl),
        ("TALENT", &template.talent),
        ("RELATION", &template.relation),
        ("CFLAG", &template.cflag),
        ("EQUIP", &template.equip),
        ("JUEL", &template.juel),
    ] {
        if let Some(definition) = find_definition(artifact, name)
            && let Some(cell) = cells.get_mut(&definition.key)
        {
            for (index, value) in values {
                if *index < cell.len() {
                    let _ = cell.set(*index, VmValue::Integer(*value));
                }
            }
        }
    }
    if let Some(definition) = find_definition(artifact, "CSTR")
        && let Some(cell) = cells.get_mut(&definition.key)
    {
        for (index, value) in &template.cstr {
            if *index < cell.len() {
                let _ = cell.set(*index, VmValue::String(value.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use erabasic_bytecode::{BytecodePersistence, BytecodeStorage};

    use super::*;

    fn global(value_type: BytecodeType, dimensions: Vec<u64>) -> BytecodeGlobal {
        BytecodeGlobal {
            key: SymbolKey::derive("memory.test", format!("{value_type:?}").as_bytes()),
            name: "VALUE".into(),
            value_type,
            dimensions,
            mutable: true,
            storage: BytecodeStorage::Project,
            persistence: BytecodePersistence::GameSave,
            initial_values: Vec::new(),
            owner: None,
        }
    }

    #[test]
    fn dense_integer_cell_preserves_public_vm_value_behavior() {
        let mut cell = VariableCell::new(&global(BytecodeType::Integer, vec![4]));
        cell.write(&[2], VmValue::Integer(41)).unwrap();
        cell.set(3, VmValue::Integer(42)).unwrap();

        assert_eq!(cell.read(&[2]).unwrap(), VmValue::Integer(41));
        assert_eq!(
            cell.to_values(),
            vec![
                VmValue::Integer(0),
                VmValue::Integer(0),
                VmValue::Integer(41),
                VmValue::Integer(42),
            ]
        );
        assert!(cell.set(0, VmValue::String("wrong".into())).is_err());
        assert_eq!(cell.read(&[0]).unwrap(), VmValue::Integer(0));
    }

    #[test]
    fn dense_place_cell_boxes_only_values_crossing_the_vm_boundary() {
        let mut cell = VariableCell::new(&global(BytecodeType::IntegerPlace, vec![1]));
        let place = PlaceDescriptor {
            variable: SymbolKey::derive("memory.test", b"target"),
            indices: vec![2, 3],
            ..PlaceDescriptor::default()
        };
        cell.set(0, VmValue::IntegerPlace(Box::new(place.clone())))
            .unwrap();

        assert_eq!(cell.first(), Some(VmValue::IntegerPlace(Box::new(place))));
        assert!(cell.storage_is_valid());
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn public_vm_value_stays_small_enough_for_transient_stacks() {
        assert_eq!(std::mem::size_of::<VmValue>(), 24);
        assert_eq!(std::mem::size_of::<i64>(), 8);
    }
}
