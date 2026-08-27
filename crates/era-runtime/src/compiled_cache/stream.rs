use super::{
    DecodedProjectFile, EncodedSectionRef, FIXED_SECTION_COUNT, LEGACY_PROJECT_VERSION,
    MANIFEST_SECTION_INDEX, MAXIMUM_DECODED_PAYLOAD_BYTES, PREVIOUS_PROJECT_VERSION,
    PROFILELESS_PROJECT_VERSION, PROJECT_MAGIC, ProjectFileError, ProjectSourceIdentity,
    ProtocolBytes, StreamingConfigurationJournal, TARGET_PARALLEL_SECTIONS, VERSION, apply_journal,
    decode_manifest_section, project_identity, read_u32, read_u64,
};

pub(super) const HEADER_BYTES: usize = PROJECT_MAGIC.len() + 1 + 8 + 32 + 32 + 4 + 4;
const SECTION_HEADER_BYTES: usize = 16;
const CONTAINER_DIGEST_BYTES: usize = 32;

/// A source manifest decoded from a streamed portable project file.
#[derive(Debug, Eq, PartialEq)]
pub struct DecodedProjectFileStream {
    /// BLAKE3 digest of every byte in the original project file.
    pub file_digest: [u8; 32],
    /// Validated project identity and full source manifest.
    pub project: DecodedProjectFile,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StreamPhase {
    Header,
    SectionHeader,
    SectionBody,
    Digest,
    Journal,
}

/// Incrementally extracts the source manifest from a portable project file.
///
/// Compiled artifact sections are hashed and skipped instead of being retained. This keeps the
/// decoder's resident input proportional to the compressed source manifest and configuration
/// journal rather than to the complete project container.
pub struct ProjectFileStreamDecoder {
    expected_len: usize,
    received_len: usize,
    file_hasher: blake3::Hasher,
    container_hasher: blake3::Hasher,
    container_bytes: usize,
    phase: StreamPhase,
    header: Vec<u8>,
    identity: Option<ProjectSourceIdentity>,
    section_header: [u8; SECTION_HEADER_BYTES],
    section_header_len: usize,
    section_index: usize,
    section_count: usize,
    section_remaining: usize,
    decoded_bytes: u64,
    manifest_decoded_len: Option<u64>,
    manifest_compressed: Vec<u8>,
    digest: [u8; CONTAINER_DIGEST_BYTES],
    digest_len: usize,
    journal: StreamingConfigurationJournal,
}

impl ProjectFileStreamDecoder {
    /// Create one bounded streaming decoder for an exact project-file length.
    ///
    /// # Errors
    ///
    /// Returns an error when the declared length exceeds the negotiated transfer limit or cannot
    /// contain the mandatory project-file framing.
    pub fn new(expected_len: usize, maximum_len: usize) -> Result<Self, ProjectFileError> {
        let minimum_len = HEADER_BYTES
            .saturating_add(FIXED_SECTION_COUNT.saturating_mul(SECTION_HEADER_BYTES))
            .saturating_add(CONTAINER_DIGEST_BYTES);
        if expected_len > maximum_len {
            return Err(error("project file exceeds the negotiated transfer limit"));
        }
        if expected_len < minimum_len {
            return Err(error("project file has an invalid header"));
        }
        Ok(Self {
            expected_len,
            received_len: 0,
            file_hasher: blake3::Hasher::new(),
            container_hasher: blake3::Hasher::new(),
            container_bytes: 0,
            phase: StreamPhase::Header,
            header: Vec::with_capacity(HEADER_BYTES),
            identity: None,
            section_header: [0; SECTION_HEADER_BYTES],
            section_header_len: 0,
            section_index: 0,
            section_count: 0,
            section_remaining: 0,
            decoded_bytes: 0,
            manifest_decoded_len: None,
            manifest_compressed: Vec::new(),
            digest: [0; CONTAINER_DIGEST_BYTES],
            digest_len: 0,
            journal: StreamingConfigurationJournal::default(),
        })
    }

    /// Consume the next contiguous bytes from the project file.
    ///
    /// # Errors
    ///
    /// Returns an error for an oversized transfer, malformed framing, unsupported versions, or a
    /// container digest mismatch.
    pub fn append(&mut self, bytes: &[u8]) -> Result<(), ProjectFileError> {
        let end = self
            .received_len
            .checked_add(bytes.len())
            .ok_or_else(|| error("project file size overflow"))?;
        if end > self.expected_len {
            return Err(error("project file upload exceeds its declared size"));
        }
        self.received_len = end;
        self.file_hasher.update(bytes);

        let mut cursor = 0;
        while cursor < bytes.len() {
            match self.phase {
                StreamPhase::Header => self.append_header(bytes, &mut cursor)?,
                StreamPhase::SectionHeader => self.append_section_header(bytes, &mut cursor)?,
                StreamPhase::SectionBody => self.append_section_body(bytes, &mut cursor),
                StreamPhase::Digest => self.append_digest(bytes, &mut cursor)?,
                StreamPhase::Journal => {
                    self.journal
                        .append(&bytes[cursor..])
                        .map_err(ProjectFileError::from)?;
                    cursor = bytes.len();
                }
            }
        }
        Ok(())
    }

    /// Finish validation and decode the retained source manifest.
    ///
    /// # Errors
    ///
    /// Returns an error when the upload is incomplete or its manifest, identity, or configuration
    /// journal is invalid.
    pub fn finish(self) -> Result<DecodedProjectFileStream, ProjectFileError> {
        if self.received_len != self.expected_len || self.phase != StreamPhase::Journal {
            return Err(error(&format!(
                "project file upload is incomplete: received {} of {} bytes",
                self.received_len, self.expected_len
            )));
        }
        let decoded_length = self
            .manifest_decoded_len
            .ok_or_else(|| error("project file manifest section is missing"))?;
        let section = EncodedSectionRef {
            decoded_length,
            compressed: &self.manifest_compressed,
        };
        let identity = self
            .identity
            .ok_or_else(|| error("project file identity is missing"))?;
        let mut manifest = decode_manifest_section(
            &section,
            identity.project_revision,
            self.header[PROJECT_MAGIC.len()],
        )
        .map_err(ProjectFileError::from)?;
        if !identity.matches(&manifest) {
            return Err(error(
                "project file identity does not match its embedded manifest",
            ));
        }
        let journal = self.journal.finish(0).map_err(ProjectFileError::from)?;
        apply_journal(&mut manifest, &journal).map_err(ProjectFileError::from)?;
        Ok(DecodedProjectFileStream {
            file_digest: *self.file_hasher.finalize().as_bytes(),
            project: DecodedProjectFile {
                identity: project_identity(&manifest),
                manifest,
            },
        })
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        self.header.capacity() + self.manifest_compressed.capacity() + self.journal.retained_bytes()
    }

    fn append_header(&mut self, bytes: &[u8], cursor: &mut usize) -> Result<(), ProjectFileError> {
        let length = (HEADER_BYTES - self.header.len()).min(bytes.len() - *cursor);
        let input = &bytes[*cursor..*cursor + length];
        self.header.extend_from_slice(input);
        self.container_hasher.update(input);
        self.container_bytes += input.len();
        *cursor += length;
        if self.header.len() != HEADER_BYTES {
            return Ok(());
        }
        self.parse_header()?;
        self.phase = StreamPhase::SectionHeader;
        Ok(())
    }

    fn parse_header(&mut self) -> Result<(), ProjectFileError> {
        if self.header.get(..PROJECT_MAGIC.len()) != Some(PROJECT_MAGIC) {
            return Err(error("project file has an invalid header"));
        }
        let version = self.header[PROJECT_MAGIC.len()];
        if !matches!(
            version,
            LEGACY_PROJECT_VERSION
                | PREVIOUS_PROJECT_VERSION
                | PROFILELESS_PROJECT_VERSION
                | VERSION
        ) {
            return Err(error(&format!(
                "unsupported project file version {version:02x}"
            )));
        }
        let mut cursor = PROJECT_MAGIC.len() + 1;
        let project_revision =
            read_u64(&self.header, &mut cursor).map_err(ProjectFileError::from)?;
        let source_digest = self.header[cursor..cursor + 32].to_vec();
        cursor += 32;
        cursor += 32; // The compiled-cache key is not needed for a source-only load.
        let function_sections =
            usize::try_from(read_u32(&self.header, &mut cursor).map_err(ProjectFileError::from)?)
                .map_err(|_| error("compiled cache function section count is not addressable"))?;
        let source_sections =
            usize::try_from(read_u32(&self.header, &mut cursor).map_err(ProjectFileError::from)?)
                .map_err(|_| error("compiled cache source section count is not addressable"))?;
        if function_sections > TARGET_PARALLEL_SECTIONS.saturating_mul(2)
            || source_sections > TARGET_PARALLEL_SECTIONS
        {
            return Err(error(
                "compiled project cache has too many parallel sections",
            ));
        }
        self.section_count = FIXED_SECTION_COUNT
            .checked_add(function_sections)
            .and_then(|count| count.checked_add(source_sections))
            .ok_or_else(|| error("compiled project cache section count overflows"))?;
        self.identity = Some(ProjectSourceIdentity {
            project_revision,
            source_digest: ProtocolBytes::new(source_digest),
        });
        Ok(())
    }

    fn append_section_header(
        &mut self,
        bytes: &[u8],
        cursor: &mut usize,
    ) -> Result<(), ProjectFileError> {
        let length = (SECTION_HEADER_BYTES - self.section_header_len).min(bytes.len() - *cursor);
        let input = &bytes[*cursor..*cursor + length];
        self.section_header[self.section_header_len..self.section_header_len + length]
            .copy_from_slice(input);
        self.section_header_len += length;
        self.container_hasher.update(input);
        self.container_bytes += input.len();
        *cursor += length;
        if self.section_header_len != SECTION_HEADER_BYTES {
            return Ok(());
        }
        let decoded_length = u64::from_le_bytes(
            self.section_header[..8]
                .try_into()
                .expect("eight-byte decoded length"),
        );
        let compressed_length = usize::try_from(u64::from_le_bytes(
            self.section_header[8..]
                .try_into()
                .expect("eight-byte compressed length"),
        ))
        .map_err(|_| error("compiled cache section is not addressable"))?;
        self.decoded_bytes = self
            .decoded_bytes
            .checked_add(decoded_length)
            .ok_or_else(|| error("compiled cache decoded length overflow"))?;
        if self.decoded_bytes > MAXIMUM_DECODED_PAYLOAD_BYTES {
            return Err(error("compiled cache decoded sections exceed their limit"));
        }
        let remaining_section_headers = self
            .section_count
            .saturating_sub(self.section_index.saturating_add(1))
            .checked_mul(SECTION_HEADER_BYTES)
            .ok_or_else(|| error("compiled project cache section framing overflows"))?;
        let required_tail = compressed_length
            .checked_add(remaining_section_headers)
            .and_then(|length| length.checked_add(CONTAINER_DIGEST_BYTES))
            .ok_or_else(|| error("compiled cache section length overflow"))?;
        if self
            .container_bytes
            .checked_add(required_tail)
            .is_none_or(|length| length > self.expected_len)
        {
            return Err(error("compiled cache section is truncated"));
        }
        if self.section_index == MANIFEST_SECTION_INDEX {
            self.manifest_decoded_len = Some(decoded_length);
            self.manifest_compressed
                .try_reserve_exact(compressed_length)
                .map_err(|reserve_error| {
                    error(&format!(
                        "failed to reserve project manifest buffer: {reserve_error}"
                    ))
                })?;
        }
        self.section_remaining = compressed_length;
        self.section_header_len = 0;
        if compressed_length == 0 {
            self.finish_section();
        } else {
            self.phase = StreamPhase::SectionBody;
        }
        Ok(())
    }

    fn append_section_body(&mut self, bytes: &[u8], cursor: &mut usize) {
        let length = self.section_remaining.min(bytes.len() - *cursor);
        let input = &bytes[*cursor..*cursor + length];
        self.container_hasher.update(input);
        self.container_bytes += input.len();
        if self.section_index == MANIFEST_SECTION_INDEX {
            self.manifest_compressed.extend_from_slice(input);
        }
        self.section_remaining -= length;
        *cursor += length;
        if self.section_remaining == 0 {
            self.finish_section();
        }
    }

    fn finish_section(&mut self) {
        self.section_index += 1;
        self.phase = if self.section_index == self.section_count {
            StreamPhase::Digest
        } else {
            StreamPhase::SectionHeader
        };
    }

    fn append_digest(&mut self, bytes: &[u8], cursor: &mut usize) -> Result<(), ProjectFileError> {
        let length = (CONTAINER_DIGEST_BYTES - self.digest_len).min(bytes.len() - *cursor);
        self.digest[self.digest_len..self.digest_len + length]
            .copy_from_slice(&bytes[*cursor..*cursor + length]);
        self.digest_len += length;
        *cursor += length;
        if self.digest_len == CONTAINER_DIGEST_BYTES {
            if self.container_hasher.finalize().as_bytes() != &self.digest {
                return Err(error("compiled project cache digest mismatch"));
            }
            self.phase = StreamPhase::Journal;
        }
        Ok(())
    }
}

fn error(message: &str) -> ProjectFileError {
    ProjectFileError::from(message.to_owned())
}
