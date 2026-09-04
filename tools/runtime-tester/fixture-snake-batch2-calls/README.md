# 2B call/check fixture candidates

Prepared only; no Rust or oracle execution. `common/` contains only syntax supported
by both pinned engines. Its 2 source-derived cases cover exact CALLFORM and whole
array REF (non-character index is not evaluated). `snake/` contains 16 snake-only
cases: six CALLSTR modes with comma/parenthesized argument syntax, three blank JUMP
variants, and STRFORMCHECK retaining a completed side effect before expansion failure.
Run the existing METHODS observation group against each respective fixture root.
Never load snake/ into the original oracle. `allowedOracles` is descriptive metadata,
not a new enforced driver feature; the future executor must select the correct root.

Both roots retain base.erb, tiny GAMEBASE.CSV, config and font identity from the
accepted 2A fixture; no executable, font download, game directory or evidence copied.
The snake policy 4 requirement is the proposed complete 2B integration contract,
not a claim that a draft or current product has passed its gates.

`source-manifest.json` records pinned baseline and current file SHA256 separately.
The two Instraction.Child.cs whole-file differences are existing headless input
forwarding substitutions; the used snake CALLS_Instruction fragment was compared to
its baseline bytes and is identical. Process.CalledFunction, FunctionIdentifier,
Creator.Method and Process.State sources match their pinned baselines.

Golden candidates are derived from these actual sources:

- CALLS_Instruction: outer String then whitespace return; two argument syntaxes;
  lexical scan outside TRY, ReduceArguments protected, missing target protected,
  kind/restructure outside, ConvertArg protected, execution outside.
- Process.State.Return: successful IsJump return recursively returns the caller.
  This proves the ordinary six-mode success marker expectations; more involved
  recursive LOCAL/REF ownership remains in probes until real oracle capture.
- Original ConvertArg rejects excess count; snake ConvertArg retains fixed prefix.
  Non-character REF SetTransporter obtains the backing array without reading indices.
- StrFormCheckMethod evaluates its String before try, expands inside try and has no
  rollback of prior script writes. Rust intentionally limits catchability to Script
  categories; permission/resource/cancellation/host-contract equivalence is NOT claimed.

`probes.json` is NOT a runnable golden manifest. It names exact sources in
`probe-sources/*.erb.txt` (outside erb/, so loading normal fixtures does not include
known-fault probes). Pending items: original excess argument rejection phase/code;
8 TRY stage concrete inputs; recursive JUMP with LOCAL REF; original profile gate
normalization. A future authorized executor must create isolated probe projects,
perform each run and check result/side effects/terminal/diagnostics before promoting
an expectation. No ignored errors, invented broad code mapping, or expected 0 for
resource/permission/host failures is permitted.

These files do not alter either reference repository or the existing fixed fixtures.


## Final-draft additions (still unexecuted)

`pending/` contains isolated per-probe roots. Its nine new cross-checkpoint/parameter
cases have `candidateRustContract` for approved Rust assertions, but no `expect` or
claimed C# golden. Old TRY/extra/recursive-REF probes are also isolated; profile-gate
projects are separate from normal common/snake projects. `COMMON_REF` now explicitly
uses RETURN RESULT to preserve the observed RESULT:0 across the fixture call boundary.
The old source-derived two common and sixteen snake expectations are retained as
candidates, not execution evidence. Root ignored `2B-validation-final-draft/commands.json`
gives each selected case/profile exact future argv and expected/pending status.

The driver accepts the exact snake identity 4/4 while retaining 3/3 history and all
arithmetic/RNG/layout/save/service identity checks. Profile-gate cases leave the rejection
stage pending: a source diagnostic does not by itself prove that C# rejected `load`.
If C# loads and rejects the requested function at `run`, the existing runner captures both
responses; Rust's failed ProjectLoadReport and `compileError` remain a distinct stage in
comparison. This is a recorded difference requiring case-specific adjudication, not a
matched rejection. If C# actually rejects `load`, run.py stops before sending `run` and
retains the raw load response in evidence.json; neither process exit nor that stopped
capture proves parity. Do not relax the successful-load guard or infer an outcome from
localized text. No generic cross-engine load-rejection contract is enabled. No source
instrumentation is included in this fixture draft.

Pinned original source explains why the phase must be observed: ErbLoader marks an
InvalidLine with noError=false and a level-2 warning, but Process.Initialize assigns
that return value and can still begin TITLE and return true. EmueraConsole.Initialize
sets Error only for a false initialization result, otherwise it runs the title path.
These source facts do not establish the actual terminal state of any unexecuted gate
fixture. Keep original/snake captures separate, including the original two-argument
EXISTVAR gate, and adjudicate actual diagnostics and phases before adding expectations.
