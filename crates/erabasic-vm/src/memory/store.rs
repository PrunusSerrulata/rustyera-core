#[allow(clippy::wildcard_imports)]
use super::*;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct LegacyMemory {
    pub shared: VariableMap,
    pub statics: VariableMap,
    pub characters: Vec<VariableMap>,
}

#[derive(Clone, Debug, Default)]
struct StaticInitializationCache(HashSet<(GenerationId, SymbolKey)>);

// This cache only avoids repeated idempotent checks. It is not VM state and
// must not affect snapshot equality or the deterministic serialized payload.
impl PartialEq for StaticInitializationCache {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for StaticInitializationCache {}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Memory {
    pub shared: VariableMap,
    pub statics: VariableMap,
    pub characters: Vec<VariableMap>,
    pub legacy: BTreeMap<GenerationId, LegacyMemory>,
    pub(crate) array_leases: crate::state::array_leases::ArrayLeases,
    #[serde(skip)]
    initialized_static_functions: StaticInitializationCache,
}

impl Memory {
    pub(crate) fn materialize_snapshot(&mut self) -> Result<(), String> {
        self.array_leases.materialize_snapshot()?;
        for cell in self
            .shared
            .values_mut()
            .chain(self.statics.values_mut())
            .chain(
                self.characters
                    .iter_mut()
                    .flat_map(|character| character.values_mut()),
            )
        {
            cell.materialize_snapshot()?;
        }
        Ok(())
    }

    pub fn title(artifact: &BytecodeArtifact) -> Self {
        Self::title_with_progress(artifact, &mut |_, _| {})
    }

    pub(crate) fn title_with_progress(
        artifact: &BytecodeArtifact,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Self {
        let global_count = u64::try_from(artifact.globals.len()).unwrap_or(u64::MAX - 1);
        let total = global_count.saturating_add(1).max(1);
        let mut result = Self::empty_with_progress(artifact, total, progress);
        // Emuera initializes ordinary variable defaults before SYSTEM_TITLE, but
        // ResetData and the initial CSV characters are deferred until the player
        // actually selects "new game" from the built-in title flow.
        result.apply_runtime_defaults(artifact, &artifact.project_data.new_game_seed().defaults);
        progress(total, total);
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
        result.refresh_character_count(artifact);
        result
    }

    pub(crate) fn refresh_character_count(&mut self, artifact: &BytecodeArtifact) {
        self.set_named_integer(
            artifact,
            "CHARANUM",
            i64::try_from(self.characters.len()).unwrap_or(i64::MAX),
        );
    }

    fn empty_with_progress(
        artifact: &BytecodeArtifact,
        total: u64,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Self {
        let mut result = Self::default();
        let mut next_checkpoint = 1;
        for (index, definition) in artifact.globals.iter().enumerate() {
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
            let completed = u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1);
            let checkpoint = completed.saturating_mul(100) / total.max(1);
            if checkpoint >= next_checkpoint || completed == total {
                progress(completed, total);
                next_checkpoint = checkpoint.saturating_add(1);
            }
        }
        result
    }

    pub(crate) fn ensure_function_statics<'a>(
        &mut self,
        generation: GenerationId,
        function: SymbolKey,
        definitions: impl IntoIterator<Item = &'a BytecodeGlobal>,
    ) {
        if !self
            .initialized_static_functions
            .0
            .insert((generation, function))
        {
            return;
        }
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

    pub fn push_character(
        &mut self,
        artifact: &BytecodeArtifact,
        template: Option<&CharacterTemplate>,
    ) {
        let mut character: VariableMap = artifact
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
        let static_data = &artifact.project_data.static_data;
        let game_base = &static_data.game_base;
        for (name, value) in [
            ("ASSI", defaults.assi_0),
            ("TARGET", defaults.target_0),
            ("PBAND", defaults.pband_0),
            ("EJAC", defaults.ejac_0),
            ("NOITEM", defaults.no_item_0),
            ("RELATION", defaults.relation_default),
            ("LASTLOAD_VERSION", defaults.last_load_version),
            ("LASTLOAD_NO", defaults.last_load_no),
            ("GAMEBASE_GAMECODE", game_base.unique_code),
            ("GAMEBASE_VERSION", game_base.version),
            ("GAMEBASE_ALLOWVERSION", game_base.compatible_min_version),
            ("GAMEBASE_DEFAULTCHARA", game_base.default_character),
            ("GAMEBASE_NOITEM", game_base.no_item),
            ("__INT_MAX__", i64::MAX),
            ("__INT_MIN__", i64::MIN),
        ] {
            self.set_named_integer(artifact, name, value);
        }
        self.set_named_string(artifact, "LASTLOAD_TEXT", &defaults.last_load_text);
        for (name, value) in [
            ("GAMEBASE_AUTHER", game_base.author.as_str()),
            ("GAMEBASE_AUTHOR", game_base.author.as_str()),
            ("GAMEBASE_INFO", game_base.info.as_str()),
            ("GAMEBASE_YEAR", game_base.year.as_str()),
            ("GAMEBASE_TITLE", game_base.title.as_str()),
            ("GAMEBASE_URL", game_base.update_url.as_str()),
            ("GAMEBASE_VERSIONNAME", game_base.version_name.as_str()),
            (
                "WINDOW_TITLE",
                game_base.window_title.as_deref().unwrap_or_default(),
            ),
            ("MONEYLABEL", static_data.replace.money_label.as_str()),
            ("DRAWLINESTR", static_data.replace.draw_line_string.as_str()),
            ("EMUERA_VERSION", "1.824.0.0"),
        ] {
            self.set_named_string(artifact, name, value);
        }
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
        if let Some(definition) = shared_definition(artifact, name)
            && let Some(cell) = self.shared.get_mut(&definition.key)
        {
            let _ = cell.set(0, VmValue::Integer(value));
        }
    }

    pub(crate) fn set_runtime_calculated_string(
        &mut self,
        artifact: &BytecodeArtifact,
        name: &str,
        value: &str,
    ) {
        let Some(definition) = shared_definition(artifact, name) else {
            return;
        };
        if definition.storage != BytecodeStorage::Calculated
            || definition.value_type != BytecodeType::String
        {
            return;
        }
        if let Some(cell) = self.shared.get_mut(&definition.key) {
            let _ = cell.set(0, VmValue::String(value.into()));
        }
    }

    fn set_named_string(&mut self, artifact: &BytecodeArtifact, name: &str, value: &str) {
        if let Some(definition) = shared_definition(artifact, name)
            && let Some(cell) = self.shared.get_mut(&definition.key)
        {
            let _ = cell.set(0, VmValue::String(value.into()));
        }
    }

    fn set_named_values(&mut self, artifact: &BytecodeArtifact, name: &str, values: &[i64]) {
        if let Some(definition) = shared_definition(artifact, name)
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
        if let Some(definition) = shared_definition(artifact, name)
            && let Some(cell) = self.shared.get_mut(&definition.key)
        {
            for (index, value) in values.iter().take(cell.len()).enumerate() {
                let _ = cell.set(index, VmValue::String(value.clone().unwrap_or_default()));
            }
        }
    }

    pub fn target_character(&self, artifact: &BytecodeArtifact, generation: GenerationId) -> usize {
        self.target_character_from_definition(shared_definition(artifact, "TARGET"), generation)
    }

    #[inline]
    pub(crate) fn target_character_from_definition(
        &self,
        definition: Option<&BytecodeGlobal>,
        generation: GenerationId,
    ) -> usize {
        let Some(definition) = definition else {
            return 0;
        };
        let cell = if self.legacy.is_empty() {
            self.shared.get(&definition.key)
        } else {
            self.legacy
                .get(&generation)
                .and_then(|memory| memory.shared.get(&definition.key))
                .or_else(|| self.shared.get(&definition.key))
        };
        match cell.and_then(VariableCell::first) {
            Some(VmValue::Integer(value)) => usize::try_from(value).unwrap_or(0),
            _ => 0,
        }
    }

    #[inline]
    pub fn cell(
        &self,
        generation: GenerationId,
        definition: &BytecodeGlobal,
        character: usize,
    ) -> Option<&VariableCell> {
        if self.legacy.is_empty() {
            return match definition.storage {
                BytecodeStorage::Project
                | BytecodeStorage::Constant
                | BytecodeStorage::Calculated => self.shared.get(&definition.key),
                BytecodeStorage::FunctionStatic | BytecodeStorage::FunctionPersistent => {
                    self.statics.get(&definition.key)
                }
                BytecodeStorage::Character => self
                    .characters
                    .get(character)
                    .and_then(|values| values.get(&definition.key)),
                BytecodeStorage::FunctionLocal => None,
            };
        }
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

    #[inline]
    pub fn cell_mut(
        &mut self,
        generation: GenerationId,
        key: SymbolKey,
        storage: BytecodeStorage,
        character: usize,
    ) -> Option<&mut VariableCell> {
        if self.legacy.is_empty() {
            return match storage {
                BytecodeStorage::Project
                | BytecodeStorage::Constant
                | BytecodeStorage::Calculated => self.shared.get_mut(&key),
                BytecodeStorage::FunctionStatic | BytecodeStorage::FunctionPersistent => {
                    self.statics.get_mut(&key)
                }
                BytecodeStorage::Character => self
                    .characters
                    .get_mut(character)
                    .and_then(|values| values.get_mut(&key)),
                BytecodeStorage::FunctionLocal => None,
            };
        }
        let use_legacy = self
            .legacy
            .get(&generation)
            .is_some_and(|memory| match storage {
                BytecodeStorage::Project
                | BytecodeStorage::Constant
                | BytecodeStorage::Calculated => memory.shared.contains_key(&key),
                BytecodeStorage::FunctionStatic | BytecodeStorage::FunctionPersistent => {
                    memory.statics.contains_key(&key)
                }
                BytecodeStorage::Character => memory
                    .characters
                    .get(character)
                    .is_some_and(|values| values.contains_key(&key)),
                BytecodeStorage::FunctionLocal => false,
            });
        if use_legacy {
            let memory = self.legacy.get_mut(&generation)?;
            return match storage {
                BytecodeStorage::Project
                | BytecodeStorage::Constant
                | BytecodeStorage::Calculated => memory.shared.get_mut(&key),
                BytecodeStorage::FunctionStatic | BytecodeStorage::FunctionPersistent => {
                    memory.statics.get_mut(&key)
                }
                BytecodeStorage::Character => memory
                    .characters
                    .get_mut(character)
                    .and_then(|values| values.get_mut(&key)),
                BytecodeStorage::FunctionLocal => None,
            };
        }
        match storage {
            BytecodeStorage::Project | BytecodeStorage::Constant | BytecodeStorage::Calculated => {
                self.shared.get_mut(&key)
            }
            BytecodeStorage::FunctionStatic | BytecodeStorage::FunctionPersistent => {
                self.statics.get_mut(&key)
            }
            BytecodeStorage::Character => self
                .characters
                .get_mut(character)
                .and_then(|values| values.get_mut(&key)),
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
            characters: vec![VariableMap::default(); self.characters.len()],
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
        self.array_leases.migrate_generation(
            old_generation,
            &legacy.shared.keys().copied().collect(),
            &legacy.statics.keys().copied().collect(),
            &legacy
                .characters
                .iter()
                .flat_map(|row| row.keys().copied())
                .collect(),
        );
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
                    let migrated = self
                        .statics
                        .get(&definition.key)
                        .map(|cell| cell.migrate(definition));
                    if let Some(cell) = migrated {
                        self.statics.insert(definition.key, cell);
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
        self.initialized_static_functions
            .0
            .retain(|(cached, _)| *cached != generation);
    }
}

fn initialize_character(
    artifact: &BytecodeArtifact,
    cells: &mut VariableMap,
    template: &CharacterTemplate,
) {
    for (name, value) in [
        ("NO", VmValue::Integer(template.no)),
        ("NAME", VmValue::String(template.name.clone())),
        ("CALLNAME", VmValue::String(template.call_name.clone())),
        ("NICKNAME", VmValue::String(template.nick_name.clone())),
        ("MASTERNAME", VmValue::String(template.master_name.clone())),
    ] {
        if let Some(definition) = character_definition(artifact, name)
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
        if let Some(definition) = character_definition(artifact, name)
            && let Some(cell) = cells.get_mut(&definition.key)
        {
            for (index, value) in values {
                if *index < cell.len() {
                    let _ = cell.set(*index, VmValue::Integer(*value));
                }
            }
        }
    }
    if let Some(definition) = character_definition(artifact, "CSTR")
        && let Some(cell) = cells.get_mut(&definition.key)
    {
        for (index, value) in &template.cstr {
            if *index < cell.len() {
                let _ = cell.set(*index, VmValue::String(value.clone()));
            }
        }
    }
}
