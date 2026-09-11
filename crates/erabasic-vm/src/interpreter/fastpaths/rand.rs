//! Shared static/dynamic RAND policy; valid samples still use the existing SFMT provider.
use super::{Fiber, Vm, VmValue};
use crate::interpreter::compatibility_diagnostics::CompatibilityWarning;

impl Vm {
    pub(super) fn clamp_integer_rand(
        &mut self,
        fiber: &Fiber,
        name: &str,
        arguments: &[VmValue],
        omitted: &[usize],
    ) -> Option<i64> {
        let generation = self.generations.get(&fiber.frames.last()?.generation)?;
        if !generation
            .artifact
            .manifest
            .compatibility
            .clamps_integer_rand()
            || generation.artifact.call_compatibility.compatible_rand
        {
            return None;
        }
        let (value, warning) = match (name, arguments) {
            ("__rand_variable", [VmValue::Integer(maximum)]) if *maximum <= 0 => {
                (0, CompatibilityWarning::RandVariable)
            }
            ("rand", [VmValue::Integer(maximum)]) if *maximum <= 0 => {
                (0, CompatibilityWarning::RandFunction)
            }
            ("rand", [VmValue::Integer(minimum), VmValue::Integer(maximum)]) => {
                let minimum = if omitted.contains(&0) { 0 } else { *minimum };
                if *maximum > minimum {
                    return None;
                }
                (minimum, CompatibilityWarning::RandFunction)
            }
            _ => return None,
        };
        self.queue_compatibility_warning(warning);
        Some(value)
    }
}
