use std::collections::BTreeMap;

use erabasic_bytecode::{
    BytecodeArtifact, BytecodeConstant, BytecodeGlobal, BytecodeStorage, BytecodeType, SymbolKey,
};
use erabasic_data::{CharacterSelection, CharacterTemplate, RuntimeDefaults};
use serde::{Deserialize, Serialize};

use crate::{GenerationId, VmValue};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct VariableCell {
    pub value_type: BytecodeType,
    pub dimensions: Vec<u64>,
    pub values: Vec<VmValue>,
}

impl VariableCell {
    pub fn new(definition: &BytecodeGlobal) -> Self {
        let length = element_count(&definition.dimensions).unwrap_or(0);
        let mut values = vec![VmValue::default_for(definition.value_type); length];
        for (slot, value) in values.iter_mut().zip(&definition.initial_values) {
            *slot = match value {
                BytecodeConstant::Integer(value) => VmValue::Integer(*value),
                BytecodeConstant::String(value) => VmValue::String(value.clone()),
            };
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
            .cloned()
            .ok_or_else(|| "variable offset is outside its storage".into())
    }

    pub fn write(&mut self, indices: &[u64], value: VmValue) -> Result<(), String> {
        if value.value_type() != self.value_type {
            return Err(format!(
                "variable expects {:?}, found {:?}",
                self.value_type,
                value.value_type()
            ));
        }
        let offset = flatten(&self.dimensions, indices)?;
        let slot = self
            .values
            .get_mut(offset)
            .ok_or_else(|| "variable offset is outside its storage".to_owned())?;
        *slot = value;
        Ok(())
    }

    pub fn migrate(&self, definition: &BytecodeGlobal) -> Self {
        let mut target = Self::new(definition);
        let target_len = target.values.len();
        for target_offset in 0..target_len {
            let coordinates = unflatten(&target.dimensions, target_offset);
            if coordinates
                .iter()
                .zip(&self.dimensions)
                .all(|(index, length)| index < length)
                && coordinates.len() == self.dimensions.len()
                && let Ok(source_offset) = flatten(&self.dimensions, &coordinates)
                && let Some(value) = self.values.get(source_offset)
            {
                target.values[target_offset] = value.clone();
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
            if let Some(slot) = self.values.get_mut(target_offset) {
                *slot = value.clone();
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
    pub fn new_game(artifact: &BytecodeArtifact) -> Self {
        let mut result = Self::empty(artifact);
        result.apply_runtime_defaults(artifact, &artifact.project_data.new_game_seed().defaults);
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
                BytecodeStorage::FunctionStatic => {
                    result
                        .statics
                        .insert(definition.key, VariableCell::new(definition));
                }
                BytecodeStorage::FunctionLocal | BytecodeStorage::Character => {}
            }
        }
        result
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

    fn set_named_integer(&mut self, artifact: &BytecodeArtifact, name: &str, value: i64) {
        if let Some(definition) = find_definition(artifact, name)
            && let Some(cell) = self.shared.get_mut(&definition.key)
            && let Some(slot) = cell.values.first_mut()
        {
            *slot = VmValue::Integer(value);
        }
    }

    fn set_named_string(&mut self, artifact: &BytecodeArtifact, name: &str, value: &str) {
        if let Some(definition) = find_definition(artifact, name)
            && let Some(cell) = self.shared.get_mut(&definition.key)
            && let Some(slot) = cell.values.first_mut()
        {
            *slot = VmValue::String(value.into());
        }
    }

    fn set_named_values(&mut self, artifact: &BytecodeArtifact, name: &str, values: &[i64]) {
        if let Some(definition) = find_definition(artifact, name)
            && let Some(cell) = self.shared.get_mut(&definition.key)
        {
            for (slot, value) in cell.values.iter_mut().zip(values) {
                *slot = VmValue::Integer(*value);
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
            for (slot, value) in cell.values.iter_mut().zip(values) {
                *slot = VmValue::String(value.clone().unwrap_or_default());
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
        match cell.and_then(|cell| cell.values.first()) {
            Some(VmValue::Integer(value)) => usize::try_from(*value).unwrap_or(0),
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
            BytecodeStorage::FunctionStatic => legacy
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
                    BytecodeStorage::FunctionStatic => memory.statics.contains_key(&definition.key),
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
                BytecodeStorage::FunctionStatic => memory.statics.get_mut(&definition.key),
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
            BytecodeStorage::FunctionStatic => self.statics.get_mut(&definition.key),
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
                BytecodeStorage::FunctionStatic => {
                    if let Some(cell) = self.statics.get(&definition.key) {
                        legacy.statics.insert(definition.key, cell.clone());
                    }
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
                BytecodeStorage::FunctionStatic => {
                    let cell = self.statics.get(&definition.key).map_or_else(
                        || VariableCell::new(definition),
                        |cell| cell.migrate(definition),
                    );
                    self.statics.insert(definition.key, cell);
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
            && let Some(slot) = cells
                .get_mut(&definition.key)
                .and_then(|cell| cell.values.first_mut())
        {
            *slot = value;
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
                if let Some(slot) = cell.values.get_mut(*index) {
                    *slot = VmValue::Integer(*value);
                }
            }
        }
    }
    if let Some(definition) = find_definition(artifact, "CSTR")
        && let Some(cell) = cells.get_mut(&definition.key)
    {
        for (index, value) in &template.cstr {
            if let Some(slot) = cell.values.get_mut(*index) {
                *slot = VmValue::String(value.clone());
            }
        }
    }
}
