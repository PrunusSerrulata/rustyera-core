use std::collections::BTreeMap;

use erabasic_data::{ProjectSchema, StorageScope, ValueType, VariableSchema};
use erabasic_hir::{
    ConstantValue, FunctionId, FunctionKind, SemanticType, SourceLocation, Variable, VariableId,
    VariableScope,
};

use crate::{declarations::DeclaredVariable, options::AnalyzerOptions};

#[derive(Clone, Debug)]
pub(crate) struct FunctionSymbol {
    pub id: FunctionId,
    pub kind: FunctionKind,
    pub return_type: SemanticType,
}

pub(crate) struct Symbols {
    pub variables: Vec<Variable>,
    globals: BTreeMap<String, VariableId>,
    locals: BTreeMap<(FunctionId, String), VariableId>,
    local_templates: Vec<VariableSchema>,
    functions: Vec<FunctionSymbol>,
    functions_by_name: BTreeMap<String, usize>,
    ignore_case: bool,
}

impl Symbols {
    pub fn new(
        schema: &ProjectSchema,
        declarations: &BTreeMap<String, DeclaredVariable>,
        options: &AnalyzerOptions,
    ) -> Self {
        let mut result = Self {
            variables: Vec::new(),
            globals: BTreeMap::new(),
            locals: BTreeMap::new(),
            local_templates: Vec::new(),
            functions: Vec::new(),
            functions_by_name: BTreeMap::new(),
            ignore_case: options.ignore_case,
        };
        for variable in schema.variables.values() {
            if variable.storage == StorageScope::Local {
                result.local_templates.push(variable.clone());
                continue;
            }
            let initial_values = declarations
                .get(&result.key(variable.id.name()))
                .map_or_else(Vec::new, |declaration| declaration.initial_values.clone());
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
    ) -> Result<FunctionId, FunctionId> {
        let key = self.key(name);
        if let Some(existing) = self
            .functions_by_name
            .get(&key)
            .and_then(|index| self.functions.get(*index))
        {
            if kind == FunctionKind::Event && existing.kind == FunctionKind::Event {
                let id =
                    FunctionId(u32::try_from(self.functions.len()).expect("too many functions"));
                self.functions.push(FunctionSymbol {
                    id,
                    kind,
                    return_type,
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
        });
        Ok(id)
    }

    pub fn function(&self, name: &str) -> Option<&FunctionSymbol> {
        self.functions_by_name
            .get(&self.key(name))
            .and_then(|index| self.functions.get(*index))
    }

    pub fn prepare_function_locals(&mut self, function: FunctionId) {
        for schema in self.local_templates.clone() {
            self.add_variable(
                &schema,
                Some(function),
                VariableScope::Function,
                false,
                false,
                Vec::new(),
                None,
            );
        }
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
        Ok(self.add_variable(
            &declaration.schema,
            Some(function),
            VariableScope::Function,
            declaration.reference,
            declaration.static_lifetime,
            declaration.initial_values.clone(),
            Some(declaration.location),
        ))
    }

    pub fn resolve_variable(&self, function: FunctionId, name: &str) -> Option<&Variable> {
        let key = self.key(name);
        let id = self
            .locals
            .get(&(function, key.clone()))
            .or_else(|| self.globals.get(&key))?;
        self.variables.get(id.0 as usize)
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
            id,
            name: schema.id.name().to_owned(),
            value_type: semantic_type(schema.value_type),
            dimensions: schema.dimensions.clone(),
            storage: schema.storage,
            persistence: schema.persistence,
            mutable: schema.mutable,
            reference,
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
        if self.ignore_case {
            name.to_ascii_uppercase()
        } else {
            name.to_owned()
        }
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
