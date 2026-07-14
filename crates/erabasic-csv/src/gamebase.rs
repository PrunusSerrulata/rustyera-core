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
    for line in enabled_lines(&file.path, &file.content, options, diagnostics) {
        let tokens: Vec<_> = line.text.split(',').collect();
        if tokens.len() < 2 {
            continue;
        }
        match tokens[0] {
            "コード" => {
                if let Some(value) = parse_game_number(tokens[1]) {
                    game_base.unique_code = value;
                    if value == 0 {
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
                if let Some(value) = parse_game_number(tokens[1]) {
                    game_base.version = value;
                    game_base.version_defined = true;
                }
            }
            "バージョン違い認める" => {
                if let Some(value) = parse_game_number(tokens[1]) {
                    game_base.compatible_min_version = value;
                }
            }
            "最初からいるキャラ" => {
                if let Some(value) = parse_game_number(tokens[1]) {
                    game_base.default_character = value;
                }
            }
            "アイテムなし" => {
                if let Some(value) = parse_game_number(tokens[1]) {
                    game_base.no_item = value;
                }
            }
            "タイトル" => tokens[1].clone_into(&mut game_base.title),
            "作者" => tokens[1].clone_into(&mut game_base.author),
            "製作年" => tokens[1].clone_into(&mut game_base.year),
            "追加情報" => tokens[1].clone_into(&mut game_base.info),
            "ウィンドウタイトル" => {
                game_base.window_title = Some(tokens[1].to_owned());
                window_title_defined = true;
            }
            "動作に必要なEmueraのバージョン" => {
                tokens[1].clone_into(&mut game_base.required_emuera_version);
                let required = parse_version(tokens[1]);
                let current = parse_version(&options.current_emuera_version);
                match (required, current) {
                    (Some(required), Some(current)) if current < required => {
                        diagnostics.push(at_line(
                            CsvDiagnosticCode::RequiresNewerEmuera,
                            CsvDiagnosticSeverity::Fatal,
                            2,
                            &line,
                            format!("project requires Emuera {}", tokens[1]),
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
            "バージョン情報URL" => tokens[1].clone_into(&mut game_base.update_url),
            "バージョン名" => tokens[1].clone_into(&mut game_base.version_name),
            _ => {}
        }
    }
    if !window_title_defined {
        game_base.window_title = Some(if game_base.title.is_empty() {
            "Emuera".into()
        } else {
            format!("{} {}", game_base.title, version_text(game_base.version))
        });
    }
    (game_base, fatal)
}

fn parse_game_number(value: &str) -> Option<i64> {
    if let Ok(value) = value.trim().parse() {
        return Some(value);
    }
    let prefix: String = value.chars().take_while(char::is_ascii_digit).collect();
    (!prefix.is_empty()).then(|| prefix.parse().ok()).flatten()
}

fn parse_version(value: &str) -> Option<[u32; 4]> {
    let parts: Vec<_> = value.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    Some([
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
        parts[3].parse().ok()?,
    ])
}

fn version_text(version: i64) -> String {
    let fraction = version.rem_euclid(1000);
    if fraction % 10 != 0 {
        format!("{}.{fraction:03}", version / 1000)
    } else {
        format!("{}.{:02}", version / 1000, fraction / 10)
    }
}
