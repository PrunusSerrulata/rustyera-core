# Upstream 7b69 batch A

Fixed source fixture for original semantic baseline 7b69ebd27378c03c32b6477b74901bfc3d33223c and unchanged snake fc4fb21416768c17256d0e82f997e5f99c9bba91. Observations have not been captured yet.

`save00.sav` and `save02.sav` are UTF-8 text headers; `save01.sav` and `save03.sav` are uncompressed 1808 normal-file headers (little-endian magic/version/zero extra-header count, kind 0, game code 4247, version, 7-bit byte-length-prefixed UTF-16LE description). Versions are respectively 42, 42, 41, 41. All deliberately end immediately after the description: CHKDATA must never parse a payload. Both engines consume these exact bytes. Slot 99 is absent.

Each CHKDATA case checks statement and expression forms and then checks the missing slot to expose stale RESULT:1. Original writes the header version or zero; snake preserves the sentinel. ALS verifies first definition wins and parsing continues after the duplicate. The fresh process per case isolates duplicate-alias load diagnostics. Compare those separately from operation diagnostics.

Run one case at a time through runtime-tester snake-observations and the existing snake-compatibility-oracle/run.py with logical-output-only, then inspect its verdict before advancing. This minimal fixture does not cover all malformed-header, I/O, whitespace, or duplicate-index cases; product regressions cover those separately.

Slots 04/05 end immediately after incompatible version 41 (text/binary); slots 06/07 append an invalid description byte. These four cases are original-only early-return probes; snake retains its existing metadata path. All eight save headers are committed.
