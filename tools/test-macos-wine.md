# macOS reference oracle environment

`test-macos-wine.sh` defaults to the frankea/Whisky Wine runtime at
`$HOME/Library/Application Support/com.franke.Whisky/Libraries/Wine/bin`.
It calls Wine directly to preserve the persistent NDJSON stdin/stdout protocol;
`whisky run` is not used. All four tools (`wine`, `winepath`, `wineboot`,
`wineserver`) must exist in that directory before any build or fixture setup.
Set `EMUERA_WINE_BIN` to a different Wine bin directory to override the runtime.
A missing tool fails immediately instead of falling back to another Wine on PATH.

The existing `.wine-prefix/emuera-reference-cli` remains the default prefix.
`EMUERA_REFERENCE_WINE_PREFIX` still overrides it. Back up an old prefix before
first opening it with a different Wine runtime, and never run two runtimes against
one prefix concurrently. Concurrent tasks need independent writable prefixes and
fixture/output directories.

The script defaults `DOTNET_SYSTEM_GLOBALIZATION_USENLS` to `1` to avoid ICU
initialization failures from legacy Wine prefix DLLs. Set it to `0` explicitly
if an ICU configuration has been independently verified. NLS changes the .NET
globalization backend; oracle smoke does not establish complete culture-sensitive
NLS/ICU equivalence.

Existing build, publish, work-directory and timeout overrides remain unchanged.
For a previously verified self-contained publish directory, set
`EMUERA_REFERENCE_PUBLISH_DIR` and `EMUERA_REFERENCE_SKIP_BUILD=1`. Do not reuse
artifacts after their product inputs change. No local migration output path is
hardcoded in this script.

The snake oracle's `emuera-reference-cli/tests/test-macos-wine.sh` uses the same
`EMUERA_WINE_BIN` and NLS variables, its existing `WINEPREFIX` override, and the
independent `.wine-prefix/emuera-selfmodified-cli` default. Standalone Python
oracle drivers continue to accept explicit Wine/prefix arguments; when invoking
them directly, supply the same runtime PATH and globalization environment.
