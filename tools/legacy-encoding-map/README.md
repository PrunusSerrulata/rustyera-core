# Legacy UTF-16 encoding width maps

This .NET 10 tool captures the code-page behavior used by original Emuera semantic
baseline `7b69ebd27378c03c32b6477b74901bfc3d33223c`. It has no package dependencies.
Use an existing .NET 10 SDK with installed framework packs and an explicit offline
restore configuration. Never substitute a downloaded or machine-global code-page
provider for the provider from the verified reference publish.

## Inputs and invocation

Compile `LegacyEncodingMap.csproj` in the task's isolated build/output directories,
as implementation data generation. The Windows reference provider requires a
matching Windows runtime (or Wine); it cannot be loaded into the macOS runtime.
Invoke the built executable
with exactly four arguments:

1. Absolute path to the verified reference publish's `System.Text.Encoding.CodePages.dll`.
2. That DLL's independently recorded full SHA256 from the reference build identity.
3. Absolute path to this generator's source directory.
4. Fresh, task-local output directory; existing directories are rejected.

The generator loads that exact DLL through a separate assembly load context,
checks its SHA256 before and after loading, and directly obtains its
`CodePagesEncodingProvider.Instance`. No implicit `Encoding.GetEncoding` provider
selection occurs. Keep the parent reference build manifest and actual oracle
provider identity with this capture; a matching package name is insufficient.

## Outputs

For each of CP932, CP949, CP936 and CP950, `cpNNN.tuples.bin` preserves all 65,536
UTF-16 units, including isolated high/low surrogates. Each ascending record is:

| Field | Encoding |
| --- | --- |
| UTF-16 unit | unsigned 16-bit little endian |
| Encoded length | unsigned 32-bit little endian |
| Ordinal roundtrip equality | unsigned byte, 0 or 1 |

Files contain no header, exactly 65,536 seven-byte records. Capture uses the
provider's default encoder/decoder fallback, exactly `GetBytes(c.ToString())`
then `GetString(bytes)`; best-fit behavior must not be replaced with `?` manually.

All raw lengths must be 1 or 2 before any final-width bitsets are emitted. On
failure, the tool retains raw tuples and a failure manifest, produces no final
bitsets, and exits nonzero. The four `cpNNN.final-width.bits` files each contain
8,192 bytes: bit `(unit & 7)` of byte `(unit >> 3)`, least significant bit first,
is zero for width 1 and one for width 2. Final width is selected-codepage length
when its roundtrip is exact, otherwise CP932 length when its roundtrip is exact,
otherwise the original selected-codepage emitted length.

`manifest.json` records provider assembly version/full identity/informational
version/MVID/SHA256, generator sources and binary SHA256, runtime and fallback
identity, binary schemas, and all output hashes. `manifest.sha256` records the
manifest hash without circular self-hashing.

## Product integration

Generate the isolated implementation data, install the four final-width bitsets into `crates/erabasic-data/src/legacy_encoding_maps/`.
Verify their provenance offline before product compilation and behavior acceptance.
Keep the generation manifest and raw tuples in task evidence; record the provider
identity, generator source hashes and committed bitset hashes in the map provenance
document. Do not commit machine-specific paths from the raw manifest.

The runtime only reads precomputed bits; it neither loads .NET nor allocates per
character. Existing scalar counting and portable FORM display widths remain
independent. Generation is evidence collection, not behavioral acceptance; verify
length, substring and find through the fixed oracle and both compatibility profiles.
