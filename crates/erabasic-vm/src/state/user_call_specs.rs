use erabasic_bytecode::{BytecodeFunction, Opcode, UserCallSpec};

// Derived metadata only. Count limits bound directory storage, entry-vector growth,
// and decoded argument allocations independently; uncached calls use normal decoding.
const MAXIMUM_FUNCTIONS: usize = 65_536;
const MAXIMUM_CALLS: usize = 65_536;
const MAXIMUM_ARGUMENTS: usize = 262_144;

#[derive(Clone, Debug)]
pub(super) struct DecodedUserCallSpecs {
    functions: Vec<Vec<(u32, UserCallSpec)>>,
}

impl DecodedUserCallSpecs {
    pub(super) fn new(functions: &[BytecodeFunction], progress: impl FnMut()) -> Self {
        Self::with_limits(
            functions,
            MAXIMUM_FUNCTIONS,
            MAXIMUM_CALLS,
            MAXIMUM_ARGUMENTS,
            progress,
        )
    }

    pub(super) fn with_limits(
        functions: &[BytecodeFunction],
        maximum_functions: usize,
        mut remaining_calls: usize,
        mut remaining_arguments: usize,
        mut progress: impl FnMut(),
    ) -> Self {
        let mut cached = Vec::with_capacity(functions.len().min(maximum_functions));
        for (function_index, function) in functions.iter().enumerate() {
            if function_index < maximum_functions {
                let mut entries = Vec::new();
                if remaining_calls > 0 {
                    for (index, instruction) in function.code.iter().enumerate() {
                        if instruction.opcode != Opcode::ResolveUserCall as u16 {
                            continue;
                        }
                        let Some(header) = instruction.payload.get(..8) else {
                            continue;
                        };
                        let arguments = usize::from(u16::from_le_bytes([header[6], header[7]]));
                        if arguments > remaining_arguments {
                            continue;
                        }
                        let Ok(index) = u32::try_from(index) else {
                            break;
                        };
                        let Ok(spec) = UserCallSpec::decode(&instruction.payload) else {
                            // Do not change the execution-time diagnostic for invalid operands.
                            continue;
                        };
                        remaining_arguments -= spec.arguments.len();
                        remaining_calls -= 1;
                        entries.push((index, spec));
                        if remaining_calls == 0 {
                            break;
                        }
                    }
                }
                cached.push(entries);
            }
            progress();
        }
        Self { functions: cached }
    }

    pub(super) fn get(&self, function: usize, instruction: usize) -> Option<&UserCallSpec> {
        let entries = self.functions.get(function)?;
        let instruction = u32::try_from(instruction).ok()?;
        let index = entries
            .binary_search_by_key(&instruction, |entry| entry.0)
            .ok()?;
        Some(&entries[index].1)
    }
}

#[cfg(test)]
#[path = "user_call_specs_tests.rs"]
mod tests;
