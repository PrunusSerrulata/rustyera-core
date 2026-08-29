# CALLSTR Restructure and dynamic Native follow-up

New minimal cases for gaps discovered after the first 2B matrix. These are not
accepted observations or C# goldens. `sourceDerivedCandidate` is explanatory only;
the runner must capture, decode, assert actual values/effects/faults and adjudicate
each pair before starting another case. Prior matrix evidence remains unchanged.

The project deliberately has no static Native IMPORTS helper. Every REPLACE,
STRLEN, STRFORM, ABS, MAX and SUBSTRING call is inside runtime text. The OMITTED
helper has a normal caller write before CALLSTR; outer STRFORMCHECK must retain it
when the reference's omitted-argument unique restructuring fails. Internal bad
bytecode/resource/provider faults are not covered by that catchability assertion.

CALLSTR syntax is snake-only. Original syntax refusal is covered separately by the
existing exact original profile-gate fixture; do not reinterpret it as a supported
original CALLSTR execution. Both oracle baselines remain pinned in cases.json.
All source-derived expectations here require first actual reference capture.

Source basis: fixed snake CALLS_Instruction, VariableTerm.Restructure,
UserDefinedMethodTerm.Restructure/ConvertArg, FunctionMethodTerm.Restructure and
Creator.Method REPLACE/STRFORM/STRFORMCHECK. Their fixed-file hashes are listed in
root ignored 2B-shared-restructure-draft/source-evidence/manifest.json; the repository
fixture source-manifest.json records both semantic baselines and wrapper deltas.
