#[allow(clippy::wildcard_imports)]
use super::*;

pub(super) struct CompilerNative {
    pub(super) name: String,
    pub(super) character_width_mode: CharacterWidthModeHandle,
}

impl NativeService for CompilerNative {
    fn call(&mut self, request: NativeCallRequest) -> Result<NativeReady, ExecutionFailure> {
        match self.name.as_str() {
            "format_integer" => {
                let Some(VmValue::Integer(value)) = request.arguments.first() else {
                    return Err(native_contract_failure("format_integer expects an integer"));
                };
                Ok(NativeReady::value(VmValue::String(
                    apply_owned_width_with_mode(
                        value.to_string(),
                        request.arguments.get(1),
                        request.arguments.get(2),
                        self.character_width_mode.get(),
                    )?,
                )))
            }
            "format_string" => {
                let Some(value) = request.arguments.first() else {
                    return Err(native_contract_failure("format_string expects a value"));
                };
                let value = match value {
                    VmValue::Integer(value) => value.to_string(),
                    VmValue::String(value) => value.clone(),
                    VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => {
                        return Err(native_contract_failure(
                            "format_string cannot dereference a place",
                        ));
                    }
                };
                Ok(NativeReady::value(VmValue::String(
                    apply_owned_width_with_mode(
                        value,
                        request.arguments.get(1),
                        request.arguments.get(2),
                        self.character_width_mode.get(),
                    )?,
                )))
            }
            "times" => {
                let place = request
                    .places
                    .iter()
                    .find(|place| place.argument_index == 0)
                    .ok_or_else(|| native_contract_failure("TIMES expects an integer place"))?;
                let Some(VmValue::Integer(value)) = place.values.first() else {
                    return Err(native_contract_failure("TIMES expects an integer place"));
                };
                let Some(VmValue::Integer(numerator)) = request.arguments.get(1) else {
                    return Err(native_contract_failure(
                        "TIMES expects an integer numerator",
                    ));
                };
                let Some(VmValue::Integer(denominator)) = request.arguments.get(2) else {
                    return Err(native_contract_failure(
                        "TIMES expects an integer denominator",
                    ));
                };
                if *denominator <= 0 {
                    return Err(native_contract_failure(
                        "TIMES denominator must be positive",
                    ));
                }
                // Emuera's default rigorous path multiplies through decimal and
                // truncates toward zero. i128 preserves that result for every i64
                // operand and parser-produced i64 rational multiplier.
                let result =
                    (i128::from(*value) * i128::from(*numerator)) / i128::from(*denominator);
                let value = i64::try_from(result).unwrap_or(i64::MIN);
                Ok(NativeReady {
                    value: None,
                    writes: vec![HostWrite {
                        target: place.target.clone(),
                        value: VmValue::Integer(value),
                    }],
                })
            }
            name if name.starts_with("control_") => Err(native_contract_failure(format!(
                "compiler control placeholder {name} reached execution"
            ))),
            _ => Err(native_contract_failure(format!(
                "unknown compiler-native service {}",
                self.name
            ))),
        }
    }
}

pub(super) struct RandomNative {
    pub(super) name: String,
    pub(super) state: Arc<Mutex<Sfmt19937>>,
}

impl NativeService for RandomNative {
    fn call(&mut self, request: NativeCallRequest) -> Result<NativeReady, ExecutionFailure> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| native_contract_failure("SFMT state lock is poisoned"))?;
        match self.name.as_str() {
            "rand" => {
                let (minimum, maximum) = match request.arguments.as_slice() {
                    [VmValue::Integer(maximum)] => (0, *maximum),
                    [VmValue::Integer(minimum), VmValue::Integer(maximum)] => {
                        // The internal expression ABI represents an omitted first
                        // operand as i64::MIN. RAND(, max) is equivalent to RAND(max).
                        (if *minimum == i64::MIN { 0 } else { *minimum }, *maximum)
                    }
                    _ => {
                        return Err(native_contract_failure(
                            "RAND expects one or two integer arguments",
                        ));
                    }
                };
                let Some(width) = maximum.checked_sub(minimum) else {
                    return Err(native_script_failure(
                        ScriptFaultKind::Arithmetic,
                        "RAND range overflows i64",
                    ));
                };
                if width <= 0 {
                    return Err(native_script_failure(
                        ScriptFaultKind::Argument,
                        "RAND maximum must be greater than its minimum",
                    ));
                }
                let offset = state.next_u64() % width.cast_unsigned();
                let value = i64::try_from(offset)
                    .expect("RAND modulo positive i64 fits i64")
                    .checked_add(minimum)
                    .ok_or_else(|| native_contract_failure("RAND result overflows i64"))?;
                Ok(NativeReady::value(VmValue::Integer(value)))
            }
            "randomize" => {
                let seed = match request.arguments.first() {
                    Some(VmValue::Integer(seed)) => (*seed).cast_unsigned(),
                    None => 0,
                    _ => return Err(native_contract_failure("RANDOMIZE seed must be an integer")),
                };
                state.reseed(seed);
                Ok(NativeReady::default())
            }
            "initrand" | "dumprand" => Err(native_contract_failure(format!(
                "{} must be executed through the VM place transaction",
                self.name.to_ascii_uppercase()
            ))),
            _ => Err(native_contract_failure(format!(
                "unknown random-native service {}",
                self.name
            ))),
        }
    }

    fn snapshot(&self) -> Result<Option<Vec<u8>>, String> {
        self.state
            .lock()
            .map(|state| Some(state.encode()))
            .map_err(|_| "SFMT state lock is poisoned".into())
    }

    fn restore(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.state
            .lock()
            .map_err(|_| "SFMT state lock is poisoned".to_owned())?
            .decode(bytes)
    }
}

#[cfg(test)]
pub(crate) fn apply_width_with_mode(
    value: &str,
    width: Option<&VmValue>,
    alignment: Option<&VmValue>,
    mode: crate::CharacterWidthMode,
) -> Result<String, ExecutionFailure> {
    apply_owned_width_with_mode(value.to_owned(), width, alignment, mode)
}

pub(crate) fn apply_owned_width_with_mode(
    mut value: String,
    width: Option<&VmValue>,
    alignment: Option<&VmValue>,
    mode: crate::CharacterWidthMode,
) -> Result<String, ExecutionFailure> {
    let Some(width) = width else {
        return Ok(value);
    };
    let VmValue::Integer(signed_width) = width else {
        return Err(native_contract_failure("format width must be an integer"));
    };
    if *signed_width < 0 {
        return Err(native_script_failure(
            ScriptFaultKind::Argument,
            "format width exceeds this platform",
        ));
    }
    let width = usize::try_from(*signed_width)
        .map_err(|_| native_resource_failure("format width exceeds this platform"))?;
    let left_align = match alignment {
        Some(VmValue::Integer(value)) => *value != 0,
        Some(_) => {
            return Err(native_contract_failure(
                "format alignment must be an integer",
            ));
        }
        None => false,
    };
    let characters = crate::display_width(&value, mode);
    if characters >= width {
        return Ok(value);
    }
    let padding = width - characters;
    if left_align {
        value.try_reserve_exact(padding).map_err(|_| {
            native_resource_failure("format width allocation exceeds available capacity")
        })?;
        value.extend(std::iter::repeat_n(' ', padding));
        return Ok(value);
    }
    let capacity = value.len().checked_add(padding).ok_or_else(|| {
        native_resource_failure("format width allocation exceeds available capacity")
    })?;
    let mut padded = String::new();
    padded.try_reserve_exact(capacity).map_err(|_| {
        native_resource_failure("format width allocation exceeds available capacity")
    })?;
    padded.extend(std::iter::repeat_n(' ', padding));
    padded.push_str(&value);
    Ok(padded)
}

#[cfg(test)]
pub(super) fn apply_width(
    value: &str,
    width: Option<&VmValue>,
    alignment: Option<&VmValue>,
) -> Result<String, ExecutionFailure> {
    apply_width_with_mode(
        value,
        width,
        alignment,
        crate::CharacterWidthMode::Automatic,
    )
}
