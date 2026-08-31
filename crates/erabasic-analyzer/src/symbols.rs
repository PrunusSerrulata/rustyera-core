use std::collections::BTreeMap;

use erabasic_data::{ProjectData, StorageScope, ValueType, VariableSchema};
use erabasic_hir::{
    ConstantValue, FunctionId, FunctionKind, SemanticType, SourceLocation, Variable, VariableId,
    VariableScope,
};

use crate::{
    declarations::{DeclaredVariable, RuntimeInitializer},
    identifiers::identifier_key,
    options::AnalyzerOptions,
};

#[derive(Clone, Debug)]
pub(crate) struct FunctionRuntimeInitializer {
    pub variable: VariableId,
    pub initializer: RuntimeInitializer,
}

#[derive(Clone, Debug)]
pub(crate) struct FunctionSymbol {
    pub id: FunctionId,
    pub kind: FunctionKind,
    pub return_type: SemanticType,
    pub parameter_count: usize,
}

pub(crate) struct Symbols {
    pub variables: Vec<Variable>,
    globals: BTreeMap<String, VariableId>,
    locals: BTreeMap<(FunctionId, String), VariableId>,
    era_locals: BTreeMap<(String, String), VariableId>,
    local_templates: Vec<VariableSchema>,
    functions: Vec<FunctionSymbol>,
    functions_by_name: BTreeMap<String, usize>,
    runtime_initializers: BTreeMap<FunctionId, Vec<FunctionRuntimeInitializer>>,
    allow_function_overloading: bool,
    ignore_case: bool,
}

impl Symbols {
    pub fn new(
        project: &ProjectData,
        declarations: &BTreeMap<String, DeclaredVariable>,
        options: &AnalyzerOptions,
    ) -> Self {
        let mut result = Self {
            variables: Vec::new(),
            globals: BTreeMap::new(),
            locals: BTreeMap::new(),
            era_locals: BTreeMap::new(),
            local_templates: Vec::new(),
            functions: Vec::new(),
            functions_by_name: BTreeMap::new(),
            runtime_initializers: BTreeMap::new(),
            allow_function_overloading: options.allow_function_overloading,
            ignore_case: options.ignore_case,
        };
        for variable in project.schema.variables.values() {
            if variable.storage == StorageScope::Local {
                result.local_templates.push(variable.clone());
                continue;
            }
            let initial_values = declarations
                .get(&result.key(variable.id.name()))
                .map(|declaration| declaration.initial_values.clone())
                .or_else(|| {
                    project
                        .static_data
                        .name_tables
                        .iter()
                        .find(|(kind, _)| {
                            kind.variable_name()
                                .eq_ignore_ascii_case(variable.id.name())
                        })
                        .map(|(_, table)| {
                            table
                                .names
                                .iter()
                                .map(|name| ConstantValue::String(name.clone().unwrap_or_default()))
                                .collect()
                        })
                })
                .unwrap_or_default();
            let location = declarations
                .get(&result.key(variable.id.name()))
                .map(|declaration| declaration.location);
            result.add_variable(
                variable,
                None,
                VariableScope::Project,
                false,
                true,
                initial_values,
                location,
            );
        }
        result
    }

    pub fn register_function(
        &mut self,
        name: &str,
        kind: FunctionKind,
        return_type: SemanticType,
        parameter_count: usize,
    ) -> Result<FunctionId, FunctionId> {
        let key = self.key(name);
        if let Some(existing) = self
            .functions_by_name
            .get(&key)
            .and_then(|index| self.functions.get(*index))
        {
            if (kind == FunctionKind::Event && existing.kind == FunctionKind::Event)
                || self.allow_function_overloading
            {
                // Emuera retains every definition but resolves an ordinary call
                // through the first sorted definition. Do not replace the name
                // map when compatibility permits same-name normal functions.
                let id =
                    FunctionId(u32::try_from(self.functions.len()).expect("too many functions"));
                self.functions.push(FunctionSymbol {
                    id,
                    kind,
                    return_type,
                    parameter_count,
                });
                return Ok(id);
            }
            return Err(existing.id);
        }
        let id = FunctionId(u32::try_from(self.functions.len()).expect("too many functions"));
        self.functions_by_name.insert(key, self.functions.len());
        self.functions.push(FunctionSymbol {
            id,
            kind,
            return_type,
            parameter_count,
        });
        Ok(id)
    }

    pub fn function(&self, name: &str) -> Option<&FunctionSymbol> {
        self.functions_by_name
            .get(&self.key(name))
            .and_then(|index| self.functions.get(*index))
    }

    pub fn prepare_function_locals(&mut self, function: FunctionId, function_name: &str) {
        let scope_name = self.key(function_name);
        for schema in self.local_templates.clone() {
            let variable_name = self.key(schema.id.name());
            let group = (scope_name.clone(), variable_name.clone());
            let id = if let Some(existing) = self.era_locals.get(&group).copied() {
                existing
            } else {
                let id = self.add_variable(
                    &schema,
                    Some(function),
                    VariableScope::EraFunction,
                    false,
                    false,
                    Vec::new(),
                    None,
                );
                self.era_locals.insert(group, id);
                id
            };
            self.locals.insert((function, variable_name), id);
        }
    }

    pub fn resize_era_local(&mut self, function: FunctionId, name: &str, size: usize) -> bool {
        let key = (function, self.key(name));
        let Some(variable) = self
            .locals
            .get(&key)
            .and_then(|id| self.variables.get_mut(id.0 as usize))
        else {
            return false;
        };
        if variable.scope != VariableScope::EraFunction
            || variable.dimensions.first().copied() == Some(0)
        {
            return false;
        }
        variable.dimensions = vec![size];
        true
    }

    pub fn register_private(
        &mut self,
        function: FunctionId,
        declaration: &DeclaredVariable,
    ) -> Result<VariableId, VariableId> {
        let key = (function, self.key(declaration.schema.id.name()));
        if let Some(existing) = self.locals.get(&key) {
            return Err(*existing);
        }
        let variable = self.add_variable(
            &declaration.schema,
            Some(function),
            VariableScope::Function,
            declaration.reference,
            declaration.static_lifetime,
            declaration.initial_values.clone(),
            Some(declaration.location),
        );
        if let Some(initializer) = &declaration.runtime_initializer {
            self.runtime_initializers.entry(function).or_default().push(
                FunctionRuntimeInitializer {
                    variable,
                    initializer: initializer.clone(),
                },
            );
        }
        Ok(variable)
    }

    pub fn runtime_initializers(&self, function: FunctionId) -> &[FunctionRuntimeInitializer] {
        self.runtime_initializers
            .get(&function)
            .map_or(&[], Vec::as_slice)
    }

    pub fn resolve_variable(&self, function: FunctionId, name: &str) -> Option<&Variable> {
        let key = self.key(name);
        let id = self
            .locals
            .get(&(function, key.clone()))
            .or_else(|| self.globals.get(&key))?;
        self.variables.get(id.0 as usize)
    }

    pub fn constant_values(&self) -> BTreeMap<String, ConstantValue> {
        self.variables
            .iter()
            .filter(|variable| variable.storage == StorageScope::Constant)
            .filter_map(|variable| {
                variable
                    .initial_values
                    .first()
                    .cloned()
                    .map(|value| (self.key(&variable.name), value))
            })
            .collect()
    }

    pub fn variable_dimensions(&self, function: FunctionId) -> BTreeMap<String, Vec<usize>> {
        self.variables
            .iter()
            .filter(|variable| variable.owner.is_none() || variable.owner == Some(function))
            .map(|variable| (self.key(&variable.name), variable.dimensions.clone()))
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn add_variable(
        &mut self,
        schema: &VariableSchema,
        owner: Option<FunctionId>,
        scope: VariableScope,
        reference: bool,
        static_lifetime: bool,
        initial_values: Vec<ConstantValue>,
        location: Option<SourceLocation>,
    ) -> VariableId {
        let id = VariableId(u32::try_from(self.variables.len()).expect("too many variables"));
        let key = self.key(schema.id.name());
        self.variables.push(Variable {
            reference_semantics: crate::reference_origin::variable_semantics(
                schema,
                reference,
                location.is_some(),
            ),
            id,
            name: schema.id.name().to_owned(),
            value_type: semantic_type(schema.value_type),
            dimensions: schema.dimensions.clone(),
            storage: schema.storage,
            persistence: schema.persistence,
            mutable: schema.mutable,
            reference,
            match_name_rejection: if reference
                || matches!(scope, VariableScope::Function | VariableScope::Parameter)
                || schema.is_enabled()
            {
                None
            } else if scope == VariableScope::EraFunction || schema.can_forbid {
                Some(erabasic_hir::MatchNameRejectionKind::Script)
            } else {
                Some(erabasic_hir::MatchNameRejectionKind::Internal)
            },
            character_disposal: if matches!(&schema.id, erabasic_data::VariableId::Builtin(_))
                && schema.storage == StorageScope::Character
                && schema.dimensions.len() == 1
            {
                erabasic_hir::CharacterArrayDisposal::ClearSparse
            } else {
                erabasic_hir::CharacterArrayDisposal::Preserve
            },
            static_lifetime,
            initial_values,
            scope,
            owner,
            location,
        });
        if let Some(function) = owner {
            self.locals.insert((function, key), id);
        } else {
            self.globals.insert(key, id);
        }
        id
    }

    fn key(&self, name: &str) -> String {
        identifier_key(name, self.ignore_case)
    }
}

pub(crate) fn semantic_type(value_type: ValueType) -> SemanticType {
    match value_type {
        ValueType::Integer => SemanticType::Integer,
        ValueType::String => SemanticType::String,
    }
}

pub(crate) fn is_reserved(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "IS" | "TO"
            | "INT"
            | "STR"
            | "REFFUNC"
            | "STATIC"
            | "DYNAMIC"
            | "GLOBAL"
            | "PRIVATE"
            | "SAVEDATA"
            | "CHARADATA"
            | "REF"
            | "__DEBUG__"
            | "__SKIP__"
            | "_"
    )
}
