//! Private staged DEFAULT calls. Tickets never re-resolve a user-visible name.

use crate::ExecutionFailure;
use crate::structured::argument_failure;
use erabasic_bytecode::{BytecodeType, NATIVE_ABI_VERSION};

use super::data_table::{
    data_type_code, implicit_place, indexed_target, parse_data_type, string_argument,
};
use super::{Cell, DataType, HostWrite, NativeCallRequest, NativeReady, StructuredState, VmValue};

pub(crate) fn is_internal_column_native(name: &str) -> bool {
    [
        "dt__column_resolve",
        "dt__column_check_int",
        "dt__column_check_str",
        "dt__column_apply_int",
        "dt__column_apply_str",
    ]
    .iter()
    .any(|candidate| name.eq_ignore_ascii_case(candidate))
}

#[derive(Clone, Copy)]
struct ColumnTicket {
    identity: u64,
    value_type: DataType,
}

impl ColumnTicket {
    fn encode(self) -> String {
        format!(
            "dtc1:{:016x}:{}",
            self.identity,
            data_type_code(self.value_type)
        )
    }

    fn decode(value: &str, next_identity: u64) -> Result<Self, ExecutionFailure> {
        let bytes = value.as_bytes();
        if bytes.len() != 23
            || !bytes.starts_with(b"dtc1:")
            || bytes[21] != b':'
            || !bytes[5..21]
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
            || !(b'1'..=b'5').contains(&bytes[22])
        {
            return Err("DT_COLUMN_OPTIONS invalid column ticket".into());
        }
        let identity = u64::from_str_radix(&value[5..21], 16)
            .map_err(|_| "DT_COLUMN_OPTIONS invalid column identity")?;
        if identity == 0 || identity >= next_identity {
            return Err("DT_COLUMN_OPTIONS column identity is outside its timeline".into());
        }
        Ok(Self {
            identity,
            value_type: parse_data_type(&VmValue::Integer(i64::from(bytes[22] - b'0')))?,
        })
    }
}

impl StructuredState {
    pub(super) fn call_column_option(
        &mut self,
        name: &str,
        request: &NativeCallRequest,
    ) -> Result<NativeReady, ExecutionFailure> {
        validate_request(name, request)?;
        if name == "dt__column_resolve" {
            return self.resolve_column_option(request);
        }
        let ticket = ColumnTicket::decode(string_argument(request, 0)?, self.next_column_identity)?;
        let expects_string = name.ends_with("_str");
        if expects_string != (ticket.value_type == DataType::String) {
            return Err(argument_failure(
                "DT_COLUMN_OPTIONS DEFAULT value type differs from selected column",
            ));
        }
        if self
            .find_column_identity(ticket.identity)
            .is_some_and(|column| column.value_type != ticket.value_type)
        {
            return Err("DT_COLUMN_OPTIONS column ticket type differs from live identity".into());
        }
        if name.starts_with("dt__column_apply_") {
            let value = default_value(ticket.value_type, &request.arguments[1])?;
            // A removed column cannot be reattached by any supported script API.
            // Value evaluation still occurred; its detached default has no observer.
            if let Some(column) = self.find_column_identity_mut(ticket.identity) {
                column.default_value = value;
            }
        }
        Ok(NativeReady::default())
    }

    fn resolve_column_option(
        &self,
        request: &NativeCallRequest,
    ) -> Result<NativeReady, ExecutionFailure> {
        let column_name = string_argument(request, 0)?;
        let table_name = string_argument(request, 1)?;
        let result = if let Some(table) = self.data_tables.get(table_name) {
            if let Some(index) = table.column(column_name) {
                let column = &table.columns[index];
                return Ok(NativeReady::value(VmValue::String(
                    ColumnTicket {
                        identity: column.identity,
                        value_type: column.value_type,
                    }
                    .encode(),
                )));
            }
            0
        } else {
            -1
        };
        let target = implicit_place(request, "RESULT")?;
        if !matches!(target.values.first(), Some(VmValue::Integer(_))) {
            return Err("DT_COLUMN_OPTIONS RESULT[0] is unavailable or not Integer".into());
        }
        Ok(NativeReady {
            value: Some(VmValue::String(String::new())),
            writes: vec![HostWrite {
                target: indexed_target(&target.target, 0),
                value: VmValue::Integer(result),
            }],
        })
    }
}

fn validate_request(name: &str, request: &NativeCallRequest) -> Result<(), ExecutionFailure> {
    use BytecodeType::{Integer, String as Text};
    let (parameters, result): (&[BytecodeType], Option<BytecodeType>) = match name {
        "dt__column_resolve" => (&[Text, Text], Some(Text)),
        "dt__column_check_int" | "dt__column_check_str" => (&[Text], None),
        "dt__column_apply_int" => (&[Text, Integer], None),
        "dt__column_apply_str" => (&[Text, Text], None),
        _ => return Err(format!("unknown private column native {name}").into()),
    };
    let import = &request.import;
    if import.namespace != "rustyera.vm"
        || import.name != name
        || import.abi_version != NATIVE_ABI_VERSION
        || import.parameters != parameters
        || import.result != result
        || !request.places.is_empty()
        || request.arguments.len() != parameters.len()
        || !request
            .arguments
            .iter()
            .zip(parameters)
            .all(|(value, expected)| value.value_type() == *expected)
    {
        return Err("DT_COLUMN_OPTIONS invalid private native signature or arguments".into());
    }
    Ok(())
}

fn default_value(value_type: DataType, value: &VmValue) -> Result<Cell, ExecutionFailure> {
    match (value_type, value) {
        (DataType::String, VmValue::String(value)) => Ok(Cell::String(value.clone())),
        (DataType::String, _)
        | (_, VmValue::String(_) | VmValue::IntegerPlace(_) | VmValue::StringPlace(_)) => Err(
            argument_failure("DT_COLUMN_OPTIONS DEFAULT value type differs from selected column"),
        ),
        (_, VmValue::Integer(value)) => Ok(Cell::Integer(match value_type {
            DataType::Int8 => (*value).clamp(i64::from(i8::MIN), i64::from(i8::MAX)),
            DataType::Int16 => (*value).clamp(i64::from(i16::MIN), i64::from(i16::MAX)),
            DataType::Int32 => (*value).clamp(i64::from(i32::MIN), i64::from(i32::MAX)),
            DataType::Int64 => *value,
            DataType::String => unreachable!(),
        })),
    }
}
