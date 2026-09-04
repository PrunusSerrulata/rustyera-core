#[allow(clippy::wildcard_imports)]
use super::*;

pub(super) struct CoreNative {
    pub(super) name: String,
    pub(super) legacy_encoding: LegacyEncoding,
    regex_cache: RegexCache,
    numeric_read_fallback: bool,
}

const REGEX_CACHE_CAPACITY: usize = 16;

// Emuera's RegexFactory reuses compiled patterns in regex string natives. Keep the same derived
// optimization bounded to one CoreNative within a RuntimeVm; it has no script-visible state and
// is intentionally rebuilt rather than persisted in VM snapshots.
#[derive(Default)]
struct RegexCache {
    entries: Vec<(String, CachedRegex)>,
}

enum CachedRegex {
    Standard(regex::Regex),
    LeadingPositiveTail(Vec<regex::Regex>),
}

impl RegexCache {
    fn get_or_compile(&mut self, pattern: &str) -> Result<&CachedRegex, regex::Error> {
        if let Some(index) = self
            .entries
            .iter()
            .position(|(cached, _)| cached == pattern)
        {
            if index + 1 != self.entries.len() {
                let entry = self.entries.remove(index);
                self.entries.push(entry);
            }
            let newest = self.entries.len() - 1;
            return Ok(&self.entries[newest].1);
        }

        let regex = compile_core_regex(pattern)?;
        if self.entries.len() == REGEX_CACHE_CAPACITY {
            self.entries.remove(0);
        }
        let index = self.entries.len();
        self.entries.push((pattern.to_owned(), regex));
        Ok(&self.entries[index].1)
    }

    fn get_standard(&mut self, pattern: &str) -> Result<&regex::Regex, regex::Error> {
        match self.get_or_compile(pattern)? {
            CachedRegex::Standard(regex) => Ok(regex),
            CachedRegex::LeadingPositiveTail(_) => {
                Err(regex::Regex::new(pattern)
                    .expect_err("look-ahead is unsupported by regex crate"))
            }
        }
    }

    fn count_matches(&mut self, pattern: &str, input: &str) -> Result<usize, regex::Error> {
        Ok(match self.get_or_compile(pattern)? {
            CachedRegex::Standard(regex) => regex.find_iter(input).count(),
            CachedRegex::LeadingPositiveTail(assertions) => {
                usize::from(assertions.iter().any(|assertion| assertion.is_match(input)))
            }
        })
    }
}

fn compile_core_regex(pattern: &str) -> Result<CachedRegex, regex::Error> {
    if let Some(assertions) = leading_positive_tail_assertions(pattern) {
        return assertions.map(CachedRegex::LeadingPositiveTail);
    }
    regex::Regex::new(pattern).map(CachedRegex::Standard)
}

/// Compile the bounded look-ahead shape used by Snake TW's name predicate.
///
/// Each alternative asserts a condition and then consumes through the end of the
/// input. Its observable result is therefore either one match or no match, which
/// can be evaluated with Rust's linear regex engine without general backtracking.
fn leading_positive_tail_assertions(
    pattern: &str,
) -> Option<Result<Vec<regex::Regex>, regex::Error>> {
    let (case_insensitive, pattern) = pattern
        .strip_prefix("(?i)")
        .map_or((false, pattern), |pattern| (true, pattern));
    let assertions = pattern
        .strip_prefix("(?=")?
        .strip_suffix(").*$")?
        .split(").*$|(?=");
    let mut compiled_assertions = Vec::new();
    for assertion in assertions {
        let compiled = regex::RegexBuilder::new(assertion)
            .case_insensitive(case_insensitive)
            .build();
        match compiled {
            Ok(compiled) => compiled_assertions.push(compiled),
            Err(error) => return Some(Err(error)),
        }
    }
    (!compiled_assertions.is_empty()).then_some(Ok(compiled_assertions))
}

impl CoreNative {
    pub(super) fn new(name: String, legacy_encoding: LegacyEncoding) -> Self {
        Self {
            name,
            legacy_encoding,
            regex_cache: RegexCache::default(),
            numeric_read_fallback: false,
        }
    }

    pub(super) fn with_compatibility(
        mut self,
        compatibility: &erabasic_compat::CompatibilityIdentity,
    ) -> Self {
        self.numeric_read_fallback = compatibility.uses_snake_numeric_read_fallback();
        self
    }
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
    evaluate_pure_native_with_compatibility(
        name,
        arguments,
        &erabasic_compat::CompatibilityIdentity::reference(),
    )
}

/// Evaluate a pure native using the active project's validated compatibility policies.
///
/// # Errors
///
/// Returns an error for an unsupported identity, unavailable native, invalid arguments, or
/// a domain error that the selected policy does not turn into an ordinary return value.
pub fn evaluate_pure_native_with_compatibility(
    name: &str,
    arguments: Vec<VmValue>,
    compatibility: &erabasic_compat::CompatibilityIdentity,
) -> Result<VmValue, String> {
    compatibility
        .validate()
        .map_err(|error| error.to_string())?;
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
            | "unchecked_add"
            | "unchecked_sub"
            | "unchecked_mul"
            | "unchecked_neg"
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
        service_key: SymbolKey::default(),
        omitted_arguments: Vec::new(),
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
    CoreNative::new(name, LegacyEncoding::default())
        .with_compatibility(compatibility)
        .call(request)
        .map_err(|failure| failure.message)?
        .value
        .ok_or_else(|| "pure core-native service returned no value".into())
}

fn replace_text(
    request: &NativeCallRequest,
    regex_cache: &mut RegexCache,
) -> Result<String, ExecutionFailure> {
    let input = request_string(request, 0)?;
    let pattern = request_string(request, 1)?;
    let mode = match request.argument(3) {
        None => 0,
        Some(VmValue::Integer(value)) => *value,
        Some(_) => {
            return Err(native_contract_failure(
                "REPLACE argument 4 must be integer",
            ));
        }
    };
    if mode == 2 {
        return Ok(input.replace(pattern, request_string(request, 2)?));
    }

    let regex = regex_cache
        .get_standard(pattern)
        .map_err(|error| regex_failure("REPLACE", &error))?;
    if mode == 1 {
        match request.argument(2) {
            Some(VmValue::StringPlace(_)) => {}
            Some(_) => {
                return Err(native_script_failure(
                    ScriptFaultKind::Argument,
                    "REPLACE mode 1 argument 3 must be a string array",
                ));
            }
            None => return Err(native_contract_failure("REPLACE argument 3 is missing")),
        }
        let replacements = request
            .places
            .iter()
            .find(|place| place.argument_index == 2)
            .ok_or_else(|| {
                native_contract_failure("REPLACE mode 1 argument 3 must be a string array")
            })?
            .values
            .iter()
            .map(|value| match value {
                VmValue::String(value) => Ok(value.as_str()),
                _ => Err(native_contract_failure(
                    "REPLACE mode 1 argument 3 must be a string array",
                )),
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

fn request_string(request: &NativeCallRequest, index: usize) -> Result<&str, ExecutionFailure> {
    match request.argument(index) {
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
            .ok_or_else(|| {
                native_contract_failure(format!(
                    "REPLACE argument {} string place is unreadable",
                    index + 1
                ))
            }),
        _ => Err(native_contract_failure(format!(
            "REPLACE argument {} must be string",
            index + 1
        ))),
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn checked_float_to_integer(value: f64, operation: &str) -> Result<VmValue, ExecutionFailure> {
    if value.is_nan() {
        return Err(native_script_failure(
            ScriptFaultKind::Arithmetic,
            format!("{operation} result is NaN"),
        ));
    }
    if value.is_infinite() {
        return Err(native_script_failure(
            ScriptFaultKind::Arithmetic,
            format!("{operation} result is infinite"),
        ));
    }
    if value >= i64::MAX as f64 || value <= i64::MIN as f64 {
        return Err(native_script_failure(
            ScriptFaultKind::Arithmetic,
            format!("{operation} result is outside signed 64-bit range"),
        ));
    }
    Ok(VmValue::Integer(value as i64))
}

/// Failure confined to the reference integer reader, before TOINT's trailing-fraction check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct NumericReadError(&'static str);

impl From<&'static str> for NumericReadError {
    fn from(message: &'static str) -> Self {
        Self(message)
    }
}

impl From<NumericReadError> for String {
    fn from(error: NumericReadError) -> Self {
        error.0.into()
    }
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
pub(super) fn parse_era_numeric(
    value: &str,
    numeric_check: bool,
) -> Result<Option<i64>, NumericReadError> {
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

pub(super) fn regex_failure(operation: &str, error: &regex::Error) -> ExecutionFailure {
    let message = format!("{operation} argument 2 is not a regex: {error}");
    match error {
        regex::Error::Syntax(_) => native_script_failure(ScriptFaultKind::Parse, message),
        regex::Error::CompiledTooBig(_) => native_resource_failure(message),
        // Future regex error variants must not accidentally become script-catchable.
        _ => native_contract_failure(message),
    }
}

impl From<NumericReadError> for ExecutionFailure {
    fn from(error: NumericReadError) -> Self {
        native_script_failure(ScriptFaultKind::Parse, error.0)
    }
}

#[cfg(test)]
mod regex_cache_tests {
    use super::*;

    #[test]
    fn compiled_patterns_are_reused_without_unbounded_or_invalid_entries() {
        let mut cache = RegexCache::default();

        cache.get_or_compile("reused").unwrap();
        cache.get_or_compile("reused").unwrap();
        assert_eq!(cache.entries.len(), 1);

        assert!(cache.get_or_compile("[").is_err());
        assert_eq!(cache.entries.len(), 1);

        for index in 0..=REGEX_CACHE_CAPACITY {
            cache.get_or_compile(&format!("pattern-{index}")).unwrap();
            assert!(cache.entries.len() <= REGEX_CACHE_CAPACITY);
        }
        assert_eq!(cache.entries.len(), REGEX_CACHE_CAPACITY);
    }
}

mod dispatch;
