# Fixed root collation input

`icu72-root.icu` is the full root collation dataset, including canonical closure,
extracted from the installed Wine ICU 72 package. It is not the ICU4X 1.1 pruned
collation export. Its original item name is `icudt72l/coll/ucadata.icu`.

| Input | SHA-256 |
| --- | --- |
| Wine `icudtl.dat` (31,079,968 bytes) | `9546414797ef032962d166fa2863b3e6e1053461cc19c7815a7d6d8672cb830d` |
| Root item (570,192 bytes, original offset 6,010,288) | `860fd450924b1e6d713aaa500a7d4e93327b46c7015f6b9e834fcf88542c110c` |
| Unicode 15.0.0 `UnicodeData.txt` | `806e9aed65037197f1ec85e12be6e8cd870fc5608b4de0fffd990f689f376a73` |

The binary format is little endian UCol 5 with a 32-bit UTrie2. Fixed options
are root, tertiary strength, normalization off, numeric off, and non-shifted
weights. The reader rejects other versions or options. No system locale,
runtime download, or mutable provider is consulted.

`../fcd_data.rs` contains first/last canonical combining classes of canonical
decompositions from Unicode 15.0.0, compressed into equal-value ranges.
Compatibility decompositions are excluded. These classes only control
contraction blocking; the collation reader does not normalize the input.

Data and algorithm notices are retained in the repository's Unicode/ICU license
notice. Source references:

- https://github.com/unicode-org/icu/tree/release-72-1/icu4c/source
- https://www.unicode.org/Public/15.0.0/ucd/UnicodeData.txt

Draft status: provider loader binding and runtime parity remain to be accepted;
the hashes above describe input identity, not behavioral verification.
