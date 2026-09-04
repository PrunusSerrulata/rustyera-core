//! Runtime-owned physical observations. No input text interpretation lives here.
use era_runtime_protocol::{DeviceStateChanged, InputDeviceKind};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Key {
    held: bool,
    // Retained as negotiated host telemetry. Snake GETKEY intentionally does
    // not expose the independent toggle bit used by the legacy service path.
    #[allow(dead_code)]
    toggle: bool,
    latch: bool,
    observation: u8,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeviceInput {
    keys: [Key; 256],
    pub(crate) event_sequence: u64,
}
impl Default for DeviceInput {
    fn default() -> Self {
        Self {
            keys: [Key::default(); 256],
            event_sequence: 0,
        }
    }
}
impl DeviceInput {
    pub(crate) fn apply(&mut self, event: &DeviceStateChanged) -> Result<(), &'static str> {
        let expected = self
            .event_sequence
            .checked_add(1)
            .ok_or("device event sequence exhausted")?;
        if event.event_sequence != expected {
            return Err("device event sequence is stale or has a gap");
        }
        if matches!(
            event.device,
            InputDeviceKind::Touch | InputDeviceKind::Gamepad
        ) {
            self.event_sequence = event.event_sequence;
            return Ok(());
        }
        let index = usize::try_from(event.code)
            .ok()
            .filter(|index| *index < self.keys.len())
            .ok_or("device key code is outside 0..255")?;
        if !matches!(
            event.device,
            InputDeviceKind::Keyboard | InputDeviceKind::Mouse
        ) || event.device == InputDeviceKind::Mouse && !matches!(event.code, 1 | 2 | 4)
            || event.repeat && !event.pressed
        {
            return Err("invalid keyboard/mouse event shape");
        }
        let key = &mut self.keys[index];
        key.held = event.pressed;
        key.toggle = event.toggle;
        if event.pressed {
            key.latch = true;
        }
        self.event_sequence = event.event_sequence;
        Ok(())
    }
    pub(crate) fn clear_latches(&mut self) {
        for key in &mut self.keys {
            key.latch = false;
        }
    }
    /// Caller has already performed the active gate before evaluating the key
    /// expression. Do not recheck focus after an argument method has suspended.
    pub(crate) fn snake_query(&mut self, code: i64, triggered: bool) -> i64 {
        let Some(key) = usize::try_from(code)
            .ok()
            .and_then(|index| self.keys.get_mut(index))
        else {
            return 0;
        };
        let old = key.observation;
        // Fixed WinInput.GetKeyState never includes its separate toggle array.
        // Preserve real toggle telemetry without inventing a snake raw low bit.
        key.observation = 1;
        if !triggered {
            return i64::from(key.held);
        }
        let latched = std::mem::take(&mut key.latch);
        i64::from(latched || key.held && old != 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(sequence: u64, pressed: bool) -> DeviceStateChanged {
        DeviceStateChanged {
            device: InputDeviceKind::Keyboard,
            code: 65,
            pressed,
            x: 0,
            y: 0,
            monotonic_time_ns: sequence,
            event_sequence: sequence,
            toggle: true,
            repeat: false,
        }
    }
    #[test]
    fn held_query_does_not_consume_down_latch_and_up_does_not_clear_it() {
        let mut input = DeviceInput::default();
        input.apply(&event(1, true)).unwrap();
        assert_eq!(input.snake_query(65, false), 1);
        input.apply(&event(2, false)).unwrap();
        assert_eq!(input.snake_query(65, false), 0);
        assert_eq!(input.snake_query(65, true), 1);
        assert_eq!(input.snake_query(65, true), 0);
        assert!(input.keys[65].toggle); // Telemetry is retained; it is not the snake raw low bit.
    }
    #[test]
    fn pump_clear_retains_held_and_observation_but_new_repeat_relatches() {
        let mut input = DeviceInput::default();
        input.apply(&event(1, true)).unwrap();
        assert_eq!(input.snake_query(65, false), 1);
        input.clear_latches();
        assert_eq!(input.snake_query(65, true), 0);
        let mut repeat = event(2, true);
        repeat.repeat = true;
        input.apply(&repeat).unwrap();
        assert_eq!(input.snake_query(65, true), 1);
        assert_eq!(input.snake_query(65, true), 0);
    }
    #[test]
    fn stale_gap_and_invalid_shape_do_not_mutate_device_cells_or_watermark() {
        let mut input = DeviceInput::default();
        input.apply(&event(1, true)).unwrap();
        let before = input.clone();
        for invalid in [
            event(1, false),
            event(3, false),
            DeviceStateChanged {
                code: 256,
                ..event(2, false)
            },
            DeviceStateChanged {
                repeat: true,
                ..event(2, false)
            },
        ] {
            assert!(input.apply(&invalid).is_err());
            assert_eq!(input, before);
        }
    }
}
