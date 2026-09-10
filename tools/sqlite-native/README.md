# Native SQLite 3.53.4 prebuild

This tool compiles the checked-in official SQLite amalgamation **before Cargo starts**.
It does not download sources, install tools, invoke Cargo, or change a Cargo lockfile.
`source/manifest.json` records the upstream archive provenance and SHA3-256 of both
extracted files. Every invocation verifies the C/header contents and header version/source ID.
The archive itself is not required at build time; its recorded digest is provenance, not a
claim that this entry point downloaded or reverified the original ZIP.

## Entry point and output

From the workspace root, a macOS ARM64 invocation is:

```sh
node rustyera-core/tools/sqlite-native/build.mjs \
  --target aarch64-apple-darwin \
  --output "$PWD/target/sqlite-native/aarch64-apple-darwin"
```

The output directory contains only these persistent products:

- `libsqlite3.a` (`sqlite3.lib` on Windows): the independently compiled static library;
- `sqlite3.h`: a byte-identical copy of the verified public header;
- `manifest.json`: source, builder, compiler, target, SDK, flags, environment and artifact hashes;
- `env.json`: the link environment variables below, plus Windows library/input paths.

The CLI writes one JSON result to stdout, including `inputs`, `artifacts`, `environment` and
`manifestPath`. Failures go to stderr with nonzero exit status. Intermediate C/object files
live in a unique sibling temporary directory and are removed in `finally`.
No command in this README has been executed as part of this implementation delegation.

```json
{
  "SQLITE3_LIB_DIR": "/absolute/output/directory",
  "SQLITE3_INCLUDE_DIR": "/absolute/output/directory",
  "SQLITE3_STATIC": "1",
  "SQLITE3_NO_PKG_CONFIG": "1"
}
```

These variables belong in the **Cargo process environment**, not in a parent crate's
`build.rs`: dependency build scripts run first. Use `rusqlite = 0.40.2` with
`libsqlite3-sys = 0.38.2` and no `bundled`, `bundled-windows`, SQLCipher or loadable-extension
features. That binding release's bundled SQLite is not this version. The runtime provider's
`identity::verify() -> crate::Result<()>` independently checks the actual linked engine using
a private in-memory rusqlite connection, `sqlite_version()`, `sqlite_source_id()` and
`PRAGMA compile_options`, without unsafe code. Required identity is exactly SQLite 3.53.4 and
`2026-07-24 19:02:57 bf7c7f30031888f4e796e429ab3978879485813aaca6f641c7b33e4e09459bcc`.
Every `SQLITE_DEFINES` option is required using the amalgamation's reported option spelling;
OMIT_DESERIALIZE, DEFAULT_FOREIGN_KEYS and HAS_CODEC are rejected. Identity failures return
`SqlErrorCodeV1::Unsupported`, with the actual mismatch and prebuild digest in the message.
The controlling implementation must register `mod identity` and invoke `identity::verify()?`
from Policy initialization before project SQL. Those integration files are outside this change.

The provider build script requires the four variables above, matching archive/header directories,
and a supported native host/target. It rejects provider-local bundled/sysbundled/SQLCipher/extension
feature escape hatches. Cargo does not expose transitive dependency features to this crate's
build script: a merged libsqlite3-sys bundled engine is rejected by the actual engine check at
Policy initialization, not inferred from environment variables or the expected source manifest.

`build.rs` runs the existing Node executable (`RUSTYERA_NODE`, default `node`) with
`build.mjs --verify-link-inputs <directory> <target>`. This read-only entry verifies manifest
version/source/required flags/system libraries and archive/header SHA-256; it does not probe
compilers, compile, repair a cache or download. It prints the combined manifest/artifact SHA-256.
The script tracks all four link variables, archive, header, manifest and verifier with Cargo
rerun directives. `RUSTYERA_SQLITE_LINK_IDENTITY` carries the digest into identity.rs diagnostics,
so replacing archive contents in the same directory changes Rust metadata and forces relinking.
An archive changed without its manifest is rejected. No registry-cache modifications are used.

## Web/Tauri integration

Only with **`RUSTYERA_NATIVE_SQL_PROVIDER=1`**, `rustyera-web/scripts/cargo-local.mjs`
invokes `prepareNativeSqlite()` before native build,
check, clippy, test, run, bench or rustc commands, then passes the returned environment to
Cargo (including npm's nested Tauri build). Metadata and formatting commands do not prebuild.
The wrapper's existing lockfile backup/recovery/restore flow remains intact. Without this
explicit r34 opt-in, the wrapper and Tauri cache do not import/prebuild/require native SQLite;
unsupported native targets do not obstruct the old path. Runtime does not yet depend on the
new crate; formal product enablement belongs to r35.

- `RUSTYERA_SQLITE_NATIVE_OUTPUT` selects the output directory. If omitted, the default is
  workspace `target/sqlite-native/<Rust target>`; it is independent of Cargo profile.
- `RUSTYERA_SQLITE_NATIVE_CACHE_ONLY=1` disallows compilation in `cargo-local`.
- `--target` or `CARGO_BUILD_TARGET` selects the Rust triple; otherwise `rustc -vV` supplies it.
- `RUSTYERA_SQLITE_CC` and `RUSTYERA_SQLITE_AR` select single executable paths/names. `CC`/`AR`
  are fallback choices, not shell command strings. No shell expansion or tool-install fallback.
- `RUSTYERA_SQLITE_CFLAGS_JSON` accepts an explicit JSON array of `-march=`, `-mcpu=` or
  `-mtune=` arguments only; `native` is rejected. Contract macros and output/target selection
  cannot be overridden through that array. Nonempty ambient CFLAGS/CPPFLAGS/LDFLAGS or include/
  library search overrides are rejected rather than silently ignored.

Ordinary wasm-pack and explicit WASM crate/target builds do not run this tool. The wrapper
removes inherited `SQLITE3_*`, `LIBSQLITE3_*`, `RUSTYERA_SQLITE_*` and
`RUSTYERA_NATIVE_SQL_PROVIDER` variables for those
children. The WASM dependency graph must continue to exclude the native SQLite backend.

When explicitly enabled, the official Tauri build-cache contract invokes this tool in
**cache-only mode**, even when
checking a potential normal rebuild. Therefore explicitly prebuild SQLite before entering
the official `--reuse-build` / `--require-reuse-build` workflow. It must not materialize a
missing library after calculating the pre-build contract. Keep the same target/output/tool
environment for the prebuild and Tauri command. The contract includes verified actual archive
and header hashes, all compile inputs, and the generated Cargo link environment; it also tracks
`core/tools/sqlite-native/` source inputs. A missing or changed SQLite cache stops before Cargo
or GUI startup. No version-string-only cache hit is possible.

For an explicit read-only cache check, append `--cache-only` to the entry-point invocation.
It still runs bounded compiler/rustc identity queries and reads/hashes files, but never compiles.
The module export is `prepareNativeSqlite({ target, output, environment, cacheOnly })`.

## Compile contract and platform boundaries

Windows x64 MSVC uses existing LLVM `clang` and `llvm-lib`, selected by
`RUSTYERA_SQLITE_CC` and `RUSTYERA_SQLITE_AR`. Set
`RUSTYERA_SQLITE_WINDOWS_TOOLCHAIN` to an absolute JSON file containing exactly
`schemaVersion: 1`, `sdkVersion` (a four-component Windows SDK version), and
`vcTools`, `sdkRoot`, `clangResource`. Directory values must match Node's
`fs.realpath()` spelling. The roots identify one MSVC Tools version, the Windows
Kits root, and one LLVM resource version respectively. No SDK discovery or download
fallback is performed.

The compiler uses explicit target, resource and include paths, and the dynamic
MSVC CRT. Rust `crt-static` is rejected. The manifest covers the five selected
header trees and six CRT/OS import libraries; cache and link checks revalidate
their contents, including added or deleted headers. `LIB` and
`RUSTYERA_SQLITE_WINDOWS_INPUT_ROOTS` from `env.json` must reach Cargo unchanged.
Windows GNU/arm64 and cross-compilation remain unsupported.

The baseline definitions in `SQLITE_DEFINES` follow compile-option strings present in the
installed official `@sqlite.org/sqlite-wasm@3.53.4-build1` artifact. This was a static artifact
inspection, not execution of `PRAGMA compile_options`; real provider conformance remains a
required downstream validation.

- Match WASM SQL-facing defaults: DQS disabled, recursive triggers/autovacuum enabled, cache
  default -16384, mmap disabled; **do not enable `SQLITE_DEFAULT_FOREIGN_KEYS`**. SQLite's
  ordinary foreign-key default remains unchanged.
- Retain SQLite's default serialize/deserialize APIs; never define `SQLITE_OMIT_DESERIALIZE`.
- Use `SQLITE_THREADSAFE=1` for native threads (WASM uses 0), and `SQLITE_TEMP_STORE=3` to forbid
  disk-backed temporary tables (WASM reports 2). These are explicit native safety differences.
- Include WASM's math/percentile/offset, FTS5/RTREE, session/preupdate, column metadata,
  DBSTAT/DBPAGE/bytecode/statement virtual-table and EXPLAIN unknown-function features.
  Compiling an extension does not authorize it: the provider's authorizer must still reject
  unauthorized virtual tables, ATTACH, external paths and other disallowed SQL.
- Disable loadable extensions, shared cache, deprecated and UTF16 interfaces as in WASM.
  No `-ffast-math`, no hidden feature auto-detection, no arbitrary user macro overrides.

Platform recipes are explicit:

| Native target                                    | Recipe                                                                                                                    | Verification status                              |
| ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| aarch64-apple-darwin                             | clang, verified native target, explicit `-arch arm64`, macOS SDK, deployment target default 11.0, `ar rcs`                | Implemented; not built/tested in this delegation |
| x86_64-apple-darwin                              | Same, explicit `-arch x86_64` on an x86_64 host                                                                           | Implemented; untested                            |
| aarch64-/x86_64-unknown-linux-gnu                | Native matching cc target, PIC object, `ar rcs`                                                                           | Implemented; untested                            |
| Windows x64 MSVC                                 | LLVM clang + llvm-lib, explicit SDK/CRT identity, dynamic CRT and `sqlite3.lib` output | Native host only; no cross compilation |
| musl, Android, iOS, other OS/ABI or cross target | Requires a separately verified sysroot/compiler/archiver recipe                                                           | Not implemented; explicit refusal                |

Requested target must equal both the Rust host triple and the running platform/architecture.
The compiler's reported architecture/OS must match too. macOS records the resolved SDK and its
SDKSettings digest; `SDKROOT`, `DEVELOPER_DIR` and `MACOSX_DEPLOYMENT_TARGET` participate in
identity. A host archive is never returned as a cross-target result. `inputs.systemLibraries`
is exactly `["m", "pthread"]` on Linux GNU and `[]` on macOS; the provider's build.rs emits
the Linux math/thread link directives. Loadable extensions are omitted, so this contract does
not add libdl. Native Linux linking still requires downstream validation before claiming support.

## Cache, failures and resource bounds

Cache hits compare the full recorded input object and rehash the static archive and header;
the header must also match the source header. They validate `env.json` against the expected
output directory. Inputs are rechecked after compilation before publication; manifest is
published last. A malformed/missing manifest is a miss, not a successful result.

Only a previously owned output or an empty/new output directory can be populated. A sibling
`.build-lock` prevents concurrent writers; cache readers refuse an active lock. A crash may
leave that lock: the owner must confirm the process stopped and remove that exact stale lock
before retrying. The tool never guesses that another process is dead or deletes its output.

Tool output is capped at 1 MiB; identity/archive commands have 30 s timeouts, C compilation
has a 300 s limit. Unix interruption sends TERM and escalates after 1 s. Windows
uses a bounded 5 s process-tree termination. A separate 6 s cleanup deadline
closes inherited output pipes and reports unconfirmed descendant termination if
normal child closure cannot complete.
The parent test runner must additionally enforce its remaining round budget. Compilation
refuses to start below 10 GiB available disk space. No downloaded archives or multiple expanded
source caches are retained by this tool.
