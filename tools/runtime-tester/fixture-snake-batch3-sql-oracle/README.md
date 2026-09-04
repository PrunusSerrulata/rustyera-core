# Snake batch 3.0 SQL oracle fixture

This fixture freezes the small SQLite behavior surface needed by batch 3. It is
owned by RustyEra and must only be run from a disposable copy: the snake Emuera
reference process can create SQLite files and sidecars.

`cases.json` is the machine-readable execution plan. `oracle.json` is populated
only from the fixed snake reference CLI and contains the normalized golden plus
the snake TW resource baseline. Raw envelopes, diagnostics, watchdog snapshots,
and temporary database inspection stay in the ignored `batch-3-work/3.0/`
evidence directory.

The fixture deliberately records reference behaviors that RustyEra will not copy
literally:

- reconnecting an existing logical name with a different connection string is
  accepted by the snake reference and keeps the first connection;
- `SQL_CONNECT` does not enforce the batch-3 logical-name grammar or safe
  connection-string subset;
- SQL and MAP XML paths are unrestricted reference inputs;
- `SQL_IMPORT_MAP_XML` interpolates the table identifier without validating the
  batch-3 identifier grammar.

Batch 3 applies the safe Resource/Data, logical-name, connection-string, path,
and table-identifier contract defined by the implementation plan. These are
intentional safety differences, not reference equivalence claims.

The BBAS preflight is assembled by the runner from the exact
`CREATE_BBAS_DATABASE` source and the two present XML resources in the selected
snake TW checkout. It does not create either missing `bbas_map_*.xml` file and
does not write to the game checkout.

`--case` may be repeated for ordinary case IDs and for the shared
`bbas-preflight` and `snake-tw-resources` checks. Selecting every ordinary and
shared ID produces a complete projection, which is useful for a targeted
recovery capture after a common reference executable fix.

Typical macOS capture command (run through the required test-only agent):

```sh
WINEPREFIX=/absolute/workspace/.wine-prefix/emuera-selfmodified-cli \
  python3 tools/runtime-tester/snake_batch3_sql_oracle.py \
  --wine wine --winepath winepath \
  --exe ../emuera_lazyloading_selfmodified_version/emuera-reference-cli/bin/smoke-win-x64/Emuera.ReferenceCli.exe \
  --snake-tw-root ../games/eratw-sub-modding \
  --mode capture --output ../batch-3-work/3.0/oracle-capture.json
```

After `oracle.json` has been reviewed, verify the already captured evidence
offline. This does not start the reference process again:

```sh
python3 tools/runtime-tester/snake_batch3_sql_oracle.py \
  --mode verify --capture ../batch-3-work/3.0/oracle-capture.json
```

Verification compares the complete stable projection exactly. Human-readable
SQLite exception text, output text, diagnostics messages, and stderr remain raw
evidence and are never the pass/fail oracle.
