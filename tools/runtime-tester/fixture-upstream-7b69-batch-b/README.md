# Upstream 7b69 batch B fixed inputs

Four independent game roots select JAPANESE, KOREAN, CHINESE_HANS and CHINESE_HANT
through `Default ANSI encoding`. Each binds original semantic SHA
`7b69ebd27378c03c32b6477b74901bfc3d33223c` with Rust policy 3, and snake semantic SHA
`fc4fb21416768c17256d0e82f997e5f99c9bba91` with Rust policy 12.

Each original-only `UPSTREAM_B` full case observes traditional string lengths, scalar/private-use
and nonroundtrip mapping boundaries, emoji, substring bounds, 32-bit index wrapping,
find bounds, BMP U-function behavior, and numeric-conversion guards. Watch names are
listed in `watchMeaning`. The ERH constant and runtime variable use the same input
`A😀ｶ`; their observed lengths and RESULT:51 from STRFORM dynamic expression compilation
must all equal 4 under the original profile. Mapping
values are not guessed in fixture expectations: raw watches remain authoritative and
are compared separately against each engine's fixed reference.

Each root also contains an original-only `intentionalUtf16Replacement` case.
`SUBSTRING("😀", 0, 1)` creates an isolated UTF-16 high surrogate in the reference;
comparison with U+FFFD therefore returns 0. Rust materializes valid UTF-8 U+FFFD and
returns 1. Only this exact declared watch difference may be registered as the
intentional representation difference; preserve the raw differing observations.
The case exports only an integer, never invalid Unicode in JSON.

The probe supplies its existing SYSTEM_TITLE wrapper; these files intentionally
contain no duplicate SYSTEM_TITLE. Select the `upstream-snake-stable-LANGUAGE` case for snake captures:
the reference driver respects case `allowedOracles`, while the Rust probe should be
invoked with the selected case ID explicitly.

The snake-only `UPSTREAM_SNAKE_STABLE` case asserts 14 old-policy observations:
ASCII/common CJK lengths, positive multibyte boundaries, zero-length and
out-of-range substring, valid/end find indexes, BMP U functions, and ASCII numeric
conversion. Negative indexes, int32 wrapping and differing non-ASCII best-fit
mappings belong only to the original upgrade case. This oracle scope follows the
authorized compatibility boundary; it does not assert full snake Unicode mapping
equivalence. Emoji is excluded from the snake oracle scope because its .NET
whole-string fallback width has not been established here; scalar emoji behavior
remains covered by data/VM regression tests. Shared policy selection and preservation of scalar mapping behavior
are additionally covered by data/VM unit regressions.
Input itself is not acceptance evidence; final results belong to the implementation
record (输入本身不构成验收证据，最终结果见实施记录).
