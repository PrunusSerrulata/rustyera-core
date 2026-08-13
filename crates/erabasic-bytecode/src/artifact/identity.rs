use std::io::Write;

use serde::Serialize;

use super::{
    BytecodeConstant, BytecodeFunction, BytecodeFunctionKind, BytecodeType, Digest, ImportKind,
};

#[cfg(any(not(target_arch = "wasm32"), target_feature = "atomics"))]
use rayon::prelude::*;
pub(super) fn canonical_digest<T: Serialize + ?Sized>(
    domain: &str,
    value: &T,
) -> Result<Digest, serde_json::Error> {
    let mut writer = DigestWriter {
        hasher: blake3::Hasher::new_derive_key(domain),
    };
    serde_json::to_writer(&mut writer, value)?;
    Ok(Digest(*writer.hasher.finalize().as_bytes()))
}

#[cfg(any(not(target_arch = "wasm32"), target_feature = "atomics"))]
pub(super) fn identity_join<A, B, RA: Send, RB: Send>(left: A, right: B) -> (RA, RB)
where
    A: FnOnce() -> RA + Send,
    B: FnOnce() -> RB + Send,
{
    rayon::join(left, right)
}

#[cfg(all(target_arch = "wasm32", not(target_feature = "atomics")))]
pub(super) fn identity_join<A, B, RA, RB>(left: A, right: B) -> (RA, RB)
where
    A: FnOnce() -> RA,
    B: FnOnce() -> RB,
{
    (left(), right())
}

pub(super) fn parallel_binary_digest<T: Sync>(
    domain: &str,
    chunk_domain: &str,
    values: &[T],
    chunk_size: usize,
    encode_chunk: fn(&[T], &mut Vec<u8>),
) -> Digest {
    #[cfg(any(not(target_arch = "wasm32"), target_feature = "atomics"))]
    let chunks = values
        .par_chunks(chunk_size)
        .map(|chunk| {
            let mut encoded = Vec::new();
            encode_chunk(chunk, &mut encoded);
            Digest::hash(chunk_domain, &[&encoded])
        })
        .collect::<Vec<_>>();
    #[cfg(all(target_arch = "wasm32", not(target_feature = "atomics")))]
    let chunks = values
        .chunks(chunk_size)
        .map(|chunk| {
            let mut encoded = Vec::new();
            encode_chunk(chunk, &mut encoded);
            Digest::hash(chunk_domain, &[&encoded])
        })
        .collect::<Vec<_>>();
    binary_digest_sequence(domain, &chunks)
}

pub(super) fn binary_digest_sequence(domain: &str, values: &[Digest]) -> Digest {
    let mut encoded = Vec::with_capacity(8 + values.len().saturating_mul(32));
    append_length(&mut encoded, values.len());
    for value in values {
        encoded.extend_from_slice(&value.0);
    }
    Digest::hash(domain, &[&encoded])
}

/// Canonical binary identity encoding for the bytecode section.
///
/// Unlike the public JSON representation, this internal versioned encoding avoids converting
/// millions of numeric operand bytes to decimal text. Every variable-width value is length
/// prefixed, every enum has an explicit tag, and the identity domains are versioned alongside the
/// compiler ABI.
pub(super) fn encode_function_chunk(functions: &[BytecodeFunction], output: &mut Vec<u8>) {
    append_length(output, functions.len());
    for function in functions {
        output.extend_from_slice(&function.key.0);
        append_string(output, &function.name);
        output.push(match function.kind {
            BytecodeFunctionKind::Normal => 0,
            BytecodeFunctionKind::Event => 1,
            BytecodeFunctionKind::System => 2,
            BytecodeFunctionKind::Method => 3,
        });
        append_length(output, function.parameters.len());
        for parameter in &function.parameters {
            output.extend_from_slice(&parameter.key.0);
            append_length(output, parameter.indices.len());
            for index in &parameter.indices {
                output.extend_from_slice(&index.to_le_bytes());
            }
            append_bytecode_type(output, parameter.value_type);
            output.push(u8::from(parameter.by_reference));
            append_constant(output, parameter.default.as_ref());
        }
        match function.result {
            Some(value_type) => {
                output.push(1);
                append_bytecode_type(output, value_type);
            }
            None => output.push(0),
        }
        append_length(output, function.labels.len());
        for label in &function.labels {
            append_string(output, &label.name);
            output.extend_from_slice(&label.instruction.to_le_bytes());
        }
        append_length(output, function.imports.len());
        for import in &function.imports {
            output.push(match import.kind {
                ImportKind::Function => 0,
                ImportKind::Native => 1,
                ImportKind::Host => 2,
            });
            output.extend_from_slice(&import.key.0);
        }
        append_length(output, function.code.len());
        for instruction in &function.code {
            output.extend_from_slice(&instruction.opcode.to_le_bytes());
            append_length(output, instruction.payload.len());
            output.extend_from_slice(instruction.payload.as_slice());
        }
        output.extend_from_slice(&function.max_stack.to_le_bytes());
    }
}

fn append_bytecode_type(output: &mut Vec<u8>, value_type: BytecodeType) {
    output.push(match value_type {
        BytecodeType::Integer => 0,
        BytecodeType::String => 1,
        BytecodeType::IntegerPlace => 2,
        BytecodeType::StringPlace => 3,
    });
}

fn append_constant(output: &mut Vec<u8>, constant: Option<&BytecodeConstant>) {
    match constant {
        None => output.push(0),
        Some(BytecodeConstant::Integer(value)) => {
            output.push(1);
            output.extend_from_slice(&value.to_le_bytes());
        }
        Some(BytecodeConstant::String(value)) => {
            output.push(2);
            append_string(output, value);
        }
    }
}

pub(super) fn encode_source_entry_chunk(entries: &[crate::SourceMapEntry], output: &mut Vec<u8>) {
    append_length(output, entries.len());
    let group_count = entries
        .windows(2)
        .filter(|pair| pair[0].function != pair[1].function)
        .count()
        + usize::from(!entries.is_empty());
    append_length(output, group_count);
    let mut group_start = 0;
    while group_start < entries.len() {
        let function = entries[group_start].function;
        let group_length =
            entries[group_start..].partition_point(|entry| entry.function == function);
        output.extend_from_slice(&function.0);
        append_length(output, group_length);
        for entry in &entries[group_start..group_start + group_length] {
            append_varint(output, entry.code_start);
            append_varint(output, entry.code_end);
            append_varint(output, entry.byte_start);
            append_varint(output, entry.byte_end);
            append_varint(output, u64::from(entry.statement_fingerprint));
            match entry.origin_chain.as_deref() {
                None => output.push(0),
                Some(origins) => {
                    output.push(1);
                    append_length(output, origins.len());
                    for &(source_index, byte_start, byte_end) in origins {
                        append_varint(output, u64::from(source_index));
                        append_varint(output, byte_start);
                        append_varint(output, byte_end);
                    }
                }
            }
            append_varint(output, u64::from(entry.source_index));
        }
        group_start += group_length;
    }
}

fn append_string(output: &mut Vec<u8>, value: &str) {
    append_length(output, value.len());
    output.extend_from_slice(value.as_bytes());
}

fn append_length(output: &mut Vec<u8>, value: usize) {
    output.extend_from_slice(&(value as u64).to_le_bytes());
}

fn append_varint(output: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        output.push(u8::try_from(value & 0x7f).expect("masked varint byte fits in u8") | 0x80);
        value >>= 7;
    }
    output.push(u8::try_from(value).expect("final varint byte fits in u8"));
}

struct DigestWriter {
    hasher: blake3::Hasher,
}

impl Write for DigestWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.hasher.update(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    use crate::artifact::sort_if_needed_by_key;

    fn encode_u64_chunk(values: &[u64], output: &mut Vec<u8>) {
        append_length(output, values.len());
        for value in values {
            output.extend_from_slice(&value.to_le_bytes());
        }
    }

    #[test]
    fn parallel_binary_identity_preserves_serial_chunk_order() {
        let values = (0_u64..10_003).rev().collect::<Vec<_>>();
        let parallel = parallel_binary_digest(
            "test.sequence",
            "test.chunk",
            &values,
            127,
            encode_u64_chunk,
        );
        let serial_chunks = values
            .chunks(127)
            .map(|chunk| {
                let mut encoded = Vec::new();
                encode_u64_chunk(chunk, &mut encoded);
                Digest::hash("test.chunk", &[&encoded])
            })
            .collect::<Vec<_>>();
        assert_eq!(
            parallel,
            binary_digest_sequence("test.sequence", &serial_chunks)
        );
    }

    #[test]
    fn ordered_canonicalization_is_idempotent() {
        let mut values = vec![(1_u8, "a"), (2, "b"), (3, "c")];
        sort_if_needed_by_key(&mut values, |value| value.0);
        let once = values.clone();
        sort_if_needed_by_key(&mut values, |value| value.0);
        assert_eq!(values, once);

        values.swap(0, 2);
        sort_if_needed_by_key(&mut values, |value| value.0);
        assert_eq!(values, once);
    }
}
