include!("debug_session/dispatch.rs");
include!("debug_session/tests.rs");
mod console;
mod protocol;
mod runtime_console;

#[cfg(test)]
use console::parse_console_expression;
use console::{
    all_debug_scopes, command_scope, console_diagnostic, next_char_boundary,
    parse_console_expression_with_compatibility, previous_char_boundary, scope_bit,
};
use protocol::{
    game_field_descriptors, protocol_breakpoint, protocol_fiber, protocol_frame, protocol_source,
    protocol_storage, protocol_value, protocol_value_in_generation, protocol_variable_value,
    usize_cursor, vm_breakpoint, vm_step_kind, vm_value, vm_variable_reference,
};
