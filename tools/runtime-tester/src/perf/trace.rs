use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use era_runtime_protocol::{
    ClientCapabilities, DEVICE_PUMP_OPERATION, DEVICE_PUMP_OPERATION_VERSION,
    GET_KEY_STATE_OPERATION, GET_KEY_STATE_OPERATION_VERSION, INPUT_DEVICE_LATCH_CAPABILITY,
    INPUT_DEVICE_PUMP_CAPABILITY, INPUT_ENVIRONMENT_VERSION, InputIntent, InputModality,
    RuntimeFeature, RuntimeMessage, RuntimePhase, SQL_OPERATION, SQL_OPERATION_VERSION,
    ServiceKind, ServiceResult, StorageNamespace, StorageResult, WaitKind,
};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{AuditResult, Cli, TRACE_SCHEMA_VERSION};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PerfTrace {
    pub(super) schema_version: u32,
    pub(super) trace_digest: String,
    pub(super) scenario: String,
    pub(super) project_digest: String,
    pub(super) seed: u64,
    pub(super) client: TraceClient,
    #[serde(default)]
    pub(super) setup_messages: Vec<RuntimeMessage>,
    pub(super) steps: Vec<TraceStep>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct TraceClient {
    pub(super) features: Vec<RuntimeFeature>,
    pub(super) capabilities: ClientCapabilities,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct TraceStep {
    pub(super) id: String,
    pub(super) checkpoint: String,
    pub(super) expect: CheckpointExpectation,
    #[serde(default)]
    pub(super) action: TraceAction,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CheckpointExpectation {
    #[serde(default)]
    pub(super) phase: Option<RuntimePhase>,
    #[serde(default)]
    pub(super) wait_kind: Option<WaitKind>,
    #[serde(default)]
    pub(super) text_contains: Vec<String>,
    #[serde(default)]
    pub(super) outbound_tags: Vec<u32>,
    #[serde(default)]
    pub(super) services: Vec<ServiceExpectation>,
    #[serde(default)]
    pub(super) storage: Vec<StorageExpectation>,
    #[serde(default)]
    pub(super) variables: BTreeMap<String, Value>,
    pub(super) state_signature: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ServiceExpectation {
    pub(super) kind: ServiceKind,
    pub(super) operation: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct StorageExpectation {
    pub(super) namespace: StorageNamespace,
    pub(super) relative_path: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum TraceAction {
    #[default]
    None,
    Input {
        intent: InputIntent,
        #[serde(default)]
        message_skip: bool,
    },
    ServiceResponse {
        service: ServiceExpectation,
        result: ServiceResult,
    },
    StorageResponse {
        storage: StorageExpectation,
        result: StorageResult,
    },
    Submit {
        message: Box<RuntimeMessage>,
    },
}

pub(super) fn load(path: &Path, cli: &Cli) -> AuditResult<PerfTrace> {
    let bytes = fs::read(path)?;
    let value: Value = serde_json::from_slice(&bytes)?;
    validate_canonical_digest(&value)?;
    let trace: PerfTrace = serde_json::from_value(value)?;
    validate(&trace, cli)?;
    Ok(trace)
}

fn validate(trace: &PerfTrace, cli: &Cli) -> AuditResult<()> {
    if trace.schema_version != TRACE_SCHEMA_VERSION {
        return Err(format!("unsupported perf trace schema {}", trace.schema_version).into());
    }
    if trace.scenario != cli.scenario {
        return Err("--scenario does not match trace scenario".into());
    }
    require_lower_hex("traceDigest", &trace.trace_digest)?;
    require_lower_hex("projectDigest", &trace.project_digest)?;
    validate_snake_client(&trace.client)?;
    if trace.steps.is_empty() {
        return Err("perf trace must contain at least one step".into());
    }
    let mut ids = BTreeSet::new();
    let mut checkpoints = BTreeSet::new();
    for step in &trace.steps {
        if step.id.is_empty() || !ids.insert(&step.id) {
            return Err(format!("empty or duplicate trace step id {:?}", step.id).into());
        }
        if step.checkpoint.is_empty() || !checkpoints.insert(&step.checkpoint) {
            return Err(format!("empty or duplicate checkpoint {:?}", step.checkpoint).into());
        }
        require_lower_hex("stateSignature", &step.expect.state_signature)?;
    }
    validate_final_action(&trace.steps)?;
    if let Some(checkpoint) = &cli.pause_at
        && !checkpoints.contains(checkpoint)
    {
        return Err(format!("--pause-at checkpoint {checkpoint:?} is absent from trace").into());
    }
    Ok(())
}

fn validate_snake_client(client: &TraceClient) -> AuditResult<()> {
    for feature in [
        RuntimeFeature::ExternalServices,
        RuntimeFeature::TimedInput,
        RuntimeFeature::StateResynchronization,
    ] {
        if !client.features.contains(&feature) {
            return Err(format!("snake trace is missing required feature {feature:?}").into());
        }
    }
    if !client
        .capabilities
        .input_modalities
        .contains(&InputModality::Keyboard)
        || !client.capabilities.column_cells
        || !client.capabilities.separators
    {
        return Err("snake trace requires keyboard, column-cell, and separator capabilities".into());
    }
    for name in [INPUT_DEVICE_LATCH_CAPABILITY, INPUT_DEVICE_PUMP_CAPABILITY] {
        if !client.capabilities.environment.iter().any(|capability| {
            capability.name == name
                && capability.versions == era_protocol::VersionRange::exact(INPUT_ENVIRONMENT_VERSION)
        }) {
            return Err(format!("snake trace is missing environment capability {name}").into());
        }
    }
    for (kind, operation, version) in [
        (ServiceKind::InputState, GET_KEY_STATE_OPERATION, GET_KEY_STATE_OPERATION_VERSION),
        (ServiceKind::InputState, DEVICE_PUMP_OPERATION, DEVICE_PUMP_OPERATION_VERSION),
        (ServiceKind::Sql, SQL_OPERATION, SQL_OPERATION_VERSION),
    ] {
        if !client.capabilities.services.iter().any(|capability| {
            capability.kind == kind
                && capability.operation == operation
                && capability.versions == era_protocol::VersionRange::exact(version)
        }) {
            return Err(format!("snake trace is missing service capability {kind:?}/{operation}").into());
        }
    }
    Ok(())
}

fn validate_final_action(steps: &[TraceStep]) -> AuditResult<()> {
    if !matches!(steps.last().map(|step| &step.action), Some(TraceAction::None)) {
        return Err("the final trace step must use action kind none".into());
    }
    Ok(())
}

fn validate_canonical_digest(value: &Value) -> AuditResult<()> {
    let expected = value
        .get("traceDigest")
        .and_then(Value::as_str)
        .ok_or("traceDigest is required")?;
    require_lower_hex("traceDigest", expected)?;
    let mut unsigned = value.clone();
    unsigned
        .as_object_mut()
        .ok_or("perf trace root must be an object")?
        .remove("traceDigest");
    let actual = canonical_digest(&unsigned)?;
    if expected != actual {
        return Err(format!("traceDigest mismatch: expected={expected} actual={actual}").into());
    }
    Ok(())
}

pub(super) fn canonical_digest(value: &Value) -> AuditResult<String> {
    let normalized = normalize_json(value);
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&normalized)?)
    ))
}

fn normalize_json(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(normalize_json).collect()),
        Value::Object(values) => {
            let sorted = values
                .iter()
                .map(|(key, value)| (key.clone(), normalize_json(value)))
                .collect::<BTreeMap<_, _>>();
            Value::Object(sorted.into_iter().collect())
        }
        scalar => scalar.clone(),
    }
}

fn require_lower_hex(name: &str, value: &str) -> AuditResult<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(format!("{name} must be 64 lowercase hexadecimal characters").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU32;
    use std::path::PathBuf;
    use serde_json::json;

    #[test]
    fn canonical_digest_is_order_independent_and_lowercase() -> AuditResult<()> {
        let left = json!({"b": [2, {"d": 4, "c": 3}], "a": 1});
        let right = json!({"a": 1, "b": [2, {"c": 3, "d": 4}]});
        let digest = canonical_digest(&left)?;
        assert_eq!(digest, canonical_digest(&right)?);
        require_lower_hex("digest", &digest)?;
        Ok(())
    }

    #[test]
    fn canonical_digest_rejects_tampering_and_uppercase() -> AuditResult<()> {
        let mut value = json!({"schemaVersion": 1, "traceDigest": "0".repeat(64)});
        let unsigned = json!({"schemaVersion": 1});
        value["traceDigest"] = json!(canonical_digest(&unsigned)?);
        validate_canonical_digest(&value)?;
        value["schemaVersion"] = json!(2);
        assert!(validate_canonical_digest(&value).is_err());
        value["traceDigest"] = json!("A".repeat(64));
        assert!(validate_canonical_digest(&value).is_err());
        Ok(())
    }

    #[test]
    fn final_step_must_not_submit_more_work() {
        let step = |action| TraceStep {
            id: "final".into(),
            checkpoint: "final".into(),
            expect: CheckpointExpectation {
                phase: None,
                wait_kind: None,
                text_contains: Vec::new(),
                outbound_tags: Vec::new(),
                services: Vec::new(),
                storage: Vec::new(),
                variables: BTreeMap::new(),
                state_signature: "0".repeat(64),
            },
            action,
        };
        assert!(validate_final_action(&[step(TraceAction::None)]).is_ok());
        assert!(validate_final_action(&[step(TraceAction::Input {
            intent: InputIntent::Continue,
            message_skip: false,
        })]).is_err());
    }

    #[test]
    fn state_signature_covers_full_normalized_state() -> AuditResult<()> {
        let first = json!({
            "phase": "waiting_input", "wait": {"kind": "integer_value"},
            "lines": [{"runs": ["one"]}], "resources": {"sprites": []},
            "scene": {"layers": []}, "variables": {"FLAG:0": 1},
            "services": [{"kind": "sql", "operation": "sql", "payload": [1]}],
            "storage": [], "otherOutboundTags": []
        });
        let mut changed = first.clone();
        changed["lines"][0]["runs"][0] = json!("two");
        assert_ne!(canonical_digest(&first)?, canonical_digest(&changed)?);
        Ok(())
    }

    #[test]
    fn successful_fixture_trace_schema_smoke_validates() -> AuditResult<()> {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixture-snake-perf");
        let project = super::super::input::prepare(&fixture, &mut |_, _| {})?;
        let mut source = json!({
            "schemaVersion": 1,
            "traceDigest": "0".repeat(64),
            "scenario": "fixture-smoke",
            "projectDigest": project.digest,
            "seed": 7,
            "client": {
                "features": ["external_services", "timed_input", "state_resynchronization"],
                "capabilities": {
                    "environment": [
                        {"name": "input.device_latch", "versions": {
                            "minimum": {"major": 1, "minor": 0}, "maximum": {"major": 1, "minor": 0}}},
                        {"name": "input.device_pump", "versions": {
                            "minimum": {"major": 1, "minor": 0}, "maximum": {"major": 1, "minor": 0}}}
                    ],
                    "input_modalities": ["keyboard"], "rich_text": false,
                    "html": false, "graphics": false, "audio": false, "video": false,
                    "font_metrics": false, "column_cells": true, "separators": true,
                    "available_fonts": [], "services": [
                        {"kind": "input_state", "operation": "get_key_state", "versions": {
                            "minimum": {"major": 1, "minor": 0}, "maximum": {"major": 1, "minor": 0}}},
                        {"kind": "input_state", "operation": "device_pump", "versions": {
                            "minimum": {"major": 1, "minor": 0}, "maximum": {"major": 1, "minor": 0}}},
                        {"kind": "sql", "operation": "rustyera.sql", "versions": {
                            "minimum": {"major": 1, "minor": 0}, "maximum": {"major": 1, "minor": 0}}}
                    ],
                    "storage": {"revisions": false, "atomic_replace": false,
                        "missing_precondition": false, "delete": false}
                }
            },
            "steps": [{
                "id": "final", "checkpoint": "final",
                "expect": {"stateSignature": "2".repeat(64)},
                "action": {"kind": "none"}
            }]
        });
        let mut unsigned = source.clone();
        unsigned.as_object_mut().unwrap().remove("traceDigest");
        source["traceDigest"] = json!(canonical_digest(&unsigned)?);
        validate_canonical_digest(&source)?;
        let parsed: PerfTrace = serde_json::from_value(source)?;
        let cli = Cli {
            project: PathBuf::from("copy"),
            profile: super::super::SNAKE_PROFILE,
            scenario: "fixture-smoke".into(),
            trace: PathBuf::from("trace.json"),
            iterations: NonZeroU32::new(1).unwrap(),
            output: None,
            pause_at: None,
            pause_iteration: None,
            maximum_pumps: 1,
            allocator: crate::perf_allocator::MeasurementMode::Off,
        };
        validate(&parsed, &cli)
    }
}
