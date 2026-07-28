---
name: test-rustyera-core
description: Validate changes in the rustyera-core repository with scope-appropriate checks, ordered Rust workspace gates, regression tests, and Emuera reference-oracle comparisons. Use after modifying rustyera-core code, tests, documentation, tools, configuration, or the C# reference CLI, and whenever preparing the repository's required verification report.
---

# Test RustyEra Core

## Assign testing

Delegate every test command to a sub-agent running **gpt-5.6-terra low**. Instruct it to run
tests only and return each command, exit code, and relevant output. Do not allow it to edit,
format, or commit code, fixtures, documentation, or configuration. Permit test-generated files
only in temporary or ignored directories.

Keep implementation, formatting, test authoring, failure diagnosis, and fixes with the main
agent. Never substitute a main-agent test run for the testing sub-agent.

If any implementation, test, fixture, dependency, or build input changes after a relevant test
starts, immediately tell the testing sub-agent what changed. Require it to rebuild as needed and
rerun every affected check; discard stale results.

## Select the scope

- For Rust implementation changes, run the complete ordered workflow below.
- For C# reference CLI implementation changes, run the same Rust workspace gates before the
  reference smoke and differential checks, even when Rust code did not change.
- When neither Rust nor C# reference CLI implementation changed, do not run the full Rust
  workspace suite or reference differential checks merely as routine validation. Run only checks
  directly relevant to the changed documentation, language, frontend, tool, or configuration.
- For changes to this skill, run the skill validator as a directly relevant check.

## Run the Rust workflow

Require the main agent to format changed Rust code and write the smallest useful unit or
integration test first. Then have the testing sub-agent run these gates in order, stopping at the
first failure:

1. `cargo fmt --all -- --check`
2. `cargo check --workspace --all-targets`
3. `cargo clippy --workspace --all-targets -- -D warnings`
4. The smallest Rust regression test that covers the change
5. `cargo test --workspace`

Do not run the full workspace tests until formatting, compilation, Clippy, and the minimal
regression test pass. Report failures without editing anything; let the main agent fix them and
then rerun affected gates through the testing sub-agent.

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
in `AGENTS.md`, then rerun the failing request and the complete platform smoke test.

If the repair touches `../emuera.em`, also verify that a normal Emuera project still compiles and
append the required entry to `emuera-reference-cli/REFERENCE_CHANGES.md`.

If the current machine cannot run the target platform script, report it as unverified and give the
exact command that must run on that platform.

## Report evidence

Return the commands, exit codes, and concise outcomes for all checks run. Explicitly list skipped
or blocked checks and why. Include the platform smoke result and Rust/C# comparison result when
required. Never describe an unrun or stale check as passing.
