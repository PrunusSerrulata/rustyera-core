use erabasic_data::{LegacyEncoding, PROJECT_DATA_FORMAT_VERSION, ProjectData, ProjectStaticData};
use serde::{Deserialize, Serialize};

use crate::{
    CsvDiagnostic, CsvLoadOptions,
    characters::load_characters,
    deferred::collect_deferred_indices,
    extensions::load_extensions,
    gamebase::load_game_base,
    input::{FileIndex, ProjectFiles},
    special::{load_rename, load_replace},
    tables::load_name_tables,
    variable_size::load_variable_sizes,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CsvLoadReport {
    pub data: Option<ProjectData>,
    pub diagnostics: Vec<CsvDiagnostic>,
}

/// Load a complete virtual project snapshot without performing filesystem I/O.
#[must_use]
pub fn load_project(files: &ProjectFiles, options: &CsvLoadOptions) -> CsvLoadReport {
    let mut diagnostics = Vec::new();
    let index = FileIndex::build(files, &mut diagnostics);

    let replace = load_replace(index.csv_file("_Replace.csv"), options, &mut diagnostics);
    let rename = load_rename(index.csv_file("_Rename.csv"), options, &mut diagnostics);
    let (game_base, game_base_fatal) =
        load_game_base(index.csv_file("GAMEBASE.CSV"), options, &mut diagnostics);
    if game_base_fatal {
        return CsvLoadReport {
            data: None,
            diagnostics,
        };
    }

    let (schema, schema_fatal) = load_variable_sizes(
        index.csv_file("VariableSize.CSV"),
        options,
        &mut diagnostics,
    );
    if schema_fatal {
        return CsvLoadReport {
            data: None,
            diagnostics,
        };
    }
    let (name_tables, item_prices) = load_name_tables(&index, &schema, options, &mut diagnostics);
    let (characters, relation_lookup) =
        load_characters(&index, &schema, &name_tables, options, &mut diagnostics);
    let extensions = load_extensions(&index, options, &mut diagnostics);
    let deferred_indices = collect_deferred_indices(&index, options);

    CsvLoadReport {
        data: Some(ProjectData {
            format_version: PROJECT_DATA_FORMAT_VERSION,
            schema,
            static_data: ProjectStaticData {
                legacy_encoding: LegacyEncoding::default(),
                game_base,
                name_tables,
                item_prices,
                characters,
                relation_lookup,
                extensions,
                rename,
                replace,
                deferred_indices,
            },
        }),
        diagnostics,
    }
}
