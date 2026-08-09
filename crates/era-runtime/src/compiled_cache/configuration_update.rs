use era_config::{ReraConfigDocument, normalize_line_endings};
use era_protocol::ProtocolBytes;
use era_runtime_protocol::{FileCategory, FilePayload, ProjectManifest, SubmittedFile};

const RECORD_MAGIC: &[u8; 8] = b"RERACFG1";
const FOOTER_MAGIC: &[u8; 8] = b"RERACEND";
const RECORD_VERSION: u8 = 1;
const FLAG_PREVIOUS_SOURCE: u8 = 1;
const FIXED_PREFIX_BYTES: usize = 8 + 1 + 1 + 2 + 4 + 32 + 32;
const FIXED_SUFFIX_BYTES: usize = 32 + 4 + 8;

#[derive(Clone, Copy, Debug)]
pub(super) struct ConfigurationUpdateRef<'a> {
    source_digest: [u8; 32],
    source: &'a str,
}

#[derive(Debug)]
pub(super) struct ConfigurationJournal<'a> {
    first_previous_exists: bool,
    first_previous_digest: [u8; 32],
    final_update: Option<ConfigurationUpdateRef<'a>>,
    pub(super) valid_end: usize,
}

pub(super) fn parse_journal(
    bytes: &[u8],
    base_end: usize,
) -> Result<ConfigurationJournal<'_>, String> {
    let mut cursor = base_end;
    let mut first_previous_exists = false;
    let mut first_previous_digest = [0; 32];
    let mut final_update: Option<ConfigurationUpdateRef<'_>> = None;
    while cursor < bytes.len() {
        let remaining = &bytes[cursor..];
        if remaining.len() < RECORD_MAGIC.len() {
            if RECORD_MAGIC.starts_with(remaining) {
                break;
            }
            return Err("project configuration journal has trailing data".into());
        }
        if &remaining[..RECORD_MAGIC.len()] != RECORD_MAGIC {
            return Err("project configuration journal has an invalid record header".into());
        }
        if remaining.len() < FIXED_PREFIX_BYTES {
            break;
        }
        let version = remaining[8];
        let flags = remaining[9];
        if version != RECORD_VERSION {
            return Err(format!(
                "unsupported project configuration record version {version:02x}"
            ));
        }
        if flags & !FLAG_PREVIOUS_SOURCE != 0 || remaining[10..12] != [0, 0] {
            return Err("project configuration record has unsupported flags".into());
        }
        let source_length = u32::from_le_bytes(
            remaining[12..16]
                .try_into()
                .expect("four-byte source length"),
        ) as usize;
        let record_length = FIXED_PREFIX_BYTES
            .checked_add(source_length)
            .and_then(|length| length.checked_add(FIXED_SUFFIX_BYTES))
            .ok_or("project configuration record length overflows")?;
        if remaining.len() < record_length {
            break;
        }
        let record = &remaining[..record_length];
        let footer_offset = record_length - FOOTER_MAGIC.len();
        let length_offset = footer_offset - 4;
        let checksum_offset = length_offset - 32;
        if &record[footer_offset..] != FOOTER_MAGIC
            || u32::from_le_bytes(
                record[length_offset..footer_offset]
                    .try_into()
                    .expect("four-byte record length"),
            ) as usize
                != record_length
        {
            return Err("project configuration record has an invalid footer".into());
        }
        if blake3::hash(&record[..checksum_offset]).as_bytes()
            != &record[checksum_offset..length_offset]
        {
            return Err("project configuration record checksum mismatch".into());
        }
        let previous_digest = record[16..48].try_into().expect("32-byte previous digest");
        let previous_exists = flags & FLAG_PREVIOUS_SOURCE != 0;
        let source_digest = record[48..80].try_into().expect("32-byte source digest");
        let source = std::str::from_utf8(&record[FIXED_PREFIX_BYTES..checksum_offset])
            .map_err(|_| "project configuration record is not valid UTF-8")?;
        if source.contains('\r') {
            return Err("project configuration record does not use normalized LF endings".into());
        }
        if blake3::hash(source.as_bytes()).as_bytes() != &source_digest {
            return Err("project configuration record source digest mismatch".into());
        }
        ReraConfigDocument::parse(source)
            .map_err(|error| format!("project configuration record is invalid: {error}"))?;
        if let Some(previous) = final_update {
            if !previous_exists || previous_digest != previous.source_digest {
                return Err("project configuration journal has a broken digest chain".into());
            }
        } else {
            first_previous_exists = previous_exists;
            first_previous_digest = previous_digest;
        }
        final_update = Some(ConfigurationUpdateRef {
            source_digest,
            source,
        });
        cursor += record_length;
    }
    Ok(ConfigurationJournal {
        first_previous_exists,
        first_previous_digest,
        final_update,
        valid_end: cursor,
    })
}

pub(super) fn apply_journal(
    manifest: &mut ProjectManifest,
    journal: &ConfigurationJournal<'_>,
) -> Result<(), String> {
    let Some(final_update) = journal.final_update else {
        return Ok(());
    };
    let current = configuration_digest(manifest)?;
    if current.is_some() != journal.first_previous_exists
        || current.is_some_and(|digest| digest != journal.first_previous_digest)
    {
        return Err("project configuration record does not follow the embedded source".into());
    }
    replace_configuration(manifest, final_update.source, final_update.source_digest);
    Ok(())
}

pub(super) fn configuration_digest(manifest: &ProjectManifest) -> Result<Option<[u8; 32]>, String> {
    let Some(file) = manifest
        .files
        .iter()
        .find(|file| is_root_configuration(file))
    else {
        return Ok(None);
    };
    let FilePayload::Utf8(source) = &file.payload else {
        return Err("embedded reraconfig.toml is not valid UTF-8".into());
    };
    Ok(Some(
        *blake3::hash(normalize_line_endings(source).as_bytes()).as_bytes(),
    ))
}

pub(super) fn encode_record(
    previous: Option<[u8; 32]>,
    contents: &str,
) -> Result<(Vec<u8>, [u8; 32]), String> {
    let source = normalize_line_endings(contents);
    ReraConfigDocument::parse(&source)
        .map_err(|error| format!("reraconfig.toml is invalid: {error}"))?;
    let source_length = u32::try_from(source.len())
        .map_err(|_| "reraconfig.toml is too large for a project update")?;
    let source_digest = *blake3::hash(source.as_bytes()).as_bytes();
    let record_length = FIXED_PREFIX_BYTES
        .checked_add(source.len())
        .and_then(|length| length.checked_add(FIXED_SUFFIX_BYTES))
        .ok_or("project configuration record length overflows")?;
    let mut record = Vec::with_capacity(record_length);
    record.extend_from_slice(RECORD_MAGIC);
    record.push(RECORD_VERSION);
    record.push(u8::from(previous.is_some()) * FLAG_PREVIOUS_SOURCE);
    record.extend_from_slice(&[0, 0]);
    record.extend_from_slice(&source_length.to_le_bytes());
    record.extend_from_slice(&previous.unwrap_or_default());
    record.extend_from_slice(&source_digest);
    record.extend_from_slice(source.as_bytes());
    record.extend_from_slice(blake3::hash(&record).as_bytes());
    record.extend_from_slice(
        &u32::try_from(record_length)
            .map_err(|_| "project configuration record is too large")?
            .to_le_bytes(),
    );
    record.extend_from_slice(FOOTER_MAGIC);
    Ok((record, source_digest))
}

pub(super) fn replace_configuration(
    manifest: &mut ProjectManifest,
    source: &str,
    source_digest: [u8; 32],
) {
    if let Some(file) = manifest
        .files
        .iter_mut()
        .find(|file| is_root_configuration(file))
    {
        file.category = FileCategory::Configuration;
        file.payload = FilePayload::Utf8(source.to_owned());
        file.content_hash = Some(ProtocolBytes::new(source_digest.to_vec()));
        return;
    }
    manifest.files.push(SubmittedFile {
        relative_path: "reraconfig.toml".into(),
        category: FileCategory::Configuration,
        payload: FilePayload::Utf8(source.to_owned()),
        content_hash: Some(ProtocolBytes::new(source_digest.to_vec())),
    });
}

fn is_root_configuration(file: &SubmittedFile) -> bool {
    file.category == FileCategory::Configuration
        && file
            .relative_path
            .replace('\\', "/")
            .eq_ignore_ascii_case("reraconfig.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_streams_many_records_and_keeps_the_last_complete_source() {
        let mut bytes = Vec::new();
        let mut previous = None;
        let mut final_source = String::new();
        for index in 0..2_048 {
            final_source = format!("[audio]\nvolume = {}\n", index % 101);
            let (record, digest) = encode_record(previous, &final_source).unwrap();
            bytes.extend_from_slice(&record);
            previous = Some(digest);
        }
        let complete_end = bytes.len();
        let (interrupted, _) = encode_record(previous, "[audio]\nvolume = 42\n").unwrap();
        bytes.extend_from_slice(&interrupted[..interrupted.len() / 2]);

        let journal = parse_journal(&bytes, 0).unwrap();
        assert_eq!(journal.valid_end, complete_end);
        let mut manifest = ProjectManifest {
            project_revision: 1,
            files: Vec::new(),
        };
        apply_journal(&mut manifest, &journal).unwrap();
        assert!(matches!(
            &manifest.files[0].payload,
            FilePayload::Utf8(source) if source == &final_source
        ));
    }

    #[test]
    fn journal_rejects_noncanonical_line_endings_and_broken_digest_chains() {
        let (mut noncanonical, _) = encode_record(None, "[audio]\nvolume = 42\n").unwrap();
        let source_end = noncanonical.len() - FIXED_SUFFIX_BYTES;
        let newline = noncanonical[FIXED_PREFIX_BYTES..source_end]
            .iter()
            .position(|byte| *byte == b'\n')
            .unwrap()
            + FIXED_PREFIX_BYTES;
        noncanonical[newline] = b'\r';
        resign_record(&mut noncanonical);
        assert!(
            parse_journal(&noncanonical, 0)
                .unwrap_err()
                .contains("normalized LF")
        );

        let (first, first_digest) = encode_record(None, "[audio]\nvolume = 42\n").unwrap();
        let (second, _) = encode_record(Some([9; 32]), "[audio]\nvolume = 80\n").unwrap();
        let mut broken = first;
        broken.extend_from_slice(&second);
        assert_ne!(first_digest, [9; 32]);
        assert!(
            parse_journal(&broken, 0)
                .unwrap_err()
                .contains("broken digest chain")
        );
    }

    fn resign_record(record: &mut [u8]) {
        let checksum_offset = record.len() - FIXED_SUFFIX_BYTES;
        let source_digest = blake3::hash(&record[FIXED_PREFIX_BYTES..checksum_offset]);
        record[48..80].copy_from_slice(source_digest.as_bytes());
        let checksum = blake3::hash(&record[..checksum_offset]);
        record[checksum_offset..checksum_offset + 32].copy_from_slice(checksum.as_bytes());
    }
}
