use erabasic_data::GameBase;

use crate::{
    CsvDiagnostic, CsvDiagnosticCode, CsvDiagnosticSeverity, CsvLoadOptions, input::IndexedFile,
    reader::enabled_lines, tables::at_line,
};

pub(crate) fn load_game_base(
    file: Option<&IndexedFile>,
    options: &CsvLoadOptions,
    diagnostics: &mut Vec<CsvDiagnostic>,
) -> (GameBase, bool) {
    let mut game_base = GameBase::default();
    let Some(file) = file else {
        return (game_base, false);
    };
    let mut window_title_defined = false;
    let mut fatal = false;
    for line in enabled_lines(&file.source_path, &file.content, options, diagnostics) {
        let mut tokens = line.text.split(',');
        let key = tokens.next().unwrap_or_default();
        let Some(value) = tokens.next() else {
            continue;
        };
        match key {
            "コード" => {
                if let Some(number) = parse_game_number(value) {
                    game_base.unique_code = number;
                    if number == 0 {
                        diagnostics.push(at_line(
                            CsvDiagnosticCode::InvalidInteger,
                            CsvDiagnosticSeverity::Notice,
                            0,
                            &line,
                            "save code 0 is a wildcard",
                        ));
                    }
                }
            }
            "バージョン" => {
                if let Some(number) = parse_game_number(value) {
                    game_base.version = number;
                    game_base.version_defined = true;
                }
            }
            "バージョン違い認める" => {
                if let Some(number) = parse_game_number(value) {
                    game_base.compatible_min_version = number;
                }
            }
            "最初からいるキャラ" => {
                if let Some(number) = parse_game_number(value) {
                    game_base.default_character = number;
                }
            }
            "アイテムなし" => {
                if let Some(number) = parse_game_number(value) {
                    game_base.no_item = number;
                }
            }
            "タイトル" => value.clone_into(&mut game_base.title),
            "作者" => value.clone_into(&mut game_base.author),
            "製作年" => value.clone_into(&mut game_base.year),
            "追加情報" => value.clone_into(&mut game_base.info),
            "ウィンドウタイトル" => {
                game_base.window_title = Some(value.to_owned());
                window_title_defined = true;
            }
            "動作に必要なEmueraのバージョン" => {
                value.clone_into(&mut game_base.required_emuera_version);
                let required = parse_version(value);
                let current = parse_version(&options.current_emuera_version);
                match (required, current) {
                    (Some(required), Some(current)) if current < required => {
                        diagnostics.push(at_line(
                            CsvDiagnosticCode::RequiresNewerEmuera,
                            CsvDiagnosticSeverity::Fatal,
                            2,
                            &line,
                            format!("project requires Emuera {value}"),
                        ));
                        fatal = true;
                        break;
                    }
                    (None, _) => diagnostics.push(at_line(
                        CsvDiagnosticCode::InvalidGameVersion,
                        CsvDiagnosticSeverity::Notice,
                        0,
                        &line,
                        "required Emuera version must contain four numeric components",
                    )),
                    _ => {}
                }
            }
            "バージョン情報URL" => value.clone_into(&mut game_base.update_url),
            "バージョン名" => value.clone_into(&mut game_base.version_name),
            _ => {}
        }
    }
    if !window_title_defined {
        game_base.window_title = Some(if game_base.title.is_empty() {
            "Emuera".into()
        } else {
            format!("{} {}", game_base.title, game_base.script_version_text())
        });
    }
    (game_base, fatal)
}

fn parse_game_number(value: &str) -> Option<i64> {
    if let Ok(value) = value.trim().parse() {
        return Some(value);
    }
    let end = value
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(value.len());
    value[..end].parse().ok()
}

fn parse_version(value: &str) -> Option<[u32; 4]> {
    let mut parts = value.split('.');
    let version = [
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ];
    parts.next().is_none().then_some(version)
}
