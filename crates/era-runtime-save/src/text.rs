use crate::{
    SaveCodecError, SaveCodecLimits, SaveDocument, SaveEntry, SaveFileKind, SaveFormat,
    SaveMetadata, SaveValue, Text1808Layout, Text1808ValueType, Text1808Variable,
};

const CURRENT_MARKER: &str = "__EMUERA_1808_STRAT__";
const HISTORICAL_MARKERS: [&str; 4] = [
    "__EMUERA_STRAT__",
    "__EMUERA_1708_STRAT__",
    "__EMUERA_1729_STRAT__",
    "__EMUERA_1803_STRAT__",
];
const FINISHER: &str = "__FINISHED";
const SEPARATOR: &str = "__EMU_SEPARATOR__";

mod value;

use value::{scalar_string, trimmed_values};

/// Decode a current text save without interpreting its project-specific positional fields.
///
/// Emuera's text format begins with an eramaker-compatible positional section. Its variable
/// names and array lengths only exist in the loaded project schema, so a schema-independent
/// codec cannot safely turn that section into named entries. We validate the common envelope
/// and retain the complete UTF-8 payload for the runtime's schema-aware adapter.
///
/// # Errors
///
/// Returns an error for non-UTF-8 data, missing metadata, or a limit breach. The metadata-only
/// path accepts pre-Emuera eramaker saves which have no extension marker; schema-aware decoding
/// decides whether their positional payload can be restored.
pub fn decode_text(data: &[u8], limits: SaveCodecLimits) -> Result<SaveDocument, SaveCodecError> {
    if data.len() > limits.maximum_bytes {
        return Err(SaveCodecError::LimitExceeded("maximum bytes"));
    }
    let data = data.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(data);
    let source = std::str::from_utf8(data)
        .map_err(|_| SaveCodecError::InvalidFormat("text save is not UTF-8".into()))?;
    let mut reader = TextReader::new(source);
    let unique_code = reader.integer("unique code")?;
    let version = reader.integer("script version")?;
    let description = reader.line("a description")?.to_owned();
    Ok(SaveDocument {
        format: SaveFormat::Text1808,
        kind: SaveFileKind::Normal,
        metadata: SaveMetadata {
            unique_code,
            version,
            description,
        },
        characters: Vec::new(),
        character_user_defined_starts: Vec::new(),
        variables: Vec::new(),
        opaque_extensions: Vec::new(),
        text_payload: Some(data.to_vec()),
    })
}

/// Encode a text document using its schema-aware payload.
///
/// The runtime constructs this payload from the active project schema. Requiring it here keeps
/// the generic codec from silently producing a positional save with the wrong variable layout.
///
/// # Errors
///
/// Returns an error when the payload is absent, invalid, inconsistent, or too large.
pub fn encode_text(
    document: &SaveDocument,
    limits: SaveCodecLimits,
) -> Result<Vec<u8>, SaveCodecError> {
    let payload = document.text_payload.as_ref().ok_or_else(|| {
        SaveCodecError::InvalidFormat(
            "text encoding requires a schema-aware positional payload".into(),
        )
    })?;
    if payload.len() > limits.maximum_bytes {
        return Err(SaveCodecError::LimitExceeded("maximum bytes"));
    }
    let decoded = decode_text(payload, limits)?;
    if decoded.metadata != document.metadata {
        return Err(SaveCodecError::InvalidFormat(
            "text payload metadata differs from the save document".into(),
        ));
    }
    Ok(payload.clone())
}

/// Decode a current UTF-8 text save using the active project's positional schema.
///
/// # Errors
///
/// Returns an error before producing a document when any required position,
/// separator, value, or array shape is malformed.
pub fn decode_text_with_layout(
    data: &[u8],
    layout: &Text1808Layout,
    limits: SaveCodecLimits,
) -> Result<SaveDocument, SaveCodecError> {
    if data.len() > limits.maximum_bytes {
        return Err(SaveCodecError::LimitExceeded("maximum bytes"));
    }
    let data = data.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(data);
    let source = std::str::from_utf8(data)
        .map_err(|_| SaveCodecError::InvalidFormat("text save is not UTF-8".into()))?;
    let mut reader = TextReader::new(source);
    let unique_code = reader.integer("unique code")?;
    let version = reader.integer("script version")?;
    let description = if layout.kind == SaveFileKind::Normal {
        reader.line("description")?.to_owned()
    } else {
        String::new()
    };
    let mut characters = Vec::new();
    if layout.kind == SaveFileKind::Normal {
        let count = usize::try_from(reader.integer("character count")?).map_err(|_| {
            SaveCodecError::InvalidFormat("text save has a negative character count".into())
        })?;
        if count > limits.maximum_characters {
            return Err(SaveCodecError::LimitExceeded("maximum characters"));
        }
        for _ in 0..count {
            characters.push(read_base_entries(
                &mut reader,
                &layout.base_character_variables,
                limits,
            )?);
        }
    }
    let mut variables = read_base_entries(&mut reader, &layout.base_variables, limits)?;
    let Some(marker) = reader.seek_extension_marker() else {
        // EraMaker saves end after the positional prefix. Emuera restores that prefix and leaves
        // every later variable at its project default, so the same document is useful here.
        return finish_text_document(
            data,
            layout.kind,
            SaveMetadata {
                unique_code,
                version,
                description,
            },
            characters,
            variables,
            limits,
        );
    };
    let extension_version = match marker {
        "__EMUERA_STRAT__" => 1700,
        "__EMUERA_1708_STRAT__" => 1708,
        "__EMUERA_1729_STRAT__" => 1729,
        "__EMUERA_1803_STRAT__" => 1803,
        CURRENT_MARKER => 1808,
        _ => unreachable!("seek_extension_marker returned an unknown marker"),
    };
    let character_group_count = if extension_version < 1803 {
        layout.extended_character_groups.len().min(4)
    } else {
        layout.extended_character_groups.len()
    };
    for character in &mut characters {
        read_extended_groups(
            &mut reader,
            &layout.extended_character_groups[..character_group_count],
            &layout.unsupported_extended_character_groups,
            character,
            limits,
        )?;
    }
    let variable_group_count = if extension_version < 1808 {
        layout.extended_groups.len().min(8)
    } else {
        layout.extended_groups.len()
    };
    read_extended_groups(
        &mut reader,
        &layout.extended_groups[..variable_group_count],
        &layout.unsupported_extended_groups,
        &mut variables,
        limits,
    )?;
    if reader.lines.next().is_some() {
        return Err(SaveCodecError::InvalidFormat(
            "text save contains trailing extended groups or data".into(),
        ));
    }
    finish_text_document(
        data,
        layout.kind,
        SaveMetadata {
            unique_code,
            version,
            description,
        },
        characters,
        variables,
        limits,
    )
}

fn finish_text_document(
    data: &[u8],
    kind: SaveFileKind,
    metadata: SaveMetadata,
    characters: Vec<Vec<SaveEntry>>,
    variables: Vec<SaveEntry>,
    limits: SaveCodecLimits,
) -> Result<SaveDocument, SaveCodecError> {
    if variables.len() + characters.iter().map(Vec::len).sum::<usize>() > limits.maximum_entries {
        return Err(SaveCodecError::LimitExceeded("maximum entries"));
    }
    Ok(SaveDocument {
        format: SaveFormat::Text1808,
        kind,
        metadata,
        character_user_defined_starts: vec![None; characters.len()],
        characters,
        variables,
        opaque_extensions: Vec::new(),
        text_payload: Some(data.to_vec()),
    })
}

/// Encode a schema-aware current text save as UTF-8 BOM plus CRLF lines.
///
/// # Errors
///
/// Returns an error for unsupported string arrays above one dimension, invalid
/// shapes, embedded newlines, or configured size limits.
pub fn encode_text_with_layout(
    document: &SaveDocument,
    layout: &Text1808Layout,
    limits: SaveCodecLimits,
) -> Result<Vec<u8>, SaveCodecError> {
    if document.kind != layout.kind {
        return Err(SaveCodecError::InvalidFormat(
            "text layout kind differs from save document".into(),
        ));
    }
    let mut writer = TextWriter::default();
    writer.line(&document.metadata.unique_code.to_string())?;
    writer.line(&document.metadata.version.to_string())?;
    if layout.kind == SaveFileKind::Normal {
        writer.line(&document.metadata.description)?;
        writer.line(&document.characters.len().to_string())?;
        for character in &document.characters {
            write_base_entries(&mut writer, &layout.base_character_variables, character)?;
        }
    }
    write_base_entries(&mut writer, &layout.base_variables, &document.variables)?;
    writer.line(CURRENT_MARKER)?;
    for character in &document.characters {
        write_extended_groups(&mut writer, &layout.extended_character_groups, character)?;
    }
    write_extended_groups(&mut writer, &layout.extended_groups, &document.variables)?;
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(writer.output.as_bytes());
    if bytes.len() > limits.maximum_bytes {
        return Err(SaveCodecError::LimitExceeded("maximum bytes"));
    }
    // Decode our own output to enforce element and entry limits at the API edge.
    let _ = decode_text_with_layout(&bytes, layout, limits)?;
    Ok(bytes)
}

struct TextReader<'a> {
    lines: std::str::Lines<'a>,
}

impl<'a> TextReader<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            lines: source.lines(),
        }
    }

    fn line(&mut self, field: &str) -> Result<&'a str, SaveCodecError> {
        self.lines
            .next()
            .map(|line| line.trim_end_matches('\r'))
            .ok_or_else(|| SaveCodecError::InvalidFormat(format!("text save lacks {field}")))
    }

    fn integer(&mut self, field: &str) -> Result<i64, SaveCodecError> {
        self.line(field)?
            .parse()
            .map_err(|_| SaveCodecError::InvalidFormat(format!("text save has invalid {field}")))
    }

    /// Match `EraDataReader.SeekEmuStart`: obsolete writers may leave ignorable positional tail
    /// lines before the extension marker, while pure `EraMaker` saves have no marker at all.
    fn seek_extension_marker(&mut self) -> Option<&'a str> {
        self.lines
            .by_ref()
            .map(|line| line.trim_end_matches('\r'))
            .find(|line| *line == CURRENT_MARKER || HISTORICAL_MARKERS.contains(line))
    }
}

#[derive(Default)]
struct TextWriter {
    output: String,
}

impl TextWriter {
    fn line(&mut self, value: &str) -> Result<(), SaveCodecError> {
        if value.contains(['\r', '\n']) {
            return Err(SaveCodecError::InvalidFormat(
                "text save values cannot contain newlines".into(),
            ));
        }
        self.output.push_str(value);
        self.output.push_str("\r\n");
        Ok(())
    }
}

fn read_base_entries(
    reader: &mut TextReader<'_>,
    variables: &[Text1808Variable],
    limits: SaveCodecLimits,
) -> Result<Vec<SaveEntry>, SaveCodecError> {
    variables
        .iter()
        .map(|variable| {
            let value = if variable.dimensions.is_empty() {
                read_scalar(reader, variable.value_type, &variable.name)?
            } else {
                read_base_array(reader, variable, limits)?
            };
            Ok(SaveEntry {
                name: variable.name.clone(),
                value,
            })
        })
        .collect()
}

fn read_scalar(
    reader: &mut TextReader<'_>,
    value_type: Text1808ValueType,
    field: &str,
) -> Result<SaveValue, SaveCodecError> {
    match value_type {
        Text1808ValueType::Integer => Ok(SaveValue::Integer(reader.integer(field)?)),
        Text1808ValueType::String => Ok(SaveValue::String(reader.line(field)?.to_owned())),
    }
}

fn read_base_array(
    reader: &mut TextReader<'_>,
    variable: &Text1808Variable,
    limits: SaveCodecLimits,
) -> Result<SaveValue, SaveCodecError> {
    if variable.dimensions.len() != 1 {
        return Err(SaveCodecError::InvalidFormat(
            "positional text arrays must be one-dimensional".into(),
        ));
    }
    let length = usize::try_from(variable.dimensions[0]).unwrap_or(usize::MAX);
    let mut strings = Vec::new();
    while strings.len() <= limits.maximum_elements {
        let line = reader.line(&variable.name)?;
        if line == FINISHER {
            break;
        }
        strings.push(line.to_owned());
    }
    if strings.len() > limits.maximum_elements {
        return Err(SaveCodecError::LimitExceeded("maximum elements"));
    }
    strings.truncate(length);
    strings.resize(length, default_text(variable.value_type).to_owned());
    strings_to_value(variable, strings)
}

fn strings_to_value(
    variable: &Text1808Variable,
    strings: Vec<String>,
) -> Result<SaveValue, SaveCodecError> {
    match variable.value_type {
        Text1808ValueType::Integer => Ok(SaveValue::Integers {
            dimensions: variable.dimensions.clone(),
            values: strings
                .into_iter()
                .map(|value| {
                    value.parse().map_err(|_| {
                        SaveCodecError::InvalidFormat(format!(
                            "text save has invalid {} element",
                            variable.name
                        ))
                    })
                })
                .collect::<Result<_, _>>()?,
        }),
        Text1808ValueType::String => Ok(SaveValue::Strings {
            dimensions: variable.dimensions.clone(),
            values: strings,
        }),
    }
}

fn read_extended_groups(
    reader: &mut TextReader<'_>,
    groups: &[Vec<Text1808Variable>],
    unsupported_groups: &[bool],
    entries: &mut Vec<SaveEntry>,
    limits: SaveCodecLimits,
) -> Result<(), SaveCodecError> {
    for (group_index, group) in groups.iter().enumerate() {
        let mut parsed = std::collections::BTreeMap::new();
        loop {
            let line = reader.line("extended group")?;
            if line == SEPARATOR {
                break;
            }
            if unsupported_groups
                .get(group_index)
                .is_some_and(|unsupported| *unsupported)
            {
                return Err(SaveCodecError::InvalidFormat(
                    "unsupported Float value in text save".into(),
                ));
            }
            let variable = if let Some((name, value)) = line.split_once(':') {
                let Some(descriptor) = find_layout(group, name) else {
                    // Extended dictionaries are forward compatible: variables
                    // removed from the active project are ignored by Emuera.
                    continue;
                };
                if !descriptor.dimensions.is_empty() {
                    return Err(SaveCodecError::InvalidFormat(
                        "array text entry used scalar syntax".into(),
                    ));
                }
                parsed.insert(
                    descriptor.name.to_ascii_uppercase(),
                    SaveEntry {
                        name: descriptor.name.clone(),
                        value: match descriptor.value_type {
                            Text1808ValueType::Integer => {
                                SaveValue::Integer(value.parse().map_err(|_| {
                                    SaveCodecError::InvalidFormat("invalid extended integer".into())
                                })?)
                            }
                            Text1808ValueType::String => SaveValue::String(value.to_owned()),
                        },
                    },
                );
                continue;
            } else {
                let Some(descriptor) = find_layout(group, line) else {
                    skip_extended_array(reader, limits)?;
                    continue;
                };
                descriptor
            };
            let values = read_extended_array(reader, variable, limits)?;
            parsed.insert(
                variable.name.to_ascii_uppercase(),
                SaveEntry {
                    name: variable.name.clone(),
                    value: values,
                },
            );
        }
        for descriptor in group {
            if let Some(entry) = parsed.remove(&descriptor.name.to_ascii_uppercase()) {
                entries.push(entry);
            }
        }
    }
    Ok(())
}

fn find_layout<'a>(group: &'a [Text1808Variable], name: &str) -> Option<&'a Text1808Variable> {
    group
        .iter()
        .find(|variable| variable.name.eq_ignore_ascii_case(name))
}

fn read_extended_array(
    reader: &mut TextReader<'_>,
    variable: &Text1808Variable,
    limits: SaveCodecLimits,
) -> Result<SaveValue, SaveCodecError> {
    if variable.value_type == Text1808ValueType::String && variable.dimensions.len() > 1 {
        return Err(SaveCodecError::InvalidFormat(
            "current text saves do not support multidimensional string arrays".into(),
        ));
    }
    let mut flattened = Vec::new();
    let dimensions = variable
        .dimensions
        .iter()
        .map(|value| usize::try_from(*value).unwrap_or(usize::MAX))
        .collect::<Vec<_>>();
    match dimensions.as_slice() {
        [_] => loop {
            let line = reader.line(&variable.name)?;
            if line == FINISHER {
                break;
            }
            flattened.push(line.to_owned());
        },
        [_, width] => loop {
            let line = reader.line(&variable.name)?;
            if line == FINISHER {
                break;
            }
            flattened.extend(padded_row(line, *width));
        },
        [_, rows, width] => loop {
            let line = reader.line(&variable.name)?;
            if line == FINISHER {
                break;
            }
            if !line.ends_with('{') {
                return Err(SaveCodecError::InvalidFormat(
                    "invalid extended three-dimensional array".into(),
                ));
            }
            let mut row_count = 0;
            loop {
                let row = reader.line(&variable.name)?;
                if row == "}" {
                    break;
                }
                if row_count < *rows {
                    flattened.extend(padded_row(row, *width));
                }
                row_count += 1;
            }
            while row_count < *rows {
                flattened.extend(std::iter::repeat_n("0".into(), *width));
                row_count += 1;
            }
        },
        _ => {
            return Err(SaveCodecError::InvalidFormat(
                "unsupported text array rank".into(),
            ));
        }
    }
    let total = dimensions
        .iter()
        .try_fold(1usize, |total, value| total.checked_mul(*value))
        .ok_or(SaveCodecError::LimitExceeded("maximum elements"))?;
    if total > limits.maximum_elements {
        return Err(SaveCodecError::LimitExceeded("maximum elements"));
    }
    flattened.truncate(total);
    flattened.resize(total, "0".into());
    strings_to_value(variable, flattened)
}

fn write_base_entries(
    writer: &mut TextWriter,
    layout: &[Text1808Variable],
    entries: &[SaveEntry],
) -> Result<(), SaveCodecError> {
    for descriptor in layout {
        let Some(value) = find_entry(entries, &descriptor.name).map(|entry| &entry.value) else {
            if descriptor.dimensions.is_empty() {
                writer.line(default_text(descriptor.value_type))?;
            } else {
                writer.line(FINISHER)?;
            }
            continue;
        };
        if descriptor.dimensions.is_empty() {
            write_scalar(writer, value, descriptor.value_type)?;
        } else {
            write_trimmed_1d(writer, value, descriptor)?;
        }
    }
    Ok(())
}

fn default_text(value_type: Text1808ValueType) -> &'static str {
    match value_type {
        Text1808ValueType::Integer => "0",
        Text1808ValueType::String => "",
    }
}

fn padded_row(line: &str, width: usize) -> Vec<String> {
    let mut values = if line.is_empty() {
        Vec::new()
    } else {
        line.split(',').map(str::to_owned).collect()
    };
    values.truncate(width);
    values.resize(width, "0".into());
    values
}

fn skip_extended_array(
    reader: &mut TextReader<'_>,
    limits: SaveCodecLimits,
) -> Result<(), SaveCodecError> {
    let mut ignored = 0usize;
    loop {
        if reader.line("unknown extended array")? == FINISHER {
            return Ok(());
        }
        ignored += 1;
        if ignored > limits.maximum_elements {
            return Err(SaveCodecError::LimitExceeded("maximum elements"));
        }
    }
}

fn write_extended_groups(
    writer: &mut TextWriter,
    groups: &[Vec<Text1808Variable>],
    entries: &[SaveEntry],
) -> Result<(), SaveCodecError> {
    for group in groups {
        for descriptor in group {
            let Some(entry) = find_entry(entries, &descriptor.name) else {
                continue;
            };
            write_extended(writer, descriptor, &entry.value)?;
        }
        writer.line(SEPARATOR)?;
    }
    Ok(())
}

fn find_entry<'a>(entries: &'a [SaveEntry], name: &str) -> Option<&'a SaveEntry> {
    entries
        .iter()
        .find(|entry| entry.name.eq_ignore_ascii_case(name))
}

fn write_scalar(
    writer: &mut TextWriter,
    value: &SaveValue,
    value_type: Text1808ValueType,
) -> Result<(), SaveCodecError> {
    match (value_type, value) {
        (Text1808ValueType::Integer, SaveValue::Integer(value)) => writer.line(&value.to_string()),
        (Text1808ValueType::String, SaveValue::String(value)) => writer.line(value),
        _ => Err(SaveCodecError::InvalidFormat(
            "text value type differs from its layout".into(),
        )),
    }
}

fn write_trimmed_1d(
    writer: &mut TextWriter,
    value: &SaveValue,
    descriptor: &Text1808Variable,
) -> Result<(), SaveCodecError> {
    if descriptor.dimensions.len() != 1 {
        return Err(SaveCodecError::InvalidFormat(
            "positional text arrays must be one-dimensional".into(),
        ));
    }
    for value in trimmed_values(value)? {
        writer.line(&value)?;
    }
    writer.line(FINISHER)
}

fn write_extended(
    writer: &mut TextWriter,
    descriptor: &Text1808Variable,
    value: &SaveValue,
) -> Result<(), SaveCodecError> {
    if descriptor.dimensions.is_empty() {
        let rendered = scalar_string(value)?;
        if rendered.is_empty() || rendered == "0" {
            return Ok(());
        }
        return writer.line(&format!("{}:{rendered}", descriptor.name));
    }
    if descriptor.value_type == Text1808ValueType::String && descriptor.dimensions.len() > 1 {
        return Err(SaveCodecError::InvalidFormat(
            "current text saves do not support multidimensional string arrays".into(),
        ));
    }
    let values = trimmed_values(value)?;
    if values.is_empty() {
        return Ok(());
    }
    writer.line(&descriptor.name)?;
    match descriptor.dimensions.as_slice() {
        [_] => {
            for value in values {
                writer.line(&value)?;
            }
        }
        [_, width] => {
            let width = usize::try_from(*width).unwrap_or(usize::MAX);
            for row in values.chunks(width) {
                writer.line(&row.join(","))?;
            }
        }
        [_, rows, width] => {
            let rows = usize::try_from(*rows).unwrap_or(usize::MAX);
            let width = usize::try_from(*width).unwrap_or(usize::MAX);
            let plane = rows.saturating_mul(width);
            for (x, values) in values.chunks(plane).enumerate() {
                writer.line(&format!("{x}{{"))?;
                for row in values.chunks(width) {
                    writer.line(&row.join(","))?;
                }
                writer.line("}")?;
            }
        }
        _ => {
            return Err(SaveCodecError::InvalidFormat(
                "unsupported text array rank".into(),
            ));
        }
    }
    writer.line(FINISHER)
}
