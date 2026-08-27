# Snake data client integration fixture

Authored for batch 1C; no execution is claimed. This is a self-contained small
RustyEra fixture for the TUI, Browser/WASM and Tauri real-client runners. It has
its own identity and does not change the separate COLUMNS oracle fixture.

The initial integer prompt follows `SNAKE_DATA_START`. Submit the visible input
`1` to run ALS/ERD → computed GETMETH → original resource reads → Data overlay and
recursive enumeration → MAP/XML/DT DEFAULT → GLOBAL save/mutate/load. A final
integer prompt follows `SNAKE_DATA_READY`. The script emits distinct markers for
every stage; only seeing READY is insufficient for acceptance.

Each runner must use its existing isolated project/OPFS copy. The original project
and its five resource files are read-only inputs. String SAVETEXT writes Data,
not the original Resource. GLOBAL is created and consumed within the same run;
ordinary FLAG:0 must remain 55 after the saved GLOBAL:0 value 7 is restored.
The loaded table keeps its existing row and default 12; its saved MAP/XML also
restore. A missing GLOBAL before the first save returns 0 without changing 66/55.

Browser/Tauri runners retain their five-second full DOM/runtime snapshot watchdog
and identical-snapshot failure rule. TUI observes stable waits through its real
RuntimeWorker/C ABI. Neither snapshots nor private state mutation replace visible
input. No SQL, game title, graph initialization, HTML pixel service or placeholder
for a missing real-game resource is involved.

The five resources are exact copies of the existing small owned inputs in
`../fixture-snake-data/plugins`. SHA-256 was computed while copying, not as a
runtime verification claim:

| Resource | SHA-256 |
|---|---|
| `plugins/data.txt` | `f78a3fbee4897e22f376b74372d12e701ba83b0961ba4816a358a34cd7295635` |
| `plugins/nested/child.txt` | `152544fe66e233715b5804252621927e94ef9d227c775c110125308d43e469ec` |
| `plugins/map.xml` | `1adcc04f4f2e121fb1cb44ade4416b3546453f973e871b7e22f5bc5941334416` |
| `plugins/dataset-schema.xml` | `aef51897dc625dad6273fe4bb91d44fe5b3cef951de33c004f0a31110ede7c99` |
| `plugins/dataset.xml` | `207d0d1190ca9cca18f4ff47d0fda04fe67ccba5cb3722db334d5ac12de0b976` |

## Runner entry points

Run only after the batch's single review and required static gates have passed.
Commands below are relative to the corresponding frontend worktree. Reuse the
installed Chromium and this group's verified WASM/native library; do not install
or download another browser. Each invocation creates fresh runtime storage.

```sh
# Web: existing Chromium, real native Firefox/Safari, and real Tauri respectively.
npm run test:game -- run --scenario tools/runtime-tester/scenarios/snake-data.json
npm run test:browser-compat -- --browser firefox --project ../rustyera-core/tools/runtime-tester/fixture-snake-data-clients --snake-data
npm run test:browser-compat -- --browser safari --project ../rustyera-core/tools/runtime-tester/fixture-snake-data-clients --snake-data
npm run test:tauri -- --spec tests/tauri/snake-data.spec.mjs --project ../rustyera-core/tools/runtime-tester/fixture-snake-data-clients

# TUI: replace the library path with this group's verified build artifact.
uv run rustyera-test run --scenario tools/runtime-tester/scenarios/snake-data.json --runtime-library /absolute/path/to/group-built-library
```

The expected markers express authored behavior assertions, not oracle results.
No command above has been executed as part of writing this fixture. XML null
serialization goldens and missing real-game resources remain outside this client
fixture; no synthetic files stand in for those missing resources.
