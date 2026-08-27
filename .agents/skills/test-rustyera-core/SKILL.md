---
name: test-rustyera-core
description: Validate rustyera-core changes with scope-appropriate checks, ordered Rust gates, regression tests, and original or snake Emuera reference-oracle comparisons. Use after modifying core code, tests, documentation, tools, configuration, or either C# reference CLI, and when preparing the required verification report.
---

# Test RustyEra Core

## Enforce the task budget

Follow the root `AGENTS.md` parallel scheduling rules. Run independent checks concurrently when
their inputs, outputs, and mutable resources are isolated; pipeline dependent checks as prerequisites
pass. Parallelism never bypasses the required review, focused-before-full, or static-before-dynamic
gates. Delegate test execution as required by the component's `AGENTS.md`.

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
these gates with the dependencies below, stopping affected downstream work on failure:

1. `cargo fmt --all -- --check`
2. `cargo check --workspace --all-targets`
3. `cargo clippy --workspace --all-targets -- -D warnings`
4. The smallest Rust regression test that covers the change
5. `cargo test --workspace` (once only)

The first four gates may run concurrently when build/output isolation permits it. Do not run the
full workspace tests until formatting, compilation, Clippy, and the minimal
regression test pass. If the one full workspace run fails, report it, fix the problem separately,
and rerun only the affected package, test binary, module, or named tests. Never rerun
`cargo test --workspace` in the same task.

Do not use a C# oracle test as a replacement for a Rust implementation test.

## Validate against the reference implementation

### Select the oracle before running C# tests

Paths below are relative to the `rustyera-core` repository root. “蛇版emuera” means
`../emuera_lazyloading_selfmodified_version`; “蛇版TW” means `../games/eratw-sub-modding`;
“eraTW” means the original TW at `../games/eraTW`. Never substitute one game for the other.
Resolve bare “蛇版” from context; if its meaning is not reliable, ask the user before selecting
an engine, game, or oracle. The original reference engine is `../emuera.em`, not a TW game.

| Changed behavior | Required C# oracle(s) |
| --- | --- |
| Does not involve snake Emuera | Original `emuera.em` |
| Involves snake Emuera | Snake Emuera instead of the original-only flow |
| Involves snake Emuera and compatibility behavior, or also needs comparison with the original | Both snake Emuera and original `emuera.em` |

Use the most specific applicable row. This routing applies to feature development, modifications,
and fixes. It does not expand the
documentation-only validation scope above. Record the selection and reason before testing.
For any snake Emuera work, read [references/snake-oracle.md](references/snake-oracle.md) for
the independent entrypoints, fixtures, protocol differences, and targeted rerun commands.

### Run the selected reference checks

After all Rust gates pass for a Rust or C# reference CLI implementation change:

1. Run the current platform smoke script for each selected oracle:
   - Original / Windows: `tools/protocol-smoke.ps1`
   - Original / macOS: `tools/test-macos-wine.sh`
   - Snake / Windows: `../emuera_lazyloading_selfmodified_version/emuera-reference-cli/tests/protocol-smoke.ps1`
   - Snake / macOS: `bash ../emuera_lazyloading_selfmodified_version/emuera-reference-cli/tests/test-macos-wine.sh`
2. Feed the same input to Rust and each selected C# reference CLI. For dual-oracle work, record
   Rust/original and Rust/snake comparisons separately; do not require the two C# engines to
   agree on intentional variant behavior.
3. Compare token data, AST or semantic structure, diagnostics, output, variable values, and
   termination reason.

Treat a passing platform smoke test only as proof that the oracle starts and responds; it is not
a differential comparison. Ignore only explicit environment metadata such as request IDs and
absolute paths. Validate and retain each oracle's `schemaVersion` and `referenceCommit` before
normalizing responses: original baseline `26a35dc9334bb67590b96f7b8efbefbf199e391e`, snake baseline
`fc4fb21416768c17256d0e82f997e5f99c9bba91`. Also record the wrapper checkout commit. Do not hide
semantic differences as metadata; record every intentional difference in tests and the report.

When adding syntax or an execution path, extend the applicable fixtures, request set, and Rust
tests so Rust and the selected oracle receive identical input. For dual-oracle work, distinguish
shared-language cases from snake extensions; an unsupported original operation is an explicit
comparison outcome, not grounds for silently skipping the original. Reference repositories remain
read-only except for the narrow authorization in `AGENTS.md`; place task-specific comparison
fixtures in core or temporary copies unless reference fixture edits are explicitly authorized.

The original macOS script uses the workspace's `.wine-prefix/emuera-reference-cli` and ignored
`.wine-tmp/emuera-reference-cli`. Snake uses `.wine-prefix/emuera-selfmodified-cli` and temporary
fixture copies. Keep prefixes, processes, requests, and evidence separate; never point the original
script's `EMUERA_REFERENCE_ROOT` at the snake tree to bypass its distinct .NET target and fixtures.
Both suites share the same task-wide 60-minute budget, static-before-dynamic gates, and watchdog
rules. Each full oracle smoke suite may start once only, independently of the other suite.

## Handle oracle failures

Treat timeout, empty output, premature exit, and protocol errors as reference CLI defects. Do not
skip the oracle and claim validation. Diagnose and repair the CLI first within the authorization
in `AGENTS.md`, then rerun only the failing request or directly affected smoke case. The complete
platform smoke test must not be run a second time in the same task.

If the repair touches either reference repository, also verify that its normal Emuera project still
compiles and append the required per-file audit entry: original
`../emuera.em/emuera-reference-cli/REFERENCE_CHANGES.md`, or snake
`../emuera_lazyloading_selfmodified_version/emuera-reference-cli/HEADLESS_CHANGES.md`.

If the current machine cannot run the target platform script, report it as unverified and give the
exact command that must run on that platform.

## Report evidence

Return the commands, exit codes, and concise outcomes for all checks run. Explicitly list skipped
or blocked checks and why. Include the oracle selection and reason, baseline and wrapper commits,
each selected platform smoke result, and each required Rust/C# comparison result. Separate the
first full-suite outcome from targeted post-fix checks for each engine. Never describe an unrun or
stale check as passing, or treat snake-oracle success as original-oracle success (or vice versa).
