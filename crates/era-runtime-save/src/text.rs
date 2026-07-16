use crate::{
    SaveCodecError, SaveCodecLimits, SaveDocument, SaveFileKind, SaveFormat, SaveMetadata,
};

const CURRENT_MARKER: &str = "__EMUERA_1808_STRAT__";

/// Decode a current text save without interpreting its project-specific positional fields.
///
/// Emuera's text format begins with an eramaker-compatible positional section. Its variable
/// names and array lengths only exist in the loaded project schema, so a schema-independent
/// codec cannot safely turn that section into named entries. We validate the common envelope
/// and retain the complete UTF-8 payload for the runtime's schema-aware adapter.
///
/// # Errors
///
/// Returns an error for non-UTF-8 data, missing metadata/marker, or a limit breach.
pub fn decode_text(data: &[u8], limits: SaveCodecLimits) -> Result<SaveDocument, SaveCodecError> {
    if data.len() > limits.maximum_bytes {
        return Err(SaveCodecError::LimitExceeded("maximum bytes"));
    }
    let data = data.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(data);
    let source = std::str::from_utf8(data)
        .map_err(|_| SaveCodecError::InvalidFormat("text save is not UTF-8".into()))?;
    let mut lines = source.lines();
    let unique_code = parse_integer(lines.next(), "unique code")?;
    let version = parse_integer(lines.next(), "script version")?;
    let description = lines
        .next()
        .ok_or_else(|| SaveCodecError::InvalidFormat("text save lacks a description".into()))?
        .trim_end_matches('\r')
        .to_owned();
    if !source
        .lines()
        .any(|line| line.trim_end_matches('\r') == CURRENT_MARKER)
    {
        return Err(SaveCodecError::InvalidFormat(
            "text save lacks the Emuera 1808 marker".into(),
        ));
    }
    Ok(SaveDocument {
        format: SaveFormat::Text1808,
        kind: SaveFileKind::Normal,
        metadata: SaveMetadata {
            unique_code,
            version,
            description,
        },
        characters: Vec::new(),
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

fn parse_integer(line: Option<&str>, field: &str) -> Result<i64, SaveCodecError> {
    line.ok_or_else(|| SaveCodecError::InvalidFormat(format!("text save lacks {field}")))?
        .trim_end_matches('\r')
        .parse()
        .map_err(|_| SaveCodecError::InvalidFormat(format!("text save has invalid {field}")))
}
