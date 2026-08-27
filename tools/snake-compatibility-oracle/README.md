# Dual-oracle compatibility evidence

This directory drives core-owned fixtures. The default remains
`../runtime-tester/fixture-snake-compatibility`; `--fixture` selects a different
fixture explicitly. It does not implement deferred snake semantics or assert Rust parity.
All commands are validation commands and must wait for the shared batch review,
static gates and delegated test executor. The user removed the total batch test
time limit on 2026-08-27; per-command timeouts and stall detection still apply.

## Inputs and isolation

Use a separately published executable and Wine prefix for each oracle. Supply the
full wrapper SHA, not the semantic `referenceCommit`. The font file is pinned by
SHA-256 in `cases.json`; its source is the snake reference repository's bundled
`BIZUDGothic-Regular.ttf`. Install it in both dedicated Wine prefixes before the
run. The requested family is `BIZ UDGothic`. A missing font, substituted family,
unexpected fallback run, or wrong file hash is a setup failure, never a passing
approximation. The driver does not install fonts or change the font baseline.

The prepared `game/` directory is a pristine template, never a running game's
storage. Every case launches a fresh process with `case-games/NNNN/` as its working
directory, loads that copy and records its initial effective fixture hash. Requests inside one case share that copy; saves, overlays
and deletions cannot leak into later cases. Copies and per-case stderr logs remain with the run evidence. The snake engine
resolves string paths relative to the process CWD; passing `gameDir` to the CLI
alone does not set that CWD. No reference implementation change is needed.

Example, from the core worktree (replace the wrapper SHA and paths with actual
isolated build inputs; do not paste a symbolic placeholder as a real SHA):

```sh
python3 tools/snake-compatibility-oracle/run.py \
  --oracle snake --exe /isolated/snake/Emuera.ReferenceCli.exe \
  --wrapper-sha FULL_WRAPPER_SHA --wine wine --wine-prefix /isolated/snake-prefix \
  --rust-evidence ../test-output/batch-0/rust-snake.json \
  --font-file ../emuera_lazyloading_selfmodified_version/Emuera/Properties/fonts/BIZUDGothic-Regular.ttf \
  --output ../test-output/batch-0/snake-evidence --budget-seconds 300
```

Select `--oracle original` with its own executable and prefix for the other run.
`--case` accepts case IDs or group names and may be repeated for targeted recovery.
Existing output directories are never overwritten. The output keeps the disposable
game copy, stderr and `evidence.json`; the main task owns cleanup at final delivery.

## Batch 1A index inputs

Use `--fixture tools/runtime-tester/fixture-snake-index-inputs` for both
`runtime-tester snake-observations` and this driver. Generate separate Rust evidence
for `--profile emuera.em` and `--profile emuera.skia.snake`, then run the corresponding
original and snake oracle with those files. The new manifest requires snake
semantic/policy 2/2; historical policy 1/1 evidence remains supported for the older
fixture. `recompare.py --fixture` uses the same explicit fixture and policy checks.

The index fixture contains BUFF CSV/ALS, two- and three-dimensional user ERD/ALS,
and built-in FLAG aliases at 10/11/300. Numeric slots hold different values before
the shared functions read the supplied string index. Inputs cover primary-name
priority, first alias wins, trimming, same-index aliases, ERDNAME canonical names,
and negative/oversized array access. GETNUM is intentionally absent: its user-table
extension belongs to batch 2. No game source or reference repository is modified.

Static and dynamic primary names have separate acceptance cases. The
`index-static-primary-names` entry uses direct names in BUFF, COLUMNDIV and
SEMEN_MATRIX and must succeed under both Rust profiles and both references.
`index-primary-name-precedes-alias`, `index-column-primary` and
`index-matrix-primary-300` retain identical dynamic string requests for all engines:
both fixed references and Rust snake succeed, while Rust original currently faults.
Those three cases carry `original_dynamic_user_index_existing_gap` and
`knownRustDifference` metadata. Their original-oracle success assertions are unchanged;
the Rust/original comparison must remain `different`, never `expected_rejection`.
Batch 1A freezes that existing original-profile gap instead of changing its resolver.

The pinned original oracle also aborts the rest of `FLAG.als` at its duplicate-index
warning: `ConstantData.cs:1812` supplies one formatting argument to the two-placeholder
message at `Lang.cs:655`. The same-index and untrimmed-name cases therefore fail in
that oracle, while Rust preserves its existing continue-after-warning behavior.
Those cases retain `original_builtin_alias_warning_existing_gap` differences; the
trimmed-name rejection cannot independently establish whitespace semantics after
the earlier abort. These are explicit oracle defects, not Rust parity successes.
Correcting this assertion metadata changes the fixture identity even though source
files and request expressions are unchanged. Historical evidence stays immutable;
targeted recovery must use matching fixture and Rust evidence hashes.

Every oracle case must first reach the fixture title's input wait. A handled load
request with error/timeout termination is a failure, and a Rust load failure cannot
satisfy an expected operation rejection. Original user-ALS rejection and profile
specific trimming differences are recorded under `expected_rejection`; their raw
diagnostics remain `incomparable_schema`, never an asserted error-equivalence pass.
Both engines receive the same source and request expressions, with only the existing
documented title wrapper and rendering configuration adaptation. Review the Rust
comparison statuses as well as the driver exit code; observation completion alone
is not batch acceptance.

The smallest checks after the unique batch review are the existing Python driver
tests and the runtime-tester `snake_observations::fixture::tests` module. All oracle
runs remain downstream of the complete authorized static gates and use isolated
game copies, output directories and Wine prefixes. A full presentation load may fail
when diagnostic text needs a fallback font outside the pinned family. Preserve that
failure as a presentation setup failure; do not relabel the substituted font as a match.
For a separately recorded data-only recovery, `--logical-output-only` requests values,
logical output and diagnostics without a presentation snapshot or font-family assertion.
The evidence records `presentationObservation=not_requested`; it cannot establish font
or pixel compatibility. This mode rejects PRINTC and explicit presentation assertions.
It does not alter game source, expected values, error outcomes, or the five-second watchdog.

## Evidence and Rust integration

`cases.json` is the shared request plan: case ID, group, entry/arguments/input trace,
watches, known oracle assertions and later semantic batch. Every response retains
diagnostics, output, values, termination, schema and semantic reference identity.
Cases marked `observation` collect behavior whose exact result must be reviewed
before freezing a golden; they are reported `observed`, not semantic passes.

The evidence identifies the original source fixture separately from each engine's
effective configuration (snake uses SKIASHARP, original TEXTRENDERER). The font,
provider, provider version, actual family/fallback and layout values are retained.
The supplied font file hash and observed family do not prove which installed
font bytes the provider selected; `fontByteSource` explicitly retains that limit.
This establishes measured layout, not GUI/GPU raster equivalence.
Use a separate `--drawing-mode TEXTRENDERER --case PRINTC` snake run to inspect
its alternative path; the default snake run uses SKIASHARP. Provider identity is
read from the existing node measurement cache, not inferred from global mode.

The `runtime-tester snake-observations` command reads the same `cases.json` and
source files under an explicit compatibility profile. Its evidence is required by
`--rust-evidence`; the driver rejects a mismatched seed, profile, policy or source
file hash before launching an oracle. Record both full core SHA and working-tree
state. Run Rust/reference and Rust/snake separately.

Each step preserves actual values, output, diagnostics and termination, or the
specific compilation/execution blocker. `comparison.py` records differing fields
without suppressing diagnostics; it never treats missing Rust data as a match.
Eval compares values, while run/execute compare logical line arrays, watches and
termination. Wrapper-only output is not an eval result. Setup diagnostics and host
logs are retained separately from each step's script diagnostics. Non-empty
cross-engine diagnostics and errors have no shared schema yet and are explicitly
`incomparable_schema`, even if both engines failed; raw responses remain attached.
Pixel presentation and core column presentation are retained as different policies.
The instrumentation-only reset/atomicity case is labeled separately. A completed
comparison is not compatibility acceptance: `different`, `blocked` and
`incomparable` and `matched_observables` remain distinct, and `snakeTargetStatus=deferred_semantics`
is never rewritten into a passing snake-target result.

## Focused verification

`python3 -m unittest discover -s tools/snake-compatibility-oracle -p 'test_*.py'`
checks the driver helpers. The named GETKEY cases exercise mutation rejection,
reset isolation and non-consuming observation against the actual reference engine.
An independent watchdog samples the complete last observable state every five
seconds across requests and cases. Consecutive identical samples fail immediately;
only protocol envelope IDs are omitted from comparison, not script fields named
`id`. A synchronous blocked oracle is never sent a concurrent observation request.

## Correcting comparison-only defects without replaying engines

Run/execute output is cumulative since load. The comparator removes only the exact complete
output prefix captured by that case's successful load response. A changed/reset prefix is
`incomparable`, not a regex-filtered match. Script lines that happen to say “Now Loading...”
remain observable. A configuration warning is separated only when its exact code, level,
configuration source/context and validated identity match the experimental-profile diagnostic.
Raw load, step output and diagnostics remain attached. The NDJSON `ok` flag means a request
was accepted; a console `termination=error` still means execution failed. Two engine failures
remain incomparable until diagnostic schemas can be compared; they are not compatibility passes.

For an existing completed observation captured from the current fixture, use:

```sh
python3 tools/snake-compatibility-oracle/recompare.py \
  --oracle-evidence /isolated/old-run/evidence.json \
  --rust-evidence /isolated/rust-observations.json \
  --output /isolated/new-comparison.json
```

This validates semantic baseline, profile, seed, all fixture hashes and ordered requests, then
creates new evidence with source-file hashes. It never changes old evidence or reruns either
engine. Failed/incomplete captures cannot be relabeled completed.

The pinned snake RNG roundtrip observation reports `pinned_oracle_rng_state_loss`: at seed
123456 its values are 192905, 520548, 0, 0. `DumpRanddata` passes a temporary `ToArray` copy
to `GetRand`, so `INITRAND` restores the unchanged zero RANDDATA. The driver preserves this
as an observed baseline defect, not a successful roundtrip; original-oracle equality remains
strict. Normal reference engine semantics are unchanged.
