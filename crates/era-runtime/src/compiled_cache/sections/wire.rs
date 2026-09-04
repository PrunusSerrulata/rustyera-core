pub(super) fn write_bytes(writer: &mut dyn std::io::Write, bytes: &[u8]) -> Result<(), String> {
    write_varint(
        writer,
        u64::try_from(bytes.len()).map_err(|_| "compiled cache byte string is too large")?,
    )?;
    writer.write_all(bytes).map_err(|error| error.to_string())
}

pub(super) fn read_bytes(reader: &mut dyn std::io::Read, maximum: u64) -> Result<Vec<u8>, String> {
    let length = usize::try_from(read_stream_varint(reader)?)
        .map_err(|_| "compiled cache byte string is not addressable")?;
    if u64::try_from(length).unwrap_or(u64::MAX) > maximum {
        return Err("compiled cache byte string exceeds its section".into());
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| "compiled cache byte string allocation failed")?;
    bytes.resize(length, 0);
    reader
        .read_exact(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

pub(super) fn read_count(
    reader: &mut dyn std::io::Read,
    maximum: u64,
    name: &str,
) -> Result<usize, String> {
    let count = usize::try_from(read_stream_varint(reader)?)
        .map_err(|_| format!("compiled cache {name} count is not addressable"))?;
    if u64::try_from(count).unwrap_or(u64::MAX) > maximum {
        return Err(format!("compiled cache {name} count is invalid"));
    }
    Ok(count)
}

pub(super) fn expect_magic(
    reader: &mut dyn std::io::Read,
    expected: [u8; 4],
    name: &str,
) -> Result<(), String> {
    let mut actual = [0_u8; 4];
    reader
        .read_exact(&mut actual)
        .map_err(|error| error.to_string())?;
    if actual != expected {
        return Err(format!(
            "compiled cache {name} section has an invalid header"
        ));
    }
    Ok(())
}

pub(super) fn decode_file_category(value: u8) -> Result<FileCategory, String> {
    match value {
        0 => Ok(FileCategory::Csv),
        1 => Ok(FileCategory::Erh),
        2 => Ok(FileCategory::Erb),
        3 => Ok(FileCategory::ResourceManifest),
        4 => Ok(FileCategory::Resource),
        5 => Ok(FileCategory::Configuration),
        6 => Ok(FileCategory::Als),
        7 => Ok(FileCategory::Erd),
        _ => Err("project manifest file category is invalid".into()),
    }
}

pub(super) fn encode_digest_section(
    digests: &[Digest],
    kind: ProjectContainerKind,
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    encode_raw_section(kind.compression_level(), cancelled, |writer| {
        writer
            .write_all(DIGEST_SECTION_MAGIC)
            .map_err(|error| error.to_string())?;
        writer
            .write_all(
                &u64::try_from(digests.len())
                    .map_err(|_| "compiled cache digest count is too large")?
                    .to_le_bytes(),
            )
            .map_err(|error| error.to_string())?;
        for digest in digests {
            if digest.0[16..].iter().any(|byte| *byte != 0) {
                return Err(
                    "statement fingerprint exceeds the project-file 128-bit identity".into(),
                );
            }
            writer
                .write_all(&digest.0[..16])
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    })
}

pub(super) fn decode_digest_section(
    section: &EncodedSectionRef<'_>,
) -> Result<Vec<Digest>, String> {
    let decoded_length = usize::try_from(section.decoded_length)
        .map_err(|_| "compiled cache digest section is not addressable")?;
    let decoded = zstd::bulk::decompress(section.compressed, decoded_length)
        .map_err(|error| error.to_string())?;
    if decoded.len() != decoded_length {
        return Err("compiled cache decoded digest section length differs".into());
    }
    if decoded.get(..DIGEST_SECTION_MAGIC.len()) != Some(DIGEST_SECTION_MAGIC) {
        return Err("compiled cache digest section has an invalid header".into());
    }
    let mut cursor = DIGEST_SECTION_MAGIC.len();
    let count = usize::try_from(read_u64(&decoded, &mut cursor)?)
        .map_err(|_| "compiled cache digest count is not addressable")?;
    let expected_length = cursor
        .checked_add(
            count
                .checked_mul(16)
                .ok_or("compiled cache digest section length overflow")?,
        )
        .ok_or("compiled cache digest section length overflow")?;
    if expected_length != decoded.len() {
        return Err("compiled cache digest section length is invalid".into());
    }
    let mut digests = Vec::new();
    digests
        .try_reserve_exact(count)
        .map_err(|_| "compiled cache digest allocation failed")?;
    while cursor < decoded.len() {
        let end = cursor + 16;
        let mut digest = [0_u8; 32];
        digest[..16].copy_from_slice(&decoded[cursor..end]);
        digests.push(Digest(digest));
        cursor = end;
    }
    Ok(digests)
}

