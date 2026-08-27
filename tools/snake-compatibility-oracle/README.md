# Batch-0 dual-oracle evidence

This directory drives the core-owned `../runtime-tester/fixture-snake-compatibility`
sources. It does not implement the deferred snake semantics or assert Rust parity.
All commands are validation commands and must wait for the shared batch review,
static gates, delegated test executor, and the remaining 60-minute batch budget.

## Inputs and isolation

Use a separately published executable and Wine prefix for each oracle. Supply the
full wrapper SHA, not the semantic `referenceCommit`. The font file is pinned by
SHA-256 in `cases.json`; its source is the snake reference repository's bundled
`BIZUDGothic-Regular.ttf`. Install it in both dedicated Wine prefixes before the
run. The requested family is `BIZ UDGothic`. A missing font, substituted family,
unexpected fallback run, or wrong file hash is a setup failure, never a passing
approximation. The driver does not install fonts or change the font baseline.

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
