# Fixed ordinal casing notices

The shared casing algorithm and immutable tables were moved from the VM without
regeneration. They use .NET 8.0.28 ordinal casing and Unicode 15 / ICU72 simple
uppercase data. Retained upstream licenses and notices accompany this file.
The new comparator follows `OrdinalCasing.CompareStringIgnoreCase` in
[.NET 8](https://github.com/dotnet/runtime/blob/v8.0.0/src/libraries/System.Private.CoreLib/src/System/Globalization/OrdinalCasing.Icu.cs#L173).
It accepts Rust strings, so unpaired UTF-16 surrogates cannot occur.
