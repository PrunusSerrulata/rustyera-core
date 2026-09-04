#[allow(clippy::wildcard_imports)]
use super::*;
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
    fn call(&mut self, request: NativeCallRequest) -> Result<NativeReady, ExecutionFailure> {
        let args = &request.arguments;
        let integer = |index: usize| match request.argument(index) {
            Some(VmValue::Integer(value)) => Ok(*value),
            _ => Err(native_contract_failure(format!(
                "{} argument {} must be integer",
                self.name,
                index + 1
            ))),
        };
        let string = |index: usize| match request.argument(index) {
            Some(VmValue::String(value)) => Ok(value.as_str()),
            _ => Err(native_contract_failure(format!(
                "{} argument {} must be string",
                self.name,
                index + 1
            ))),
        };
        let result = match self.name.as_str() {
            "abs" => VmValue::Integer(integer(0)?.checked_abs().ok_or_else(|| {
                native_script_failure(
                    ScriptFaultKind::Arithmetic,
                    "ABS cannot accept the minimum signed integer",
                )
            })?),
            "sign" => VmValue::Integer(integer(0)?.signum()),
            "sqrt" => {
                let value = integer(0)?;
                if value < 0 {
                    return Err(native_script_failure(
                        ScriptFaultKind::Argument,
                        "SQRT argument 1 is negative",
                    ));
                }
                VmValue::Integer((value as f64).sqrt() as i64)
            }
            "cbrt" => {
                let value = integer(0)?;
                if value < 0 {
                    return Err(native_script_failure(
                        ScriptFaultKind::Argument,
                        "CBRT argument 1 is negative",
                    ));
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
                let bit = u32::try_from(integer(1)?).map_err(|_| {
                    native_script_failure(ScriptFaultKind::Argument, "GETBIT index is negative")
                })?;
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
                    return Err(native_contract_failure(
                        "STRFORM template requires VM execution context",
                    ));
                }
                VmValue::String(value.into())
            }
            "toint" => {
                // Only integer-reader errors are swallowed. Evaluating or obtaining the string
                // argument remains outside the fallback, matching the snake evaluator's try block.
                let value = string(0)?;
                let parsed = match parse_era_numeric(value, false) {
                    Ok(value) => value,
                    Err(_) if self.numeric_read_fallback => None,
                    Err(error) => return Err(error.into()),
                };
                VmValue::Integer(parsed.unwrap_or(0))
            }
            "isnumeric" => {
                VmValue::Integer(i64::from(parse_era_numeric(string(0)?, true)?.is_some()))
            }
            "unchecked_add" | "unchecked_sub" | "unchecked_mul" => {
                let [VmValue::Integer(left), VmValue::Integer(right)] = args.as_slice() else {
                    return Err(native_contract_failure(format!(
                        "{} requires exactly two integer arguments",
                        self.name
                    )));
                };
                VmValue::Integer(match self.name.as_str() {
                    "unchecked_add" => left.wrapping_add(*right),
                    "unchecked_sub" => left.wrapping_sub(*right),
                    _ => left.wrapping_mul(*right),
                })
            }
            "unchecked_neg" => {
                let [VmValue::Integer(value)] = args.as_slice() else {
                    return Err(native_contract_failure(
                        "unchecked_neg requires exactly one integer argument",
                    ));
                };
                VmValue::Integer(value.wrapping_neg())
            }
            "convert" => {
                let value = integer(0)?;
                VmValue::String(match integer(1)? {
                    2 => format!("{:b}", value.cast_unsigned()),
                    8 => format!("{:o}", value.cast_unsigned()),
                    10 => value.to_string(),
                    16 => format!("{:x}", value.cast_unsigned()),
                    _ => {
                        return Err(native_script_failure(
                            ScriptFaultKind::Argument,
                            "CONVERT base must be 2, 8, 10, or 16",
                        ));
                    }
                })
            }
            "color_fromrgb" => {
                let channels = [integer(0)?, integer(1)?, integer(2)?];
                if channels.iter().any(|value| !(0..=255).contains(value)) {
                    return Err(native_script_failure(
                        ScriptFaultKind::Argument,
                        "COLOR_FROMRGB channels must be between 0 and 255",
                    ));
                }
                VmValue::Integer((channels[0] << 16) | (channels[1] << 8) | channels[2])
            }
            "color_fromname" => {
                let name = string(0)?;
                if name.eq_ignore_ascii_case("transparent") {
                    return Err(native_script_failure(
                        ScriptFaultKind::Argument,
                        "COLOR_FROMNAME does not accept Transparent",
                    ));
                }
                VmValue::Integer(erabasic_html::named_color(name).map_or(-1, i64::from))
            }
            "unicode" => {
                let value = u32::try_from(integer(0)?).map_err(|_| {
                    native_script_failure(
                        ScriptFaultKind::Argument,
                        "UNICODE argument is outside the UTF-16 code-unit range",
                    )
                })?;
                if value > u32::from(u16::MAX) {
                    return Err(native_script_failure(
                        ScriptFaultKind::Argument,
                        "UNICODE argument is outside the UTF-16 code-unit range",
                    ));
                }
                // Rust strings cannot contain isolated UTF-16 surrogates.  BMP
                // scalar values otherwise have the same UTF-8 observable text.
                let scalar = char::from_u32(value).ok_or_else(|| {
                    native_script_failure(
                        ScriptFaultKind::Argument,
                        "UNICODE argument is an isolated UTF-16 surrogate",
                    )
                })?;
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
                    .ok_or_else(|| native_contract_failure("MAX requires an argument"))?,
            ),
            "min" => VmValue::Integer(
                args.iter()
                    .enumerate()
                    .map(|(index, _)| integer(index))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .min()
                    .ok_or_else(|| native_contract_failure("MIN requires an argument"))?,
            ),
            "limit" => {
                let minimum = integer(1)?;
                let maximum = integer(2)?;
                if minimum > maximum {
                    return Err(native_script_failure(
                        ScriptFaultKind::Argument,
                        "LIMIT minimum exceeds maximum",
                    ));
                }
                VmValue::Integer(integer(0)?.clamp(minimum, maximum))
            }
            "inrange" => VmValue::Integer(i64::from(
                (integer(1)?..=integer(2)?).contains(&integer(0)?),
            )),
            "tostr" => VmValue::String(integer(0)?.to_string()),
            "substring" => {
                let start = match request.argument(1) {
                    None | Some(VmValue::Integer(i64::MIN)) => 0,
                    Some(_) => integer(1)?,
                };
                let length = match request.argument(2) {
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
                let start = match request.argument(1) {
                    None | Some(VmValue::Integer(i64::MIN)) => 0,
                    Some(_) => integer(1)?,
                };
                let length = match request.argument(2) {
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
                let pattern = string(1)?;
                self.regex_cache
                    .get_or_compile(pattern)
                    .map_err(|error| regex_failure("STRCOUNT", &error))?;
                let input = string(0)?;
                let count = self
                    .regex_cache
                    .count_matches(pattern, input)
                    .map_err(|error| regex_failure("STRCOUNT", &error))?;
                VmValue::Integer(i64::try_from(count).unwrap_or(i64::MAX))
            }
            "getpalamlv" | "getexplv" => {
                let maximum = usize::try_from(integer(1)?).map_err(|_| {
                    native_script_failure(
                        ScriptFaultKind::Argument,
                        format!("{} maximum level is negative", self.name),
                    )
                })?;
                let variable = if self.name == "getpalamlv" {
                    "PALAMLV"
                } else {
                    "EXPLV"
                };
                let levels = request.implicit_places.get(variable).ok_or_else(|| {
                    native_contract_failure(format!("{variable} is not available"))
                })?;
                let mut level = maximum;
                for index in 0..maximum {
                    let threshold = match levels.values.get(index + 1) {
                        Some(VmValue::Integer(value)) => *value,
                        _ => {
                            return Err(native_contract_failure(format!(
                                "{variable}[{}] is unavailable",
                                index + 1
                            )));
                        }
                    };
                    if integer(0)? < threshold {
                        level = index;
                        break;
                    }
                }
                VmValue::Integer(i64::try_from(level).unwrap_or(i64::MAX))
            }
            "replace" => VmValue::String(replace_text(&request, &mut self.regex_cache)?),
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
                    let position = usize::try_from(integer(1).unwrap_or(0)).map_err(|_| {
                        native_script_failure(
                            ScriptFaultKind::Bounds,
                            "ENCODETOUNI position is negative",
                        )
                    })?;
                    let scalar = value.chars().nth(position).ok_or_else(|| {
                        native_script_failure(
                            ScriptFaultKind::Bounds,
                            "ENCODETOUNI position exceeds the string",
                        )
                    })?;
                    VmValue::Integer(i64::from(u32::from(scalar)))
                }
            }
            "unicodebyte" => VmValue::Integer(i64::from(u32::from(
                string(0)?.chars().next().ok_or_else(|| {
                    native_script_failure(ScriptFaultKind::Argument, "UNICODEBYTE input is empty")
                })?,
            ))),
            "tolower" => VmValue::String(string(0)?.to_lowercase()),
            "toupper" => VmValue::String(string(0)?.to_uppercase()),
            "unicodetostr" => {
                let scalar = u32::try_from(integer(0)?)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or_else(|| {
                        native_script_failure(
                            ScriptFaultKind::Argument,
                            "UNICODETOSTR argument is not a Unicode scalar",
                        )
                    })?;
                VmValue::String(scalar.to_string())
            }
            _ => {
                return Err(native_contract_failure(format!(
                    "unknown core-native service {}",
                    self.name
                )));
            }
        };
        Ok(NativeReady::value(result))
    }
}
