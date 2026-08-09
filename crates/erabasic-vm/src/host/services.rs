#[allow(clippy::wildcard_imports)]
use super::*;

pub(super) struct CompilerNative {
    pub(super) name: String,
}

impl NativeService for CompilerNative {
    fn call(&mut self, request: NativeCallRequest) -> Result<NativeReady, String> {
        match self.name.as_str() {
            "format_integer" => {
                let Some(VmValue::Integer(value)) = request.arguments.first() else {
                    return Err("format_integer expects an integer".into());
                };
                Ok(NativeReady::value(VmValue::String(apply_width(
                    &value.to_string(),
                    request.arguments.get(1),
                    request.arguments.get(2),
                )?)))
            }
            "format_string" => {
                let Some(value) = request.arguments.first() else {
                    return Err("format_string expects a value".into());
                };
                let value = match value {
                    VmValue::Integer(value) => value.to_string(),
                    VmValue::String(value) => value.clone(),
                    VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => {
                        return Err("format_string cannot dereference a place".into());
                    }
                };
                Ok(NativeReady::value(VmValue::String(apply_width(
                    &value,
                    request.arguments.get(1),
                    request.arguments.get(2),
                )?)))
            }
            "times" => {
                let place = request
                    .places
                    .iter()
                    .find(|place| place.argument_index == 0)
                    .ok_or("TIMES expects an integer place")?;
                let Some(VmValue::Integer(value)) = place.values.first() else {
                    return Err("TIMES expects an integer place".into());
                };
                let Some(VmValue::Integer(numerator)) = request.arguments.get(1) else {
                    return Err("TIMES expects an integer numerator".into());
                };
                let Some(VmValue::Integer(denominator)) = request.arguments.get(2) else {
                    return Err("TIMES expects an integer denominator".into());
                };
                if *denominator <= 0 {
                    return Err("TIMES denominator must be positive".into());
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
            name if name.starts_with("control_") => Err(format!(
                "compiler control placeholder {name} reached execution"
            )),
            _ => Err(format!("unknown compiler-native service {}", self.name)),
        }
    }
}

pub(super) struct RandomNative {
    pub(super) name: String,
    pub(super) state: Arc<Mutex<Sfmt19937>>,
}

impl NativeService for RandomNative {
    fn call(&mut self, request: NativeCallRequest) -> Result<NativeReady, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "SFMT state lock is poisoned".to_owned())?;
        match self.name.as_str() {
            "rand" => {
                let (minimum, maximum) = match request.arguments.as_slice() {
                    [VmValue::Integer(maximum)] => (0, *maximum),
                    [VmValue::Integer(minimum), VmValue::Integer(maximum)] => {
                        // The internal expression ABI represents an omitted first
                        // operand as i64::MIN. RAND(, max) is equivalent to RAND(max).
                        (if *minimum == i64::MIN { 0 } else { *minimum }, *maximum)
                    }
                    _ => return Err("RAND expects one or two integer arguments".into()),
                };
                let Some(width) = maximum.checked_sub(minimum) else {
                    return Err("RAND range overflows i64".into());
                };
                if width <= 0 {
                    return Err("RAND maximum must be greater than its minimum".into());
                }
                let offset = state.next_u64() % width.cast_unsigned();
                let value = i64::try_from(offset)
                    .expect("RAND modulo positive i64 fits i64")
                    .checked_add(minimum)
                    .ok_or_else(|| "RAND result overflows i64".to_owned())?;
                Ok(NativeReady::value(VmValue::Integer(value)))
            }
            "randomize" => {
                let seed = match request.arguments.first() {
                    Some(VmValue::Integer(seed)) => (*seed).cast_unsigned(),
                    None => 0,
                    _ => return Err("RANDOMIZE seed must be an integer".into()),
                };
                state.reseed(seed);
                Ok(NativeReady::default())
            }
            "initrand" | "dumprand" => Err(format!(
                "{} must be executed through the VM place transaction",
                self.name.to_ascii_uppercase()
            )),
            _ => Err(format!("unknown random-native service {}", self.name)),
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

pub(super) fn apply_width(
    value: &str,
    width: Option<&VmValue>,
    alignment: Option<&VmValue>,
) -> Result<String, String> {
    let Some(width) = width else {
        return Ok(value.into());
    };
    let VmValue::Integer(signed_width) = width else {
        return Err("format width must be an integer".into());
    };
    let width = usize::try_from(*signed_width)
        .map_err(|_| "format width exceeds this platform".to_owned())?;
    let left_align = match alignment {
        Some(VmValue::Integer(value)) => *value != 0,
        Some(_) => return Err("format alignment must be an integer".into()),
        None => false,
    };
    let characters = crate::emuera_display_width(value);
    if characters >= width {
        return Ok(value.into());
    }
    let padding = " ".repeat(width - characters);
    Ok(if left_align {
        format!("{value}{padding}")
    } else {
        format!("{padding}{value}")
    })
}
