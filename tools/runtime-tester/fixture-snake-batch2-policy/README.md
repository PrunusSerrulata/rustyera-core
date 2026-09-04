# Batch 2A policy fixture

**Prepared, not executed.** This core-owned fixture is a request/expectation plan, not
an acceptance result. It does not modify the frozen batch-0 fixture or either oracle.
The batch's unique refactoring review and all authorized static gates must finish
before running these cases. Batch 2 has no aggregate test deadline; each runner
invocation still needs a bounded request/budget and the existing watchdog.

## Inputs and groups

`cases.json` contains 36 cases in the existing `arithmetic`, `RNG`, and `TOINT`
groups. The Rust observation adapter loads the corresponding `erb/*.erb`; the oracle
loads the complete fixture and waits at `base.erb` before each request. No new driver
group or modified game source is required. `emuera.config`, `setting.json`, the tiny
GAMEBASE CSV and pinned font metadata are copied from batch 0. The actual font is
not copied or downloaded. `UseNewRandom` remains false; this fixture does not claim
compatibility with the snake engine's alternative .NET Random path.

Required Rust identities are exact: original semantic/policy 1/1 with
`wrapping_i64_v1`, snake 3/3 with `snake_saturating_i64_v1`; both use
`sfmt19937`, state version 1. The comparison helper accepts that precise new pair
without changing historical 1/1 or 2/2 identity checks.

| Group | Coverage | Expectation source |
|---|---|---|
| arithmetic | Constant and variable add/subtract/multiply/negate; division and remainder by zero; MIN/-1 errors; prefix/postfix return and storage; four UNCHECKED calls | Fixed `OperatorMethod.cs`, snake `SafeArithmetic.cs` and `VariableToken.PlusValue`; batch-0 arithmetic evidence for inherited cases |
| arithmetic | XML multi-target child count; String `>=`/`<=` at equality and ordered values | Both fixed `Creator.Method.cs:XmlAddNodeMethod` and `OperatorMethod.cs:GreaterEqualStrStr/LessEqualStrStr` |
| TOINT | Valid integer, decimal, empty, invalid text, oversized integer, variable argument, hexadecimal, negative, invalid binary digit | Batch-0 TOINT evidence; fixed `ToIntMethod` and integer-reader source |
| RNG | DUMP/INIT replay, rejected invalid state index, repeated RANDOMIZE | Batch-0 raw RNG vectors; fixed variable evaluator reseeding code |

### NOISE game call sections

Read-only source locations, relative to the main multi-component workspace:

- Snake TW: `games/eratw-sub-modding/ERB/COMMON.ERB:2990`, with XOR/shift at
  2999–3000, the nested UNCHECKED polynomial at 3001, and unchecked scaling followed
  by ordinary division at 3002. NOISE2/NOISE3 call it at 3015/3028;
  TEST_PRINTNOISE calls it with scale 100 at 3041.
- Original eraTW: `games/eraTW/ERB/COMMON.ERB:2366`, with corresponding XOR/shift
  at 2372–2373 and ordinary wrapping polynomial/scaling at 2374–2375.
  NOISE2/NOISE3 call it at 2386/2397; TEST_PRINTNOISE uses scale 100 at 2408.

The four `noise-{plain,unchecked}-{small-scale,wide-scale}` eval cases reconstruct
only that arithmetic graph: seed XOR, signed shift, nested multiply/add, a 31-bit
mask, scale multiplication, then truncating signed division. Inputs are seed 0 /
scale 100 and seed -5 / scale 8589934592. The second exercises scale overflow as
well as polynomial overflow. No whole game function, headers, lazy index or resource
data is copied. Ordinary expressions under the original profile and UNCHECKED under
snake must preserve the wrapping result; ordinary expressions under snake use the
saturating policy and have their separate oracle expectations.

Expected numbers are derived from these fixed operation graphs, not from running
Rust or either oracle. The source hashes and intermediate arithmetic are retained in
the ignored work group's `batch-2-work/2A/noise-source-derivation.json`. This is a
minimal arithmetic call section, not a claim that the full game NOISE2/NOISE3 or
the game project compiled or executed successfully.

New source-derived expectations are explicitly marked `not_executed`. Reference
sources are pinned by `semanticBaselines`; wrapper commits must be recorded by the
actual run. Existing raw RNG evidence is retained in the shared work group's
`batch-0-work/oracle-evidence-original-rng/evidence.json` and
`batch-0-work/oracle-evidence-snake-rng-driver-rerun/evidence.json`.

## Expected differences remain differences

- At seed 123456 the original oracle's DUMP/INIT sequence is
  `192905, 520548, 192905, 520548`; fixed snake is `192905, 520548, 0, 0`.
  The approved Rust SFMT policy uses the first vector for both profiles. Keep the
  `rng_roundtrip` assertion and snake `pinned_oracle_rng_state_loss` finding;
  never replace the oracle's zeros with the intended Rust values.
- Both fixed oracles accept `RANDDATA:624 = -1; INITRAND`. Rust rejects the invalid
  state under both profiles. Preserve actual error/termination and any available
  watches; this fixture alone cannot inspect RNG state after that fault. Atomic
  rejection and unchanged subsequent RNG output require the direct VM tests.
- Fixed original lacks UNCHECKED names. Rust's additive API succeeds under both
  profiles, while the original oracle rejects the names.
- Fixed original XML_ADDNODE moves one node between selected parents and leaves
  one child. Snake and existing Rust clone and leave two. Existing Rust String
  comparison also differs from the original's incorrect `c < 0` implementations.
  These original-profile differences are not repaired or hidden in this batch.

`knownRustDifference` and `rustAcceptance` are documentation metadata. Neither the
Rust adapter nor `run.py` reads them to manufacture observations or mark a result
passed. The supervising executor must check actual Rust typed values, side effects,
terminal state and diagnostics against `rustAcceptance` for those registered
differences. Failure watches may be unavailable; do not infer their values from the
script or claim fault-state memory was inspected. The compiler/VM unit tests cover
these boundaries independently.

Warnings are not asserted by localized output text. Preserve source, diagnostic
code/context and raw responses. Cross-engine warning/error schema differences stay
`incomparable`; extra Rust warning text must not turn a differing value into a pass.
Runtime arithmetic warnings also differ in presentation: fixed snake inserts them
into console history, whereas Rust preserves the existing independent diagnostic
channel and does not insert diagnostic lines into script history. This affects
history queries, not just diagnostic encoding. Preserve the exact per-case output
difference and its raw `different` verdict; never remove warning lines to obtain
parity. Rust warning acceptance uses its stable code/context/source and observable
values; oracle output is retained separately and is not parsed as a diagnostic code.

## Execution after review and static gates

Use the repository's test skill and its designated test executor. Build the tester
once through its official entry point after freezing inputs, record its source and
artifact identity, then use that verified executable in strict cache-only mode.
From the core worktree, capture one selected case under each explicit profile:

```sh
/verified/runtime-tester snake-observations \
  --fixture tools/runtime-tester/fixture-snake-batch2-policy \
  --profile emuera.skia.snake --case arithmetic-variable \
  --output /isolated/batch-2A/arithmetic-variable-rust-snake.json
```

Then use the matching current executable, full wrapper SHA, isolated Wine prefix,
existing pinned font and a new evidence directory:

```sh
python3 tools/snake-compatibility-oracle/run.py \
  --fixture tools/runtime-tester/fixture-snake-batch2-policy \
  --oracle snake --exe /verified/Emuera.ReferenceCli.exe \
  --wrapper-sha FULL_WRAPPER_SHA --wine wine --wine-prefix /isolated/snake-prefix \
  --font-file /verified/BIZUDGothic-Regular.ttf --logical-output-only \
  --case arithmetic-variable \
  --rust-evidence /isolated/batch-2A/arithmetic-variable-rust-snake.json \
  --output /isolated/batch-2A/arithmetic-variable-snake --budget-seconds 300
```

These paths and SHA are placeholders, not verified executables or evidence. Use
`--oracle original` with its corresponding Rust profile, executable and prefix for
the other comparison. After each case, validate the actual results and inspect its
comparison before starting the next case; stop on the first unregistered difference.
Group selectors are available for authorized targeted collections but do not waive
that case-by-case acceptance order. Data-only observations do not verify font pixels.
Do not recapture completed cases merely to regenerate reports.
