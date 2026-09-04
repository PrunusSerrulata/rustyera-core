# `perf-run` trace contract

`perf-run` is the runtime tester's only authoritative performance probe for both loading and
runtime execution. The former
`benchmark` command and its original-era default identity have been removed. `perf-run` replays a
versioned frontend trace against a fresh `RuntimeSession` for every iteration. It accepts only the
explicit `emuera.skia.snake` profile and rejects the shared `games/eratw-sub-modding` tree; always
pass an isolated copy. `compile`, `coverage`, `project-extractor-all`, and restore diagnostics retain
ordinary elapsed progress messages for troubleshooting, but they are not performance probes or
baselines. No probe or counter is added to a production library; all allocator
and JSONL instrumentation lives in this audit binary.

```text
cargo run --release --manifest-path tools/runtime-tester/Cargo.toml -- perf-run \
  --project AUDIT_COPY --profile emuera.skia.snake \
  --scenario day-one --trace day-one.trace.json --iterations 5 \
  --allocator counting --output day-one.core.jsonl
```

Use `--allocator off` for the no-counter timing baseline. The metadata event records a fixed
counter calibration (`offNs`, `countingNs`, `overheadPercent`); timing with excessive overhead is
suitable for attribution, not absolute latency. A profiler pause is either a single-iteration run
with `--pause-at CHECKPOINT`, or explicitly selected in a repeated run with `--pause-at CHECKPOINT
--pause-iteration ZERO_BASED_ITERATION`.

## Versioned trace

The trace is strict JSON with `schemaVersion: 1`. `traceDigest` is the lowercase SHA-256 of the
canonical JSON object after removing `traceDigest`: object keys are recursively sorted, arrays keep
their order, and the normalized value is serialized as compact JSON. The runner recomputes it and
rejects modified or uppercase digests. `projectDigest` is generated in the same sorted input pass
that constructs the manifest. Text inputs are decoded once and submitted inline; resources are
stream-hashed once and represented as `ExternalResource`, so a frontend resource service remains
part of the captured capability contract.

For a capture producer, the project digest framing is exact: sort submitted relative paths by Rust
string order; for each input append little-endian `u64(path UTF-8 byte length)`, path bytes,
little-endian `u64(category compact-JSON byte length)`, category compact-JSON bytes, little-endian
`u64(content byte length)`, and content bytes to one SHA-256 stream. Text content is the decoded
inline UTF-8 string after BOM/encoding handling; resource content is the original binary bytes.
The lowercase final digest is `projectDigest`. The per-file `contentHash` remains BLAKE3 and is not
substituted into this SHA-256 stream.

```json
{
  "schemaVersion": 1,
  "traceDigest": "64 lowercase hexadecimal characters",
  "scenario": "day-one",
  "projectDigest": "64 lowercase hexadecimal characters",
  "seed": 1,
  "client": {
    "features": ["external_services", "timed_input", "state_resynchronization"],
    "capabilities": {
      "environment": [
        {
          "name": "input.device_latch",
          "versions": {
            "minimum": {"major": 1, "minor": 0},
            "maximum": {"major": 1, "minor": 0}
          }
        },
        {
          "name": "input.device_pump",
          "versions": {
            "minimum": {"major": 1, "minor": 0},
            "maximum": {"major": 1, "minor": 0}
          }
        }
      ],
      "input_modalities": ["keyboard"],
      "rich_text": false,
      "html": false,
      "graphics": false,
      "audio": false,
      "video": false,
      "font_metrics": false,
      "column_cells": true,
      "separators": true,
      "available_fonts": [],
      "services": [
        {
          "kind": "input_state",
          "operation": "get_key_state",
          "versions": {
            "minimum": {"major": 1, "minor": 0},
            "maximum": {"major": 1, "minor": 0}
          }
        },
        {
          "kind": "input_state",
          "operation": "device_pump",
          "versions": {
            "minimum": {"major": 1, "minor": 0},
            "maximum": {"major": 1, "minor": 0}
          }
        },
        {
          "kind": "sql",
          "operation": "rustyera.sql",
          "versions": {
            "minimum": {"major": 1, "minor": 0},
            "maximum": {"major": 1, "minor": 0}
          }
        }
      ],
      "storage": {
        "revisions": false,
        "atomic_replace": false,
        "missing_precondition": false,
        "delete": false
      }
    }
  },
  "setupMessages": [],
  "steps": [
    {
      "id": "ready",
      "checkpoint": "ready-input",
      "expect": {
        "phase": "waiting_input",
        "waitKind": "integer_value",
        "textContains": ["READY"],
        "variables": {"FLAG:0": 1},
        "stateSignature": "64 lowercase hexadecimal characters"
      },
      "action": {"kind": "none"}
    }
  ]
}
```

Every step pumps until all declared expectations match, then compares `stateSignature`. The
signature is the same canonical digest over the complete normalized checkpoint: phase, wait,
logical lines and runs, resource replay, scene, requested variables, service kind/operation/version
and payload, storage namespace/path/operation, and remaining outbound tags. Session-local request
IDs, deadlines, and envelope IDs are excluded. This detects semantic drift that summary counts or
visible text alone miss. `Idle`, `Stopped`, or `Faulted` before a match fails immediately. The last
step must use `{"kind":"none"}`; an action after the final verified state is invalid.

The normalized signature object uses these exact keys: `phase`, `wait`, `lines`, `resources`,
`scene`, `variables`, `services`, `storage`, and `otherOutboundTags`. Each service item is
`{kind, operation, operationVersion, payload}`. Each storage item is
`{namespace, relativePath, operation}`. Arrays preserve captured protocol order; only JSON object
keys are sorted by canonicalization.

Expectations can also list numeric `outboundTags`, `{kind, operation}` service requests, or
`{namespace, relativePath}` storage requests. Actions are `none`, `input`, `service_response`,
`storage_response`, or `submit`. Service/storage actions bind the current request ID rather than
persisting session IDs. `setupMessages` run after negotiation and before the manifest, allowing a
Tauri capture to reproduce extension registration. The trace's client capabilities must include
every snake feature the scenario exercises. The validator requires the shared minimal snake host
contract shown above (keyboard latch/pump, input-state services, and `rustyera.sql@1.0`); captures
may include additional Tauri capabilities for resource, storage, canvas, or scenario-specific work.

The background Tauri capture is the source of truth for these records; do not maintain a second
handwritten Core trajectory. Its export maps one observation to one `steps[]` record, preserving
the shared `id`, `checkpoint`, `expect`, and `action` field names and the protocol's serde values.
The conversion boundary may only remove session-local request IDs/deadlines, normalize the complete
checkpoint with the rules above, calculate `stateSignature`, calculate the sorted project input
digest, and finally calculate `traceDigest`. It must preserve action order, service/storage payloads,
client capabilities, seed, scenario, and the final `none` action. The resulting JSON is consumed by
`perf-run` directly; a separate Core-specific action dialect or manual transcription is invalid.

## JSONL and profiling

Every telemetry record uses output schema v2 and has `schemaVersion`, `epoch`, monotonically
increasing `sequence`, and
`event` fields. Records cover metadata, iteration
boundaries, every pump, aggregate phases, full normalized checkpoints, optional profiler pauses,
and one flushed terminal record (`passed` or `failed`). Loading records cover input decoding and
hashing plus handshake, setup, manifest submission, and project load; `period` distinguishes them
from stable runtime steps in the same stream. Pump and phase records separate VM
instructions, runtime transitions, envelope counts/bytes, snapshot/delta counts, presentation
metrics, RSS, and allocator data. Allocator data is an object in `counting` mode and JSON `null` in
`off` mode. In a pump, `netBytes` is signed growth during that measurement window and
`peakNetBytes` is that window's peak. Phase totals sum signed window growth and report the maximum
of the window peaks; allocated/deallocated totals remain separately available. Allocations freed
while counting is disabled are intentionally outside both sides of the measurement.

Allocation windows cover runtime drive and outbound byte polling. Protocol decode, JSON emission,
debug-variable reads, and `/bin/ps` are outside them. The runner uses the real default runtime
limits and records them in metadata rather than silently inflating the manifest allowance.

`--pause-at` emits and flushes the child PID, then requires one complete stdin line. EOF is a
failure. This holds one selected, already-validated runtime checkpoint for `/usr/bin/sample`,
`heap`, `vmmap`, `leaks`, or `malloc_history` without GUI. The supervisor's identical-state rule is
suspended only for this explicit phase; its wall-clock budget and process cleanup still apply.
Outside the pause, the child refreshes a complete runtime watchdog snapshot at least every four
seconds and immediately on checkpoint transitions.
