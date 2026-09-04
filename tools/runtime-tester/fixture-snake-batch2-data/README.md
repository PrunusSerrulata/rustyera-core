# Batch 2C deterministic data API fixture

This fixture exercises the Batch 2C snake-only MATCH, character CSV lookup,
bit-array, and MAP contracts. Each case runs in a fresh project and process.
The pinned original engine is expected to reject the snake-only API names at
load; its rejection is captured separately and is not treated as a successful
execution comparison.

The source expectations are candidates until both the Rust snake profile and
the pinned snake reference have executed each case. Runtime-only atomicity,
snapshot, reload, stale-reference, and resource-limit boundaries remain covered
by the core test suite rather than fabricated as reference behavior.
