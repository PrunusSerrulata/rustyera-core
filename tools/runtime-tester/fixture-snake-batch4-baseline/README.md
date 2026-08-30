# Snake batch 4.0 baseline fixture

This fixture freezes the reference-facing inputs for batch 4 without implementing
batch 4.1 or later behavior. It targets the fixed snake semantic baseline
`fc4fb21416768c17256d0e82f997e5f99c9bba91` and a 320 logical-pixel viewport
using the checked BIZ UDGothic font at 16 px with a 20 px line height.
The load snapshot observes that font for every case; run snapshots request text
presentation only for the HTML cases whose contract includes rendered lines.

`cases.json` is the executable case plan and route seed list. `contracts.json`
records source-derived defaults, coordinates, errors, resource assumptions, and
the Rust-owned save-v2 decisions that cannot be claimed as reference parity.
`oracle.json` is generated only from a complete fixed-reference capture and is
then verified offline.

The runner copies the already-committed two-pixel PNG from the service oracle and
the fixed font from the clean snake reference checkout into each disposable game.
Neither the reference checkout nor snake TW is modified. BBAS preflight records
the two missing map XML inputs as an external fact and never synthesizes them.

Process evidence belongs under the ignored sibling directory
`.worktrees/snake-compatibility/batch-4-work/4.0/`; it is not an implementation-log
entry because the whole of batch 4 has not ended.
