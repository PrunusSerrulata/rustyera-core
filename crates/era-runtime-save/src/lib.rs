//! I/O-free codecs for the save formats written by the pinned Emuera 1808 runtime.
//!
//! Callers provide and receive byte slices. Path selection, atomic replacement and
//! all filesystem errors remain frontend responsibilities.

mod binary;
mod model;
mod text;

pub use binary::{decode_binary, encode_binary};
pub use model::{
    OpaqueSaveExtension, SaveCodecError, SaveCodecLimits, SaveDocument, SaveEntry, SaveFileKind,
    SaveFormat, SaveMetadata, SaveValue,
};
pub use text::{decode_text, encode_text};

/// Detect and decode one current-format save without performing I/O.
///
/// # Errors
///
/// Returns an error for an invalid header, unsupported format, malformed data, or limit breach.
pub fn decode(data: &[u8], limits: SaveCodecLimits) -> Result<SaveDocument, SaveCodecError> {
    if binary::is_binary(data) {
        decode_binary(data, limits)
    } else {
        decode_text(data, limits)
    }
}

/// Encode a document in the selected current format.
///
/// # Errors
///
/// Returns an error when the document cannot be represented or exceeds a configured limit.
pub fn encode(
    document: &SaveDocument,
    format: SaveFormat,
    limits: SaveCodecLimits,
) -> Result<Vec<u8>, SaveCodecError> {
    match format {
        SaveFormat::Text1808 => encode_text(document, limits),
        SaveFormat::Binary1808 | SaveFormat::Binary1808Gzip => {
            encode_binary(document, format, limits)
        }
    }
}
