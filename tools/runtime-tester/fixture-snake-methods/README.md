# Batch 1B dynamic-method fixture

Status: batch 1B Rust static gates passed, including the workspace suite and
runtime-tester method assertions. Dual-oracle observations remain pending. The
35-case fixture is not yet a claim of complete cross-engine parity.

This directory is independent of `fixture-snake-index-inputs` and the batch 0
fixture. It contains 35 cases using the existing version-1 `cases.json` request
schema. Each case loads a fresh project and runs one ordinary wrapper; the wrapper
resets observable state before exercising a dynamic method. `SYSTEM_TITLE` stops
at `METHODS_READY` without invoking any case. This is a small language fixture,
not a graph initialization or game startup test.

## Coverage and observations

- Integer/String results, present/missing target fallback laziness, and ordered
  target-name/argument/body side effects.
- Ordinary-function, return-type, argument-type, required-argument, REF type,
  REF rank, and omitted-REF errors before actual-argument evaluation. Global
  counters remain observable after an error; error messages are not goldens.
- Explicit omitted slots and trailing defaults, with real `i64::MIN` as a
  separate present argument. The minimum is formed by a representable constant
  subtraction, not an out-of-range positive integer token.
- Integer and String whole-array REF writeback. `ARRAY:METHOD_INDEX()` still
  binds the whole array and must not evaluate the element index. This does not
  request element REF, OUT, variadic, or Float support.
- `EXISTMETH` performs zero-argument resolution, including defaulted/required/REF
  formals, and never executes method bodies. Ordinary functions, builtins,
  missing names, and event labels are included.
- Finite recursion and capturing a value before a later argument mutates its
  source cell. Resource-limit and program-generation/reload tests still belong
  in the Rust execution suite, not in this single static project.
- Minimal `CAN_MOVE_*`, `ODEKAKEMAP_SETTING_*`, and `SCOM_DIFF_*` computed-name
  forms, plus a call nested in a formatted expression. Targets are deliberately
  reachable only through computed names. The baseline config keeps
  `Ignore uncalled functions:NO`; separate compiler tests must repeat with
  pruning and optimizations enabled.
- Runtime `STRFORM` has a separate **observation-only** case because nested
  quoting and runtime parser acceptance have not been executed. Its expected
  semantic target is documented in the case, without a success assertion.

The non-variadic extra-argument case is a deliberate difference: the original
reference rejects excess actuals before evaluation; the snake reference executes
only the fixed formals and never evaluates discarded extras. Batch 1B keeps the
current strict Rust arity policy, so the snake comparison must remain different
until batch 2. `knownRustDifference` is explanatory metadata, not a waiver and
not an instruction to turn a mismatch into a pass.

All concrete expected values are derived from the fixture body and the fixed
source branches listed below. They are prospective assertions, not recorded
oracle results. The formatted-expression case is additionally marked for
observation of parser/span validity. A load failure must never satisfy an
expected operation rejection.

## Source basis

Paths in this section are relative to the workspace component's sibling
reference or game repository. Neither reference repository was modified.

| Basis | Fixed source location |
|---|---|
| Target resolution, missing-only fallback, return check, existence query | Both `Emuera/Runtime/Script/Statements/Function/Creator.Method.cs`, `GetMethMethod`, `GetMethsMethod`, `ExistMethMethod`; original around 7467, snake around 9697 |
| User-method-only lookup; ordinary function errors; event invisibility | Both `Emuera/Runtime/Script/Data/IdentifierDictionary.cs::GetFunctionMethod` and `LabelDictionary.cs::GetNonEventLabel` |
| Signature checked before evaluation | Both `Emuera/Runtime/Script/Statements/Function/UserDefinedMethodTerm.cs::Create` call `ConvertArg` |
| Omitted/default/type/REF checks and extra-argument difference | Both `Emuera/Runtime/Script/Process.CalledFunction.cs::CalledFunction.ConvertArg`; original around 154, snake around 212 |
| Left-to-right value capture; non-character whole-array REF ignores indices | Both `Process.CalledFunction.cs::UserDefinedFunctionArgument.SetTransporter` |
| `#DIM REF` without explicit sizes remains rank one | Both `Emuera/Runtime/Script/Data/UserDefinedVariable.cs`, missing `sizeNum` receives one dimension |
| Numeric dynamic game forms | Snake TW `ERB/魔改内容/qol/qol_graph_init.ERB`, `CAN_MOVE_FOR_GRAPH` and `ODEKAKEMAP_SETTING_FOR_GRAPH`, around 347–369 |
| String dynamic game form | Snake TW `ERB/魔改内容/GRAPH系.ERB`, `GETMETHS` around 725 |

`cases.json` pins both semantic baselines and the established font identity.
Config explicitly disables optional undeclared arguments and implicit numeric
to-string argument conversion, so omission and type assertions do not depend on
machine defaults. It does not enable SQL, graphics, or a new arithmetic policy.

## Harness integration and validation boundary

The METHODS group and ERH submission are now wired into the Rust harness after batch 1A completed.
The unique batch 1B review and Rust static gates have completed; oracle and client
integration remain separate acceptance steps.

1. `src/snake_observations/fixture.rs::load_fixture_files` maps group `METHODS`
   to `erb/methods.erb` and includes `erb/methods.erh` as `Erh`. Old groups keep
   their exact inputs. The generated observation title replaces `erb/base.erb`,
   so it does not duplicate `SYSTEM_TITLE`.
2. Rust execution assertions cover both profiles, including direct VM state
   inspection after errors. The current Python `comparison.py::compare_case`
   compares only `ok` on failed operations, so error watches are retained in raw
   evidence but are not part of cross-engine parity checks. Do not
   describe those error states as compared solely because both sides failed.
3. Use the existing oracle runner's `--fixture` selection after the 1B review
   and static gates. Verify successful load, preserve original/snake results
   separately, and keep the extra-argument discrepancy visible. Run against
   isolated copies only; do not write saves or traces into this fixture.
4. Rust-specific recursion depth, validator, cache/memo, hot replacement,
   pruning, and optimization-switch tests supplement this fixture. It cannot prove
   those properties or the eventual three-client integration.

Integer and String method-statement cases also verify lazy fallback and RESULT/RESULTS storage.
The runtime debugger cannot currently pause a faulted VM for watches; those missing observations
must remain explicit. VM execution tests inspect error-time variables directly, while the oracle
asserts its error watches. Do not describe those watches as compared by the runtime harness.

The original source-only preparation ran no tests. Subsequent Rust validation is
recorded in the implementation log; nothing here marks batch 1B complete.
