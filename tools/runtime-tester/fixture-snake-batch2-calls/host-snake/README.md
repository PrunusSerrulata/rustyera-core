# Dynamic Host host-snake observation candidates

Not executed, not goldens, not acceptance evidence. This minimal project contains 3 logical cases. These three cases require snake CALLSTR/STRFORMCHECK syntax and are not loaded by the original oracle. Common original behavior is observed separately in host-common.

Read `sourceDerivedCandidate` as an explanation only. `requests` deliberately contain no `expect` and the command list uses `pending_not_golden` with null expectedOracleSteps/candidateRustContract. The inherited loadExpect is the existing harness readiness contract, not a behavioral golden: unexpected load rejection preserves raw evidence and stops for root adjudication.

Explicit omitted final slots use `,,)`; a lone trailing comma is not an extra null slot in either fixed parser. String expression assignments use `'=`, including source strings; normal `=` is used only for integers here. Literal minimum Integer is spelled `(-9223372036854775807 - 1)`.

Both semantic baselines and exact original1/snake5 identities remain in cases.json. Execute only after root integration, fixture/binary input freeze and all affected static gates. Use the exact 21-row dynamic-host extension, one row at a time; retain actual values, side effects, diagnostics and final state before moving on. No real font/pixel or nonempty HTML measurement claim is made by the empty-string case.
