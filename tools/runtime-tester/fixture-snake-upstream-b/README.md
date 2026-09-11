# Snake upstream B fixtures

Fresh RAND inputs for snake semantic baseline `57170459b3d5ca175a1c57933058b569088bee0e` (Rust policy 14) and original policy 3. Fixed seed 123456 exercises valid SFMT replay, variable/function clamp without consumption, parenthesized nested indices, signed dynamic FORM, source omission rejection, and original argument rejection.

Expected values are source-derived assertions, not execution evidence. Run each selected case through the runtime probe, the matching reference engine, and `validate_rand_observations.py` before proceeding to the next case. Never dispatch a case outside its `allowedOracles`.

Snake warnings are runtime LogOnly diagnostics, deduplicated per VM session and per RAND entry. The reference emits console text and keeps process-static deduplication. The validator parses the fixed warning/error resource format into entry and numeric argument, checks Rust origin and identity independently, and retains the raw output difference. It does not equate the two diagnostic schemas or convert a raw difference to a match. Snapshot, hot reload, separate fibers, and CompatiRAND boundaries are covered by VM tests.

The source checker rejects an omitted first argument even though the reference evaluator retains an internal null-to-zero branch. The snake checked-FORM case exercises this boundary. Original policy 3 has a pre-existing source-omission acceptance gap; the shared valid case uses an explicit zero lower bound and does not claim that gap is fixed.

RAND has exactly one variable index: `RAND:(LOCAL:3)` is legal, while `RAND:LOCAL:3` is rejected by both references. Generic name-table index reassociation is not applied to RAND. The checked-FORM case also covers excess-index rejection.

The original and snake roots isolate profile-specific checked-FORM source. `base.erb` supplies only the reference title wait; the runtime probe reuses its existing generated title wrapper.
