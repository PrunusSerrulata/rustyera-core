# Fixed comparison notices

The private comparison implementation and fixed data use these upstream sources:

- Unicode ICU 72.1 collation algorithms and Unicode 15.0.0 data: Unicode license
  in [ICU4X-LICENSE](ICU4X-LICENSE), including the ICU-derived code notice.
- .NET 8.0.28 OrdinalCasing and globalization comparison/search rules: Microsoft
  MIT terms in [DOTNET-LICENSE.TXT](DOTNET-LICENSE.TXT), with retained
  [upstream third-party notices](DOTNET-THIRD-PARTY-NOTICES.TXT).
- The `icu_collections` UCharsTrie reader and `zerovec` are normal Cargo
  dependencies; their distributed licenses are retained by their upstream crates.

No .NET, Wine or ICU executable is embedded. The immutable root collation item is
Unicode data; its precise source identity and format are recorded in
[the data note](../../src/compat_collation/data/README.md).
