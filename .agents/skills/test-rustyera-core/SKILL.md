---
name: test-rustyera-core
description: Validate changes in the rustyera-core repository with scope-appropriate checks, ordered Rust workspace gates, regression tests, and Emuera reference-oracle comparisons. Use after modifying rustyera-core code, tests, documentation, tools, configuration, or the C# reference CLI, and whenever preparing the repository's required verification report.
---

# Test RustyEra Core

## Enforce the task budget

- Before starting any test command, confirm that any required refactoring subagent has completed
  its single permitted run and that every requirement it reported has been implemented. Refuse to
  start testing while any refactoring requirement remains. Once the first test starts, never spawn,
  resume, follow up with, or rerun a refactoring subagent during that task.
- Start one shared 60-minute wall-clock budget when the task's first test command starts. Include
  every later test, targeted rerun, end-to-end wait, and test-failure investigation in that budget;
  no command timeout may exceed the remaining time.
- Start each distinct full test suite at most once per task. If it exposes a failure, repair it and
  rerun only the smallest directly affected test target, never the full suite again. Report the
  original full-suite result separately from the targeted post-fix result.
- Run every command that may outlive its initial tool response in a persistent PTY. Start it with
  `exec_command` using `tty: true` and a short yield, retain the returned `session_id`, and poll only
  with `write_stdin` at intervals no longer than 30 seconds until an explicit exit code is observed.
  Do not resume a yielded exec cell with a separate wait call: the cell may be reclaimed before its
  result is collected. If a PTY session disappears without an exit code, report the command as
  unverified; never restart a full suite, and rerun a targeted command only when the suite rules
  permit it.
- Every end-to-end, long-running, and reference-oracle flow must emit a complete observable-state
  snapshot every 5 seconds. Include every HTML element when an HTML client is involved. Compare
  snapshots without timestamps and other reporting-only metadata; if two consecutive snapshots
  are identical, immediately terminate the flow as stalled and report the error.
- At the 60-minute deadline, terminate all test processes and report the active command, exact
  case or stage, last snapshot, elapsed time, completed checks, and unverified checks.

## Select the scope

- For Rust implementation changes, run the complete ordered workflow below.
- For C# reference CLI implementation changes, run the same Rust workspace gates before the
  reference smoke and differential checks, even when Rust code did not change.
- When neither Rust nor C# reference CLI implementation changed, do not run the full Rust
  workspace suite or reference differential checks merely as routine validation. Run only checks
  directly relevant to the changed documentation, language, frontend, tool, or configuration.
- For changes to this skill, run the skill validator as a directly relevant check.

## Run the Rust workflow

Format changed Rust code and write the smallest useful unit or integration test first. Then run
these gates in order, stopping at the first failure:

1. `cargo fmt --all -- --check`
2. `cargo check --workspace --all-targets`
3. `cargo clippy --workspace --all-targets -- -D warnings`
4. The smallest Rust regression test that covers the change
5. `cargo test --workspace` (once only)

Do not run the full workspace tests until formatting, compilation, Clippy, and the minimal
regression test pass. If the one full workspace run fails, report it, fix the problem separately,
and rerun only the affected package, test binary, module, or named tests. Never rerun
`cargo test --workspace` in the same task.

Do not use a C# oracle test as a replacement for a Rust implementation test.

## Validate against the reference implementation

After all Rust gates pass for a Rust or C# reference CLI implementation change:

1. Run the current platform smoke script:
   - Windows: `tools/protocol-smoke.ps1`
   - macOS: `tools/test-macos-wine.sh`
2. Feed the same input to Rust and the C# reference CLI.
3. Compare token data, AST or semantic structure, diagnostics, output, variable values, and
   termination reason.

Treat a passing platform smoke test only as proof that the oracle starts and responds; it is not
a differential comparison. Ignore only explicit environment metadata such as request IDs and
absolute paths. Record every intentional semantic difference in tests and in the delivery report.

When adding syntax or an execution path, extend the applicable fixtures, request set, and Rust
tests so both implementations receive identical input.

On macOS, expect the script to use `.wine-prefix/emuera-reference-cli` and write ignored output
under `.wine-tmp/emuera-reference-cli`.

## Handle oracle failures

Treat timeout, empty output, premature exit, and protocol errors as reference CLI defects. Do not
skip the oracle and claim validation. Diagnose and repair the CLI first within the authorization
in `AGENTS.md`, then rerun only the failing request or directly affected smoke case. The complete
platform smoke test must not be run a second time in the same task.

If the repair touches `../emuera.em`, also verify that a normal Emuera project still compiles and
append the required entry to `emuera-reference-cli/REFERENCE_CHANGES.md`.

If the current machine cannot run the target platform script, report it as unverified and give the
exact command that must run on that platform.

## Report evidence

Return the commands, exit codes, and concise outcomes for all checks run. Explicitly list skipped
or blocked checks and why. Include the platform smoke result and Rust/C# comparison result when
required. Never describe an unrun or stale check as passing.
