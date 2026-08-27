# Snake Emuera reference checks

Read this when the core task involves 蛇版emuera (`emuera_lazyloading_selfmodified_version`).
All commands below run from `rustyera-core/`. This is an additional oracle, not a replacement
baseline for unrelated original Emuera behavior. Follow the selection table in [SKILL.md](../SKILL.md).

## Entrypoints and identity

- Source and protocol: `../emuera_lazyloading_selfmodified_version/emuera-reference-cli/README.md`.
  Read it before constructing requests; inspect `capabilities` before the first comparison.
- Semantic baseline: `fc4fb21416768c17256d0e82f997e5f99c9bba91`.
- Check every response's `schemaVersion` (2) and `referenceCommit`; capabilities identifies
  `implementation = emuera_lazyloading_selfmodified_version`. A mismatched executable is a
  failed setup, not a passing alternative oracle.
- The CLI targets .NET 8 / Windows x64, configuration `Debug-NAudio`; original Emuera targets
  .NET 10. macOS needs Wine and Python 3.9+; Windows also needs Python 3.9+.
- Published executable: `../emuera_lazyloading_selfmodified_version/emuera-reference-cli/bin/smoke-win-x64/Emuera.ReferenceCli.exe`.
- Default macOS prefix: workspace `.wine-prefix/emuera-selfmodified-cli`. Explicitly select this
  prefix if the environment already has `WINEPREFIX` set; never reuse the original oracle prefix.

After all applicable static gates pass, run the selected platform entrypoint once:

```sh
WINEPREFIX="$(cd .. && pwd)/.wine-prefix/emuera-selfmodified-cli" \
  bash ../emuera_lazyloading_selfmodified_version/emuera-reference-cli/tests/test-macos-wine.sh
```

```powershell
& ../emuera_lazyloading_selfmodified_version/emuera-reference-cli/tests/protocol-smoke.ps1
```

Both entrypoints restore and publish before running the same Python smoke driver. Its default
300-second budget covers smoke execution only, not restore/build; bound the whole command by
the remaining task budget and lower `--budget-seconds` (PowerShell `-BudgetSeconds`) as needed.
Do not infer that per-request timeouts replace the skill's 5-second observable-state watchdog;
arrange observation of the active process/request and VM state where available before execution.

## Fixtures and coverage

The base fixture is `../emuera_lazyloading_selfmodified_version/emuera-reference-cli/tests/fixture`.
Only load disposable copies: the native engine can write config, logs, caches, and saves. The smoke
driver copies fixtures into temporary directories and cleans them up. Keep real 蛇版TW
(`../games/eratw-sub-modding`) separate from original eraTW (`../games/eraTW`) and from these fixtures.

| Smoke group | Coverage relevant to core comparisons |
| --- | --- |
| `protocol` | Persistent NDJSON, malformed request recovery, identity, lexer/expression parser |
| `csv` | Project loading, line/semantic analysis, CSV values |
| `runtime` | VM output, arguments, watches, float values and sparse arrays |
| `inputs` | Input/resume, ONEINPUT text normalization, NF waits and SEQUENCEINPUT |
| `reload` | Per-project config/mouse-input isolation and repeatable explicit seed |
| `save` | Read-only arbitrary save path, restored values, SYSTEM_LOADEND/EVENTLOAD continuation |
| `limits` | Instruction/time limits, explicit script error and recovery |
| `presentation` | Print-family execution completion and SKIASHARP config, not GUI rendering |

`fixture/erb/selfmodified.erb` and `.erh` contain variant-specific cases. `fixture-oneinput`,
`fixture-oneinput-long`, and `fixture-save` are overlays. `fixture-system` is available for targeted
system-flow comparisons but is not covered merely because the eight smoke groups pass.
Extend minimal core-owned fixtures and Rust regression tests for the actual change; the eight
groups are oracle health checks, not proof that Rust supports those features.

## Compare behavior without losing variant differences

- Use the same UTF-8 source/CSV, configuration intent, entry arguments, input sequence, seed,
  watch expressions and execution limits for Rust and the selected C# engine. Use independent
  copies/processes for each engine; record any engine-specific configuration adaptation.
- `parseLine` requires a loaded project in snake Emuera. Honor `capabilities.requiresLoad`;
  an uninitialized parser failure does not establish a language incompatibility.
- Float literal/evaluation/watch values are JSON numbers; non-finite values use `NaN`, `Infinity`,
  and `-Infinity` strings. Compare types and values explicitly; do not coerce these to integers/null.
- NF input exposes `state=WaitInputNoFocus` and `termination=waitingInput`. Preserve both the
  raw state and the portable wait meaning; do not erase the variant behavior in normalization.
- `output` is the full display buffer, not a delta. Save loading must verify restored variables
  and subsequent system/event execution, not just successful decoding.
- RNG algorithms, saves, config and lazy-loading behavior may differ from the original engine;
  an equal seed does not by itself prove identical random sequences. Record expected differences
  explicitly rather than weakening assertions until both oracles pass.
- When original comparison is required, also run the core original platform smoke entrypoint and
  feed the shared cases to Rust/original. For snake-only syntax, record the original engine's
  actual rejection or unsupported result alongside Rust/snake behavior; do not claim equivalence.

## Targeted post-fix verification

After rebuilding changed inputs and restoring the affected static gates, run only the failing
group(s) against the current published executable. Do not rerun the full smoke entrypoint.
Set the numeric budget below no higher than the remaining task budget:

```sh
WINEPREFIX="$(cd .. && pwd)/.wine-prefix/emuera-selfmodified-cli" WINEDEBUG=-all \
  python3 ../emuera_lazyloading_selfmodified_version/emuera-reference-cli/tests/smoke.py \
  --wine wine \
  --exe ../emuera_lazyloading_selfmodified_version/emuera-reference-cli/bin/smoke-win-x64/Emuera.ReferenceCli.exe \
  --case inputs --budget-seconds 120
```

```powershell
python ../emuera_lazyloading_selfmodified_version/emuera-reference-cli/tests/smoke.py `
  --exe ../emuera_lazyloading_selfmodified_version/emuera-reference-cli/bin/smoke-win-x64/Emuera.ReferenceCli.exe `
  --case inputs --budget-seconds 120
```

Replace `inputs` with the directly affected group; repeat `--case` only for other affected groups.
Rerun the affected Rust/C# differential cases separately. Report original full-smoke failures,
targeted recovery, and any missing platform or semantic coverage independently for each oracle.
