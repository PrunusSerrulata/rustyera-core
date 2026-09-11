# Snake Skiav13 batch A: CHKDATA

Fixed source fixture for snake semantic baseline `57170459b3d5ca175a1c57933058b569088bee0e` and original `7b69ebd27378c03c32b6477b74901bfc3d33223c`. Required Rust policies are snake 13 and original 3. Expectations are derived from the reference source; observations for this fixture have not been captured or validated.

The eight committed save headers are copied unchanged from `fixture-upstream-7b69-batch-a`. Slots 00/02 contain UTF-8 text headers and slots 01/03 contain uncompressed 1808 binary normal-file headers, with game code 4247 and versions 42/42/41/41 respectively. They end immediately after the description so CHKDATA must not parse a payload. Slot 99 is absent.

Slots 04/05 end immediately after incompatible version 41 (text/binary); slots 06/07 append an invalid description byte. Both oracles must reject the version before reading these descriptions. All eight cases now run against both oracles.

Each case checks statement and expression forms and then a missing slot. Both profiles are expected to write the header version on success or version mismatch, and zero for the subsequent missing slot. The manifest retains group `UPSTREAM_A` and entries `UPSTREAM_CHECK_0` through `UPSTREAM_CHECK_7` for the existing runtime probe. The unrelated ALS case and entry are removed; CSV/ALS source inputs are retained to preserve the existing load-diagnostic contract. A fresh process per case isolates those load diagnostics, which must be checked separately from operation diagnostics.

Run each case through the existing runtime and oracle capture paths, inspect its actual return values, side effects, terminal state, diagnostics and differential verdict, and only then advance. This fixture covers the upstream CHKDATA changes; product regressions separately cover wrong save kind, other game identity, malformed compatible headers, I/O failures and illegal arguments. Do not interpret these source-derived expectations as captured evidence.
