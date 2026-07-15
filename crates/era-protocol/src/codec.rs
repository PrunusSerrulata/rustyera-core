use minicbor::{Decode, Encode};

use crate::{ProtocolError, ProtocolErrorCode};

const MAXIMUM_CBOR_DEPTH: usize = 128;

/// Encode a protocol value using its fixed numeric field order.
///
/// # Errors
///
/// Returns a stable protocol error if the value cannot be encoded.
pub fn encode_canonical<T>(value: &T) -> Result<Vec<u8>, ProtocolError>
where
    T: Encode<()>,
{
    minicbor::to_vec(value).map_err(|error| {
        ProtocolError::new(
            ProtocolErrorCode::InvalidCbor,
            format!("failed to encode CBOR: {error}"),
        )
    })
}

/// Validate deterministic CBOR before decoding its known fields. Validation is
/// independent from the target Rust type so an older minor version may ignore a new,
/// canonically encoded map field without accepting alternate wire representations.
///
/// # Errors
///
/// Returns an error for malformed or non-deterministic input.
pub fn decode_canonical<'bytes, T>(bytes: &'bytes [u8]) -> Result<T, ProtocolError>
where
    T: Decode<'bytes, ()> + Encode<()>,
{
    validate_deterministic(bytes)?;
    minicbor::decode(bytes).map_err(|error| {
        ProtocolError::new(
            ProtocolErrorCode::InvalidCbor,
            format!("failed to decode CBOR: {error}"),
        )
    })
}

fn validate_deterministic(bytes: &[u8]) -> Result<(), ProtocolError> {
    let end = validate_item(bytes, 0, 0)?;
    if end != bytes.len() {
        return Err(invalid("trailing bytes after the CBOR data item"));
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn validate_item(bytes: &[u8], offset: usize, depth: usize) -> Result<usize, ProtocolError> {
    if depth > MAXIMUM_CBOR_DEPTH {
        return Err(invalid("CBOR nesting exceeds the protocol limit"));
    }
    let initial = *bytes
        .get(offset)
        .ok_or_else(|| invalid("truncated CBOR data item"))?;
    let major = initial >> 5;
    let additional = initial & 0x1f;
    let (argument, mut cursor) = decode_argument(bytes, offset + 1, additional)?;
    match major {
        0 | 1 => Ok(cursor),
        2 | 3 => {
            let length = usize::try_from(argument)
                .map_err(|_| invalid("CBOR string length exceeds this platform"))?;
            cursor = cursor
                .checked_add(length)
                .ok_or_else(|| invalid("CBOR string length overflow"))?;
            if cursor > bytes.len() {
                return Err(invalid("truncated CBOR string"));
            }
            if major == 3 {
                std::str::from_utf8(&bytes[cursor - length..cursor])
                    .map_err(|_| invalid("CBOR text is not UTF-8"))?;
            }
            Ok(cursor)
        }
        4 => {
            for _ in 0..argument {
                cursor = validate_item(bytes, cursor, depth + 1)?;
            }
            Ok(cursor)
        }
        5 => {
            let mut previous_key: Option<&[u8]> = None;
            for _ in 0..argument {
                let key_start = cursor;
                cursor = validate_item(bytes, cursor, depth + 1)?;
                let key = &bytes[key_start..cursor];
                if previous_key.is_some_and(|previous| previous >= key) {
                    return Err(non_canonical(
                        "CBOR map keys are duplicated or not in bytewise order",
                    ));
                }
                previous_key = Some(key);
                cursor = validate_item(bytes, cursor, depth + 1)?;
            }
            Ok(cursor)
        }
        6 => validate_item(bytes, cursor, depth + 1),
        7 => match additional {
            0..=23 => Ok(cursor),
            24 if argument >= 32 => Ok(cursor),
            24 => Err(non_canonical("CBOR simple value is not shortest form")),
            25..=27 => Err(non_canonical(
                "floating-point values are not part of the Era wire profile",
            )),
            _ => Err(invalid("invalid CBOR simple value")),
        },
        _ => Err(invalid("invalid CBOR major type")),
    }
}

fn decode_argument(
    bytes: &[u8],
    offset: usize,
    additional: u8,
) -> Result<(u64, usize), ProtocolError> {
    match additional {
        value @ 0..=23 => Ok((u64::from(value), offset)),
        24 => {
            let value = *bytes
                .get(offset)
                .ok_or_else(|| invalid("truncated CBOR argument"))?;
            if value < 24 {
                return Err(non_canonical("CBOR argument is not shortest form"));
            }
            Ok((u64::from(value), offset + 1))
        }
        25 => {
            let value = read_argument::<2, _>(bytes, offset, u16::from_be_bytes)?;
            if u8::try_from(value).is_ok() {
                return Err(non_canonical("CBOR argument is not shortest form"));
            }
            Ok((value, offset + 2))
        }
        26 => {
            let value = read_argument::<4, _>(bytes, offset, u32::from_be_bytes)?;
            if u16::try_from(value).is_ok() {
                return Err(non_canonical("CBOR argument is not shortest form"));
            }
            Ok((value, offset + 4))
        }
        27 => {
            let value = read_argument::<8, _>(bytes, offset, u64::from_be_bytes)?;
            if u32::try_from(value).is_ok() {
                return Err(non_canonical("CBOR argument is not shortest form"));
            }
            Ok((value, offset + 8))
        }
        31 => Err(non_canonical(
            "indefinite-length CBOR is not allowed by the wire profile",
        )),
        _ => Err(invalid("reserved CBOR additional information")),
    }
}

fn read_argument<const N: usize, T>(
    bytes: &[u8],
    offset: usize,
    convert: impl FnOnce([u8; N]) -> T,
) -> Result<u64, ProtocolError>
where
    T: Into<u64>,
{
    let end = offset
        .checked_add(N)
        .ok_or_else(|| invalid("CBOR argument offset overflow"))?;
    let value: [u8; N] = bytes
        .get(offset..end)
        .ok_or_else(|| invalid("truncated CBOR argument"))?
        .try_into()
        .map_err(|_| invalid("invalid CBOR argument width"))?;
    Ok(convert(value).into())
}

fn invalid(message: &str) -> ProtocolError {
    ProtocolError::new(ProtocolErrorCode::InvalidCbor, message)
}

fn non_canonical(message: &str) -> ProtocolError {
    ProtocolError::new(ProtocolErrorCode::NonCanonicalCbor, message)
}
