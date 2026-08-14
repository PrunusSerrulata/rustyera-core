use erabasic_data::{
    NameAlias, NameTable, NameTableKind, Persistence, ProjectSchema, SaveCompatibility,
};

#[test]
fn fixed_catalog_contains_reference_shapes_and_scopes() {
    let schema = ProjectSchema::builtin_defaults();

    assert_eq!(schema.variable("FLAG").unwrap().dimensions, [10_000]);
    assert_eq!(schema.variable("RANDDATA").unwrap().dimensions, [625]);
    assert_eq!(schema.variable("CDFLAG").unwrap().dimensions, [1, 1]);
    assert_eq!(schema.variable("TA").unwrap().dimensions, [100, 100, 100]);
    assert_eq!(
        schema.variable("GLOBALS").unwrap().persistence,
        Persistence::GlobalSave
    );
    assert_eq!(schema.index_spaces[&NameTableKind::Palam].length, 200);
    assert_eq!(schema.index_spaces.len(), NameTableKind::ALL.len());
}

#[test]
fn builtin_name_tables_cover_shared_variables_and_dimensions() {
    assert_eq!(
        NameTableKind::for_data_variable("CUP", 0),
        Some(NameTableKind::Palam)
    );
    assert_eq!(
        NameTableKind::for_data_variable("cup", 0),
        Some(NameTableKind::Palam)
    );
    assert_eq!(
        NameTableKind::for_data_variable("CDFLAG", 0),
        Some(NameTableKind::Cdflag1)
    );
    assert_eq!(
        NameTableKind::for_data_variable("CDFLAG", 1),
        Some(NameTableKind::Cdflag2)
    );
    assert_eq!(NameTableKind::for_data_variable("CUP", 1), None);
}

#[test]
fn name_lookup_uses_first_name_then_non_shadowing_aliases() {
    let mut table = NameTable::empty(4);
    table.names[0] = Some("same".into());
    table.names[1] = Some("same".into());
    table.aliases.push(NameAlias {
        name: "same".into(),
        index: 3,
    });
    table.aliases.push(NameAlias {
        name: "alias".into(),
        index: -1,
    });
    table.rebuild_lookup();

    assert_eq!(table.lookup["same"], 0);
    assert_eq!(table.lookup["alias"], -1);
}

#[test]
fn serde_contract_is_round_trip_stable() {
    let schema = ProjectSchema::builtin_defaults();
    let json = serde_json::to_string(&schema).unwrap();
    let decoded: ProjectSchema = serde_json::from_str(&json).unwrap();

    assert_eq!(decoded, schema);
    assert_eq!(serde_json::to_string(&decoded).unwrap(), json);
}

#[test]
fn save_compatibility_matches_reference_wildcards() {
    let compatibility = SaveCompatibility {
        unique_code: 42,
        version: 1200,
        version_defined: true,
        compatible_min_version: 1100,
    };

    assert!(compatibility.accepts(0, 1150));
    assert!(compatibility.accepts(42, 1200));
    assert!(!compatibility.accepts(41, 1200));
    assert!(!compatibility.accepts(42, 1000));
}
