use super::AuditResult;
use serde_json::Value;

#[derive(Clone, Copy, Default)]
pub(super) struct KeyState {
    pub(super) down: bool,
    pub(super) toggle: bool,
}

pub(super) fn parse_key_events(events: &Value) -> AuditResult<Vec<(u8, KeyState)>> {
    let events = events
        .as_array()
        .ok_or("key event batch must be an array")?;
    events
        .iter()
        .map(|event| {
            let code = event["keyCode"]
                .as_u64()
                .and_then(|code| u8::try_from(code).ok())
                .ok_or("keyCode must be an integer in 0..255")?;
            let down = event["down"].as_bool().ok_or("down must be boolean")?;
            let toggle = event["toggle"].as_bool().ok_or("toggle must be boolean")?;
            Ok((code, KeyState { down, toggle }))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn key_trace_validates_complete_batch_and_preserves_toggle() {
        let batch =
            parse_key_events(&json!([{ "keyCode": 65, "down": true, "toggle": false }])).unwrap();
        assert_eq!(batch[0].0, 65);
        assert!(batch[0].1.down);
        assert!(!batch[0].1.toggle);
        assert!(
            parse_key_events(&json!([{ "keyCode": 65, "down": true, "toggle": false },
            { "keyCode": 256, "down": false, "toggle": false }]))
            .is_err()
        );
    }
}
