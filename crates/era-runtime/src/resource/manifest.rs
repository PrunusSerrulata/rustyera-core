use era_runtime_protocol::validate_relative_path;

use super::{ResourceDiagnostic, ResourceGraph, SpriteDefinition, SpriteFrame};

#[allow(clippy::too_many_lines)]
pub(super) fn parse_resource_manifest(
    graph: &mut ResourceGraph,
    diagnostics: &mut Vec<ResourceDiagnostic>,
    path: &str,
    text: &str,
) {
    let directory = path.rsplit_once('/').map_or("", |(directory, _)| directory);
    let mut current_animation: Option<String> = None;
    for (line_index, raw) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        let tokens = line.split(',').map(str::trim).collect::<Vec<_>>();
        if tokens.len() < 2 || tokens[0].is_empty() || tokens[1].is_empty() {
            continue;
        }
        let name = tokens[0].to_ascii_uppercase();
        if tokens[1].eq_ignore_ascii_case("ANIME") {
            let Some((width, height)) = parse_pair(&tokens, 2).filter(|(w, h)| *w > 0 && *h > 0)
            else {
                diagnostics.push(resource_error(
                    path,
                    line_index,
                    "runtime.invalid_animation_sprite",
                    "animation sprite requires positive width and height",
                ));
                current_animation = None;
                continue;
            };
            if graph.sprites.contains_key(&name) {
                diagnostics.push(resource_warning(
                    path,
                    line_index,
                    "runtime.duplicate_sprite",
                    format!("duplicate sprite {name} was ignored"),
                ));
                current_animation = None;
                continue;
            }
            graph.sprites.insert(
                name.clone(),
                SpriteDefinition {
                    name: name.clone(),
                    revision: graph.static_sprite_revision,
                    width: width.cast_unsigned(),
                    height: height.cast_unsigned(),
                    frames: Vec::new(),
                    dynamic: false,
                    position_x: 0,
                    position_y: 0,
                    canvas_id: None,
                    canvas_revision: None,
                    canvas_rectangle: None,
                },
            );
            current_animation = Some(name);
            continue;
        }
        let image_path = if directory.is_empty() {
            tokens[1].to_owned()
        } else {
            format!("{directory}/{}", tokens[1])
        };
        let Ok(image_path) = validate_relative_path(&image_path) else {
            diagnostics.push(resource_error(
                path,
                line_index,
                "runtime.invalid_resource_path",
                "resource CSV image path is invalid",
            ));
            current_animation = None;
            continue;
        };
        if !graph.images.contains_key(&image_path.to_ascii_lowercase()) {
            diagnostics.push(resource_error(
                path,
                line_index,
                "runtime.missing_resource_image",
                format!("resource image {image_path} was not submitted by the frontend"),
            ));
            current_animation = None;
            continue;
        }
        let rect = parse_quad(&tokens, 2);
        let offset = parse_pair(&tokens, 6).unwrap_or((0, 0));
        let delay_ms = tokens
            .get(8)
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(1_000);
        let destination = parse_pair(&tokens, 9)
            .filter(|(width, height)| *width > 0 && *height > 0)
            .map(|(width, height)| (width.cast_unsigned(), height.cast_unsigned()));
        let frame = SpriteFrame {
            image_path,
            content_digest: None,
            canvas_id: None,
            canvas_revision: None,
            source_x: rect.map_or(0, |value| value.0),
            source_y: rect.map_or(0, |value| value.1),
            source_width: rect.and_then(|value| u32::try_from(value.2).ok()),
            source_height: rect.and_then(|value| u32::try_from(value.3).ok()),
            offset_x: offset.0,
            offset_y: offset.1,
            delay_ms,
            destination_width: destination.map(|value| value.0),
            destination_height: destination.map(|value| value.1),
        };
        if current_animation.as_deref() == Some(name.as_str()) {
            if let Some(animation) = graph.sprites.get_mut(&name) {
                animation.frames.push(frame);
            }
            continue;
        }
        current_animation = None;
        if graph.sprites.contains_key(&name) {
            diagnostics.push(resource_warning(
                path,
                line_index,
                "runtime.duplicate_sprite",
                format!("duplicate sprite {name} was ignored"),
            ));
            continue;
        }
        graph.sprites.insert(
            name.clone(),
            SpriteDefinition {
                name,
                revision: graph.static_sprite_revision,
                width: destination.map_or(0, |value| value.0),
                height: destination.map_or(0, |value| value.1),
                frames: vec![frame],
                dynamic: false,
                position_x: 0,
                position_y: 0,
                canvas_id: None,
                canvas_revision: None,
                canvas_rectangle: None,
            },
        );
    }
}

fn parse_pair(tokens: &[&str], start: usize) -> Option<(i32, i32)> {
    Some((
        tokens.get(start)?.parse().ok()?,
        tokens.get(start + 1)?.parse().ok()?,
    ))
}

fn parse_quad(tokens: &[&str], start: usize) -> Option<(i32, i32, i32, i32)> {
    Some((
        tokens.get(start)?.parse().ok()?,
        tokens.get(start + 1)?.parse().ok()?,
        tokens.get(start + 2)?.parse().ok()?,
        tokens.get(start + 3)?.parse().ok()?,
    ))
}

fn resource_error(
    path: &str,
    line: usize,
    code: &'static str,
    message: impl Into<String>,
) -> ResourceDiagnostic {
    ResourceDiagnostic {
        code,
        path: path.into(),
        line: Some(u64::try_from(line + 1).unwrap_or(u64::MAX)),
        message: message.into(),
        error: true,
    }
}

fn resource_warning(
    path: &str,
    line: usize,
    code: &'static str,
    message: impl Into<String>,
) -> ResourceDiagnostic {
    ResourceDiagnostic {
        error: false,
        ..resource_error(path, line, code, message)
    }
}
