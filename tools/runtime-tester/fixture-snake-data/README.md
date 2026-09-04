# Batch 1C columns, GLOBAL, and project data fixture

Status: authored only. No parser, analyzer, compiler, VM, runtime, frontend, or
reference test has run against this directory. Assertions below are prospective,
not recorded results. `cases.json` uses the existing version-1 request schema,
fixed seed 123456 and the two separately pinned reference baselines.

There are 27 independent cases in the `COLUMNS` group. `SYSTEM_TITLE` prints
`COLUMNS_READY` and waits, without running any case. Each case must load a fresh
project and isolated storage. An operation error never satisfies a load failure:
all cases, including errors, require successful loading.

## Inputs and runtime harness contract

Load `erb/base.erb`, `erb/columns.erh`, `erb/columns.erb`, `csv/GAMEBASE.CSV`, and
`csv/VarExt.csv`. Authorize these original resources in the project manifest:

- `plugins/data.txt`
- `plugins/nested/child.txt`
- `plugins/map.xml`
- `plugins/dataset-schema.xml`
- `plugins/dataset.xml`
- The five tiny filename inputs in `patterns/` (including non-BMP, NFC, mixed-case
  and literal brackets).

They are tiny RustyEra-owned inputs, not copies of game resources. No SQL,
executable plugin, fake missing game map, or full game initialization is included.
The original and snake reference CLIs operate on their own copied fixture roots.
The Rust runtime responder must retain separate Save, GlobalSave, Data, and
Resource namespaces. Resource bytes remain immutable; string SAVETEXT writes
Data; integer SAVETEXT writes Save. The snake runtime issues Data then Resource
only for NotFound. The original responder keeps its existing root fallback.
Backends must not guess or perform the snake fallback themselves.

All saves and mutations occur inside one `run` request. In particular,
`C1_CASE_GLOBAL_ROUNDTRIP` calls WRITE then READ in the same fresh process, so no
case depends on an earlier case's `global.sav`. `GLOBAL_MISSING` starts empty.
The named and numbered text case uses different filenames: the fixed reference
with `Use sav folder:NO` can physically colocate integer and string files, so it
would be incorrect to assert different contents at the same physical path there.
Namespace routing remains visible in the Rust request evidence.

## Behaviors

Twenty column cases exercise Integer/String and empty defaults, Int8/16/32
saturation, legal Int64 minimum, explicit omitted Null versus an unprovided
column, final non-null constraints, and no backfill of existing rows. Names are
evaluated column then table. Each DEFAULT is type-checked, evaluated, and applied
before the next one; errors preserve prior side effects and defaults. Missing
objects set the specified RESULT before failing. Value methods remove/recreate
columns or whole tables, and replace a table through XML, proving that the old
selection cannot target a replacement with the same name.

Explicit Null uses an interior omitted value followed by the `s, ""` pair. The
first Oracle run rejected a trailing omission because its parser discarded that
slot, leaving an unmatched column argument. The corrected fixture exercises Null
assignment without changing the engine's argument policy. First-run evidence is
retained separately; this edit is not a claim that those observations were rerun.

The XML-default roundtrip compares reconstructed values, not serializer spelling.
It includes quotes, Unicode, ampersands, CR, LF and TAB; TAB uses a string escape
because the reference UNICODE(9) reports a control-character warning and returns
an empty string. The XML Null case has no `expect`: it records schema, data and
pre/post Null observations before deciding the reference shape. The Rust unit
suite separately requires local Null/default persistence, including namespaced
`xsi:nil`. That requirement is not a predeclared .NET golden.

GLOBAL checks saved variables, ordinary-variable isolation, declared MAP/XML/DT
state, retained rows and column defaults. Missing GLOBAL retains existing values
and returns zero. Corrupt/profile-mismatched GLOBAL and atomic failure behavior
belong to runtime tests; they are not represented by a made-up reference save.

Resource cases read original assets, overlay one path with Data, count top-level
and recursive TXT enumeration without assuming platform iteration order, roundtrip
integer/named text, and load a small resource schema/data through MAP, XML and DT.
The enumeration count proves deduplication of the overlay path; path order is
covered by Rust/host tests, not a platform-specific oracle golden.

Row observations always use row position; reference TimePoint IDs must not be
compared with Rust deterministic IDs. Error-side-effect watches are requested,
but a runtime that disallows watches after a fault must report them as missing or
incomparable. Direct VM tests cover those side effects; a missing watch must never
be reported as a compared value.

## Prospective expectation sources

Both references are read-only, with semantics fixed at original
`26a35dc9334bb67590b96f7b8efbefbf199e391e` and snake
`fc4fb21416768c17256d0e82f997e5f99c9bba91`.

- `Emuera/Runtime/Script/Statements/Instraction.Child.cs`: DT_COLUMN_OPTIONS
  (original around 2620, snake around 2707), column-first selection, missing-object
  RESULT writes, static type checks and per-item mutation.
- `Emuera/Runtime/Utils/EvilMask/Utils.cs::DataTable.ConvertInt`: integer default
  saturation (original around 323, snake around 322).
- `Emuera/Runtime/Script/Statements/Function/Creator.Method.cs`: DT create/add/
  remove/row/XML object lifecycle; LOADTEXT/SAVETEXT/EXISTFILE/ENUMFILES and UNICODE.
- `VariableEvaluator.LoadGlobal` and `EraBinaryDataWriter/Reader`: save scopes and
  MAP/XML/DT extension serialization. Rust's atomic rejection of corrupt or wrong-
  profile GLOBAL is a separately documented intentional boundary.

Snapshot ticket lifetime, malformed bundle rejection, stale prepare/commit,
reloading generations, limits, and host security cannot be proven by these ERB
texts. They require the corresponding Rust and frontend execution tests before
batch 1C can be marked complete.

## Enumeration contract and intentional limits

The fixed C# implementations call `Directory.EnumerateFiles(dir, pattern,
option)` directly. They do not define a portable casing, normalization or
UTF-16/scalar wildcard layer. `column-resource-pattern-rules` therefore has no
oracle `expect`: it records actual results for casing, `?`, brackets, an empty
pattern, and NFC/NFD names on each pinned engine. Authored input is not evidence
that those platform rules were executed or that every result matches Rust.

Snake Data and Resource share a bounded portable matcher: NFC then Unicode
lowercase on pattern/name; `*` matches any scalar sequence, `?` one scalar,
`[]` are literal; absent/empty patterns do not filter. Inputs before and after
normalization are limited to 4096 UTF-8 bytes; NUL and more than 1,048,576 greedy
matching steps fail as InvalidData. The shared vectors are in
`../fixtures/snake-storage-patterns.json`. Empty-pattern handling retains the
existing Rust host behavior, and is not asserted as .NET equivalence.
Original product hosts retain their preexisting matcher rules and do not follow
new directory-link subtrees. Refusing escapes, cycles, malformed physical names,
and mid-enumeration corruption is an intentional safety boundary, not a reason
to silently broaden the original file set or fallback on arbitrary errors.

The case-local memory host models original enumeration as one selected source:
use an existing Data directory, otherwise the immutable project resources.
Directories created by writes remain present after deleting the last file.
Consequently `column-resource-overlay-enumeration` currently expects recursive
count **1 in original Rust**, **2 in snake Rust**, while the reference expectation
of 2 remains unchanged for both engines. This known original host difference is
not repaired by merging resources inside the test tool. The memory host is a
bounded, normalized-name simulator; it does not reproduce every original
frontend's filesystem, wildcard, permissions or symbolic-link behavior.
