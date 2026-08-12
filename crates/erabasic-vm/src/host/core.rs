#[allow(clippy::wildcard_imports)]
use super::*;

pub(super) struct CoreNative {
    pub(super) name: String,
    pub(super) legacy_encoding: LegacyEncoding,
}

/// Evaluate a side-effect-free core native through the same implementation used by bytecode.
///
/// Debuggers use this deliberately narrow entry point instead of maintaining a second set of
/// `EraBasic` numeric and string semantics. Natives which require implicit variables, mutate
/// state, consume entropy, or cross the Host boundary are rejected here.
///
/// # Errors
///
/// Returns an error when the native is not in the pure whitelist, its arguments do not match the
/// reference signature, or its ordinary evaluation reports a domain or format error.
pub fn evaluate_pure_native(name: &str, arguments: Vec<VmValue>) -> Result<VmValue, String> {
    let name = name.to_ascii_lowercase();
    if !matches!(
        name.as_str(),
        "abs"
            | "sign"
            | "sqrt"
            | "cbrt"
            | "log"
            | "log10"
            | "exponent"
            | "power"
            | "getbit"
            | "bitcount"
            | "strlen"
            | "strlenu"
            | "strform"
            | "toint"
            | "isnumeric"
            | "unicode"
            | "convert"
            | "color_fromrgb"
            | "color_fromname"
            | "max"
            | "min"
            | "limit"
            | "inrange"
            | "tostr"
            | "substring"
            | "substringu"
            | "strfind"
            | "strfindu"
            | "strcount"
            | "strlens"
            | "strlensu"
            | "replace"
            | "escape"
            | "unicodetostr"
            | "encodetouni"
            | "unicodebyte"
            | "charatu"
            | "tolower"
            | "toupper"
    ) {
        return Err(format!("{name} is not a pure core-native service"));
    }
    let request = NativeCallRequest {
        import: RuntimeImport {
            key: SymbolKey::default(),
            namespace: "debug".into(),
            name: name.clone(),
            abi_version: 1,
            parameters: Vec::new(),
            result: None,
        },
        arguments,
        places: Vec::new(),
        implicit_places: BTreeMap::new(),
    };
    CoreNative {
        name,
        legacy_encoding: LegacyEncoding::default(),
    }
    .call(request)?
    .value
    .ok_or_else(|| "pure core-native service returned no value".into())
}

impl NativeService for CoreNative {
    fn implicit_place_names(&self) -> &'static [&'static str] {
        match self.name.as_str() {
            "getpalamlv" => &["PALAMLV"],
            "getexplv" => &["EXPLV"],
            _ => &[],
        }
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::too_many_lines
    )]
    fn call(&mut self, request: NativeCallRequest) -> Result<NativeReady, String> {
        let args = &request.arguments;
        let integer = |index: usize| match args.get(index) {
            Some(VmValue::Integer(value)) => Ok(*value),
            _ => Err(format!(
                "{} argument {} must be integer",
                self.name,
                index + 1
            )),
        };
        let string = |index: usize| match args.get(index) {
            Some(VmValue::String(value)) => Ok(value.as_str()),
            _ => Err(format!(
                "{} argument {} must be string",
                self.name,
                index + 1
            )),
        };
        let result = match self.name.as_str() {
            "abs" => VmValue::Integer(
                integer(0)?
                    .checked_abs()
                    .ok_or("ABS cannot accept the minimum signed integer")?,
            ),
            "sign" => VmValue::Integer(integer(0)?.signum()),
            "sqrt" => {
                let value = integer(0)?;
                if value < 0 {
                    return Err("SQRT argument 1 is negative".into());
                }
                VmValue::Integer((value as f64).sqrt() as i64)
            }
            "cbrt" => {
                let value = integer(0)?;
                if value < 0 {
                    return Err("CBRT argument 1 is negative".into());
                }
                VmValue::Integer((value as f64).powf(1.0 / 3.0) as i64)
            }
            "log" => checked_float_to_integer((integer(0)? as f64).ln(), "LOG")?,
            "log10" => checked_float_to_integer((integer(0)? as f64).log10(), "LOG10")?,
            "exponent" => checked_float_to_integer((integer(0)? as f64).exp(), "EXPONENT")?,
            "power" => {
                checked_float_to_integer((integer(0)? as f64).powf(integer(1)? as f64), "POWER")?
            }
            "getbit" => {
                let bit = u32::try_from(integer(1)?).map_err(|_| "GETBIT index is negative")?;
                VmValue::Integer(if bit < 64 {
                    (integer(0)? >> bit) & 1
                } else {
                    0
                })
            }
            "bitcount" => VmValue::Integer(i64::from(integer(0)?.count_ones())),
            "strlen" | "strlens" => VmValue::Integer(
                i64::try_from(self.legacy_encoding.encoded_len(string(0)?)).unwrap_or(i64::MAX),
            ),
            "strlenu" | "strlensu" => {
                // Emuera runs on .NET, so the U variants count UTF-16 code units rather than
                // Unicode scalar values. This remains observable for supplementary characters.
                VmValue::Integer(
                    i64::try_from(string(0)?.encode_utf16().count()).unwrap_or(i64::MAX),
                )
            }
            "strform" => {
                let value = string(0)?;
                if crate::interpreter::dynamic_form::requires_runtime_form_context(value) {
                    return Err("STRFORM template requires VM execution context".into());
                }
                VmValue::String(value.into())
            }
            "toint" => VmValue::Integer(parse_era_numeric(string(0)?, false)?.unwrap_or(0)),
            "isnumeric" => {
                VmValue::Integer(i64::from(parse_era_numeric(string(0)?, true)?.is_some()))
            }
            "convert" => {
                let value = integer(0)?;
                VmValue::String(match integer(1)? {
                    2 => format!("{:b}", value.cast_unsigned()),
                    8 => format!("{:o}", value.cast_unsigned()),
                    10 => value.to_string(),
                    16 => format!("{:x}", value.cast_unsigned()),
                    _ => return Err("CONVERT base must be 2, 8, 10, or 16".into()),
                })
            }
            "color_fromrgb" => {
                let channels = [integer(0)?, integer(1)?, integer(2)?];
                if channels.iter().any(|value| !(0..=255).contains(value)) {
                    return Err("COLOR_FROMRGB channels must be between 0 and 255".into());
                }
                VmValue::Integer((channels[0] << 16) | (channels[1] << 8) | channels[2])
            }
            "color_fromname" => {
                let name = string(0)?;
                if name.eq_ignore_ascii_case("transparent") {
                    return Err("COLOR_FROMNAME does not accept Transparent".into());
                }
                VmValue::Integer(erabasic_html::named_color(name).map_or(-1, i64::from))
            }
            "unicode" => {
                let value = u32::try_from(integer(0)?)
                    .map_err(|_| "UNICODE argument is outside the UTF-16 code-unit range")?;
                if value > u32::from(u16::MAX) {
                    return Err("UNICODE argument is outside the UTF-16 code-unit range".into());
                }
                // Rust strings cannot contain isolated UTF-16 surrogates.  BMP
                // scalar values otherwise have the same UTF-8 observable text.
                let scalar = char::from_u32(value)
                    .ok_or("UNICODE argument is an isolated UTF-16 surrogate")?;
                let control = (value < 0x1f && value != 0x0a && value != 0x0d)
                    || (0x7f..=0x9f).contains(&value);
                VmValue::String(if control {
                    String::new()
                } else {
                    scalar.to_string()
                })
            }
            "max" => VmValue::Integer(
                args.iter()
                    .enumerate()
                    .map(|(index, _)| integer(index))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .max()
                    .ok_or("MAX requires an argument")?,
            ),
            "min" => VmValue::Integer(
                args.iter()
                    .enumerate()
                    .map(|(index, _)| integer(index))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .min()
                    .ok_or("MIN requires an argument")?,
            ),
            "limit" => {
                let minimum = integer(1)?;
                let maximum = integer(2)?;
                if minimum > maximum {
                    return Err("LIMIT minimum exceeds maximum".into());
                }
                VmValue::Integer(integer(0)?.clamp(minimum, maximum))
            }
            "inrange" => VmValue::Integer(i64::from(
                (integer(1)?..=integer(2)?).contains(&integer(0)?),
            )),
            "tostr" => VmValue::String(integer(0)?.to_string()),
            "substring" => {
                let start = match args.get(1) {
                    None | Some(VmValue::Integer(i64::MIN)) => 0,
                    Some(_) => integer(1)?,
                };
                let length = match args.get(2) {
                    None | Some(VmValue::Integer(i64::MIN)) => None,
                    Some(_) => Some(integer(2)?),
                };
                VmValue::String(substring_legacy_bytes(
                    string(0)?,
                    start,
                    length,
                    self.legacy_encoding,
                ))
            }
            "substringu" => {
                let start = match args.get(1) {
                    None | Some(VmValue::Integer(i64::MIN)) => 0,
                    Some(_) => integer(1)?,
                };
                let length = match args.get(2) {
                    None | Some(VmValue::Integer(i64::MIN)) => None,
                    Some(_) => Some(integer(2)?),
                };
                VmValue::String(substring_scalars(string(0)?, start, length))
            }
            "strfind" => {
                let start = usize::try_from(integer(2).unwrap_or(0)).unwrap_or(usize::MAX);
                let haystack = string(0)?;
                let start = utf8_boundary_at_or_after(haystack, start).min(haystack.len());
                VmValue::Integer(
                    haystack[start..]
                        .find(string(1)?)
                        .and_then(|offset| i64::try_from(start + offset).ok())
                        .unwrap_or(-1),
                )
            }
            "strfindu" => {
                let haystack = string(0)?;
                let start = integer(2).unwrap_or(0);
                let Ok(start) = usize::try_from(start) else {
                    return Ok(NativeReady::value(VmValue::Integer(-1)));
                };
                let scalar_count = haystack.chars().count();
                if start >= scalar_count {
                    VmValue::Integer(-1)
                } else {
                    let byte_start = haystack
                        .char_indices()
                        .nth(start)
                        .map_or(haystack.len(), |(offset, _)| offset);
                    VmValue::Integer(
                        haystack[byte_start..]
                            .find(string(1)?)
                            .and_then(|offset| {
                                i64::try_from(haystack[..byte_start + offset].chars().count()).ok()
                            })
                            .unwrap_or(-1),
                    )
                }
            }
            "strcount" => {
                let regex = regex::Regex::new(string(1)?)
                    .map_err(|error| format!("STRCOUNT argument 2 is not a regex: {error}"))?;
                VmValue::Integer(
                    i64::try_from(regex.find_iter(string(0)?).count()).unwrap_or(i64::MAX),
                )
            }
            "getpalamlv" | "getexplv" => {
                let maximum = usize::try_from(integer(1)?)
                    .map_err(|_| format!("{} maximum level is negative", self.name))?;
                let variable = if self.name == "getpalamlv" {
                    "PALAMLV"
                } else {
                    "EXPLV"
                };
                let levels = request
                    .implicit_places
                    .get(variable)
                    .ok_or_else(|| format!("{variable} is not available"))?;
                let mut level = maximum;
                for index in 0..maximum {
                    let threshold = match levels.values.get(index + 1) {
                        Some(VmValue::Integer(value)) => *value,
                        _ => return Err(format!("{variable}[{}] is unavailable", index + 1)),
                    };
                    if integer(0)? < threshold {
                        level = index;
                        break;
                    }
                }
                VmValue::Integer(i64::try_from(level).unwrap_or(i64::MAX))
            }
            "replace" => VmValue::String(replace_text(&request)?),
            "escape" => VmValue::String(regex::escape(string(0)?)),
            "charatu" => {
                let position = usize::try_from(integer(1)?).unwrap_or(usize::MAX);
                VmValue::String(
                    string(0)?
                        .chars()
                        .nth(position)
                        .map_or_else(String::new, |value| value.to_string()),
                )
            }
            "encodetouni" => {
                let value = string(0)?;
                if value.is_empty() {
                    VmValue::Integer(-1)
                } else {
                    let position = usize::try_from(integer(1).unwrap_or(0))
                        .map_err(|_| "ENCODETOUNI position is negative")?;
                    let scalar = value
                        .chars()
                        .nth(position)
                        .ok_or("ENCODETOUNI position exceeds the string")?;
                    VmValue::Integer(i64::from(u32::from(scalar)))
                }
            }
            "unicodebyte" => VmValue::Integer(i64::from(u32::from(
                string(0)?
                    .chars()
                    .next()
                    .ok_or("UNICODEBYTE input is empty")?,
            ))),
            "tolower" => VmValue::String(string(0)?.to_lowercase()),
            "toupper" => VmValue::String(string(0)?.to_uppercase()),
            "unicodetostr" => {
                let scalar = u32::try_from(integer(0)?)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or("UNICODETOSTR argument is not a Unicode scalar")?;
                VmValue::String(scalar.to_string())
            }
            _ => return Err(format!("unknown core-native service {}", self.name)),
        };
        Ok(NativeReady::value(result))
    }
}

fn replace_text(request: &NativeCallRequest) -> Result<String, String> {
    let input = request_string(request, 0)?;
    let pattern = request_string(request, 1)?;
    let mode = match request.arguments.get(3) {
        None => 0,
        Some(VmValue::Integer(value)) => *value,
        Some(_) => return Err("REPLACE argument 4 must be integer".into()),
    };
    if mode == 2 {
        return Ok(input.replace(pattern, request_string(request, 2)?));
    }

    let regex = regex::Regex::new(pattern)
        .map_err(|error| format!("REPLACE argument 2 is not a regex: {error}"))?;
    if mode == 1 {
        let replacements = request
            .places
            .iter()
            .find(|place| place.argument_index == 2)
            .ok_or("REPLACE mode 1 argument 3 must be a string array")?
            .values
            .iter()
            .map(|value| match value {
                VmValue::String(value) => Ok(value.as_str()),
                _ => Err("REPLACE mode 1 argument 3 must be a string array".to_owned()),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut index = 0;
        return Ok(regex
            .replace_all(input, |_: &regex::Captures<'_>| {
                let replacement = replacements.get(index).copied().unwrap_or_default();
                index += 1;
                replacement
            })
            .into_owned());
    }

    Ok(regex
        .replace_all(input, request_string(request, 2)?)
        .into_owned())
}

fn request_string(request: &NativeCallRequest, index: usize) -> Result<&str, String> {
    match request.arguments.get(index) {
        Some(VmValue::String(value)) => Ok(value),
        Some(VmValue::StringPlace(_)) => request
            .places
            .iter()
            .find(|place| place.argument_index == index)
            .and_then(|place| place.values.first())
            .and_then(|value| match value {
                VmValue::String(value) => Some(value.as_str()),
                _ => None,
            })
            .ok_or_else(|| format!("REPLACE argument {} string place is unreadable", index + 1)),
        _ => Err(format!("REPLACE argument {} must be string", index + 1)),
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn checked_float_to_integer(value: f64, operation: &str) -> Result<VmValue, String> {
    if value.is_nan() {
        return Err(format!("{operation} result is NaN"));
    }
    if value.is_infinite() {
        return Err(format!("{operation} result is infinite"));
    }
    if value >= i64::MAX as f64 || value <= i64::MIN as f64 {
        return Err(format!("{operation} result is outside signed 64-bit range"));
    }
    Ok(VmValue::Integer(value as i64))
}

/// Parse the numeric grammar used by Emuera's TOINT and ISNUMERIC methods.
/// Unlike Rust's `FromStr`, the reference accepts binary/hex prefixes,
/// integer exponents, and a discarded decimal fraction, but never whitespace.
// The float conversion deliberately mirrors the reference's Math.Pow and
// unchecked Int64 cast instead of substituting an integer exponent algorithm.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::too_many_lines
)]
pub(super) fn parse_era_numeric(value: &str, numeric_check: bool) -> Result<Option<i64>, String> {
    if value.is_empty() || !value.is_ascii() {
        return Ok(None);
    }
    let bytes = value.as_bytes();
    if !bytes[0].is_ascii_digit() && !matches!(bytes[0], b'+' | b'-') {
        return Ok(None);
    }
    if matches!(bytes[0], b'+' | b'-') && bytes.get(1).is_none_or(|next| !next.is_ascii_digit()) {
        return Ok(None);
    }

    let mut index = 0;
    let mut radix = 10;
    if bytes.starts_with(b"0x") || bytes.starts_with(b"0X") {
        radix = 16;
        index = 2;
    } else if bytes.starts_with(b"0b") || bytes.starts_with(b"0B") {
        radix = 2;
        index = 2;
    }
    let digits_start = index;
    if radix == 10
        && bytes
            .get(index)
            .is_some_and(|byte| matches!(byte, b'+' | b'-'))
    {
        index += 1;
    }
    let unsigned_start = index;
    while let Some(byte) = bytes.get(index) {
        if byte.is_ascii_digit() {
            if radix == 2 && !matches!(byte, b'0' | b'1') {
                return Err("binary notation contains a digit other than 0 or 1".into());
            }
            index += 1;
        } else if radix == 16 && byte.is_ascii_hexdigit() {
            index += 1;
        } else {
            break;
        }
    }
    if index == unsigned_start {
        return Ok(None);
    }
    let significand = if radix == 10 {
        value[digits_start..index]
            .parse::<i64>()
            .map_err(|_| "numeric significand exceeds signed 64-bit range")?
    } else {
        let raw = u64::from_str_radix(&value[unsigned_start..index], radix)
            .map_err(|_| "numeric significand exceeds 64-bit range")?;
        raw.cast_signed()
    };

    let mut result = significand;
    if bytes
        .get(index)
        .is_some_and(|byte| matches!(byte, b'p' | b'P' | b'e' | b'E'))
    {
        let exponent_base = if matches!(bytes[index], b'p' | b'P') {
            2_f64
        } else {
            10_f64
        };
        index += 1;
        if numeric_check && bytes.get(index).is_none_or(|byte| !byte.is_ascii_digit()) {
            return Ok(None);
        }
        let exponent_start = index;
        if !numeric_check
            && bytes
                .get(index)
                .is_some_and(|byte| matches!(byte, b'+' | b'-'))
        {
            index += 1;
        }
        let exponent_digits = index;
        while let Some(byte) = bytes.get(index) {
            let valid = if radix == 16 {
                byte.is_ascii_hexdigit()
            } else {
                byte.is_ascii_digit()
            };
            if !valid {
                break;
            }
            if radix == 2 && !matches!(byte, b'0' | b'1') {
                return Err("binary exponent contains a digit other than 0 or 1".into());
            }
            index += 1;
        }
        if index == exponent_digits {
            return if numeric_check {
                Ok(None)
            } else {
                Err("numeric exponent has no digits".into())
            };
        }
        let exponent = i32::from_str_radix(&value[exponent_start..index], radix)
            .map_err(|_| "numeric exponent exceeds supported range")?;
        let expanded = (significand as f64) * exponent_base.powi(exponent);
        if !expanded.is_finite() || expanded > i64::MAX as f64 || expanded < i64::MIN as f64 {
            return Err("numeric value exceeds signed 64-bit range".into());
        }
        result = expanded as i64;
    }

    if bytes.get(index) == Some(&b'.') {
        index += 1;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
    }
    Ok((index == bytes.len()).then_some(result))
}

pub(super) fn substring_legacy_bytes(
    value: &str,
    start: i64,
    length: Option<i64>,
    encoding: LegacyEncoding,
) -> String {
    let total = encoding.encoded_len(value);
    let start = usize::try_from(start.max(0)).unwrap_or(usize::MAX);
    if start >= total || length == Some(0) {
        return String::new();
    }
    let requested = length
        .and_then(|length| usize::try_from(length).ok())
        .filter(|length| *length <= total)
        .unwrap_or(total);

    let mut characters = value.char_indices();
    let byte_start = if start == 0 {
        0
    } else {
        let mut consumed: usize = 0;
        loop {
            let Some((index, character)) = characters.next() else {
                return String::new();
            };
            consumed = consumed.saturating_add(encoding.encoded_char_len(character));
            if consumed >= start {
                break index + character.len_utf8();
            }
        }
    };
    let mut consumed: usize = 0;
    let byte_length = value[byte_start..]
        .char_indices()
        .find_map(|(index, character)| {
            consumed = consumed.saturating_add(encoding.encoded_char_len(character));
            (consumed >= requested).then_some(index + character.len_utf8())
        })
        .unwrap_or(value.len() - byte_start);
    value[byte_start..byte_start + byte_length].into()
}

pub(super) fn substring_scalars(value: &str, start: i64, length: Option<i64>) -> String {
    let start = if start <= 0 {
        0
    } else {
        usize::try_from(start).unwrap_or(usize::MAX)
    };
    let length = length.and_then(|length| usize::try_from(length).ok());
    value
        .chars()
        .skip(start)
        .take(length.unwrap_or(usize::MAX))
        .collect()
}

fn utf8_boundary_at_or_after(value: &str, mut offset: usize) -> usize {
    while offset < value.len() && !value.is_char_boundary(offset) {
        offset += 1;
    }
    offset
}
