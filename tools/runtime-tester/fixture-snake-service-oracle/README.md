# S04 fixed-reference and real-client observations

Prepared inputs only. Nothing in this directory has been executed or accepted.
`cases.json` follows the existing version-1 oracle request format. All cases are
marked `observation`; their partial `expect` fields are source-derived assertions,
not recorded golden results. Unspecified watches must still be captured. Missing
or fault-inaccessible watches are incomparable, never substituted from `expect`.

The references are fixed at original
`26a35dc9334bb67590b96f7b8efbefbf199e391e` and snake
`fc4fb21416768c17256d0e82f997e5f99c9bba91`. Use seed 123456, the pinned BIZ UDGothic
font from the manifest, actual FontSize 16, line height 20 and configured window
width 320. Capture actual drawable width; configured outer width is not the
drawable width. Original uses TEXTRENDERER; snake uses its real SKIASHARP path.
The wrapper's `unverified-installed-source` font status is not proof that the
installed font bytes equal the supplied file. Browser/Tauri must record their
actual font loading/readiness and fallback outcome separately.

## Entries and observations

Start a fresh session for every case. In a real client, enter its one-based case
number at `S04_ORACLE_READY`; the title calls the exact entry named in `cases.json`.
The reference CLI uses `op=run` with that entry and unchanged watch list. The menu
is an operator adapter, not a replacement for the entry or an execution result.

| Case | Observation |
| --- | --- |
| empty-lazy | Empty STRINGLEN still evaluates its flag; empty STRINGLINES skips width. |
| first-row-half-units | First displayed row only; default/zero flag half-unit conversion and negative nonzero flag pixels. |
| lines-width-evaluation | Width is evaluated once per split, including explicit empty rows; RESULTS remains untouched. |
| substring-explicit-break | Exact source-preserving head/closing and reopened tail around `<br>`. |
| substring-unicode-cuts | Literal non-BMP, combining mark and paired numeric surrogate entities; exact raw results. |
| entity-measurement | Named/numeric ampersand equality; nbsp maps to ASCII space. |
| substring-amp-error | Whole unescape followed by individual-character parsing rejects a lone ampersand. |
| late-parse-error-before-flag | Invalid later row is not hidden by an already known first-row width; flag side effect remains absent. |
| substring-lazy-error-frontier | A cut can stop before an invalid suffix; observe the actual frontier. |
| style-and-position | Shaping, style parts, button positions, no-wrap and shape width; actual pixel observations. |
| missing-image | Undeclared sprite AltText is measured in default regular font, even inside bold. |
| canvas-two-pixels-revision | Never-mounted 2x1 canvas, immediate red/blue and later green samples; alpha128 replacement, transparent clear and unchanged neighbor; explicit/default image sizes. |
| canvas-invalid-dimensions | Width zero must follow the reference exception path. |
| canvas-outside-x | Positive x overflow and uncreated canvas return -1. |
| full-layout-later-rows | Later rows and indivisible missing-image fallback still execute; capture any layout error. |
| file-two-pixels | Decode the authored PNG, capture both pixels/alpha and image sizing. |
| length-int32-units | Reference int32 unit observations; real frontend giant-slot resource rejection is an explicit safety difference. |

The separate `fixture-snake-service-no-progress` is a watchdog hazard and must
never join this default case set. See its README before running it.

## Source-derived rules and limits

Both references register HTML_STRINGLEN, HTML_SUBSTRING, HTML_STRINGLINES,
GGETCOLOR and MOUSEX/Y/B. These APIs are **not snake-only rejections**. Unsupported
TUI service versions are frontend capability errors, not original-engine API
rejection. Do not conflate the two.

`HtmlManager.HtmlLength` performs the full Html2DisplayLine conversion, then sums
the widths of the first row's buttons. It does not return a DOM bounding-box
maximum. `HtmlStringLenMethod` evaluates the flag after length computation. Half
units first double `L` with unchecked int32 arithmetic, divide toward zero by
FontSize, then add 1 for original `L >= 0` or subtract 1 for original `L < 0`
when the wrapped numerator has a nonzero remainder. The familiar
`sign(L) * ceil(abs(2*L)/FontSize)` applies only when doubling does not overflow;
the rounding direction must not use the wrapped numerator's sign. The small
positive-length relation in the fixture uses FontSize 16. Different providers can legitimately return different
pixel lengths. Pixel values and negative-layout cases remain explicit
observations; equality of API names is not a layout parity claim.

Menu 17 / `S04_CASE_LENGTH_UNITS` uses existing `space` shapes at explicit
`±1073741824px` inside `nobr`: these powers of two are exact float32 widths and
the reference space part does not allocate an image. Observe reference pixel,
default and nonzero-flag values independently. Browser/Tauri deliberately reject
the first positive slot above the 32768px projection limit with `resource_limit`,
before mounting or executing the unit conversion. Preserve this safety difference
and actual fault trace; missing fault watches stay blocked. This case cannot prove
real-client unit conversion or be called a match. Do not enlarge or bypass the
projection bound to make the fixture pass. Separate core helper/planner tests
exercise int32 wrapping, sign-based rounding and accumulated layout overflow;
their slot-readiness responses are synthetic, not frontend measurements.

`HtmlSubString` first unescapes its working source. It then measures individual
UTF-16 code units with HtmlLength. Therefore `&amp;` can become a bare `&` and
fail a second parse. The source-derived amp-error assertion preserves that error;
it does not force a successful plain-text reconstruction. Rust's required legal
Unicode scalar cuts can differ from a reference UTF-16 midpoint. Save raw JSON
bytes and code units before decoding/normalizing; do not turn an unpaired
surrogate into U+FFFD and report a match. If the wrapper itself cannot serialize
the result, retain that instrumentation failure as incomparable.

`ConsoleImagePart` defaults height to FontSize, not natural height. A loaded 2x1
sprite at FontSize 16 has width 32 unless explicit `height='1px'` makes it 2.
Missing sprite uses normalized AltText/default font. An existing resource that
fails read, hash validation or decode is not Missing and cannot become zero width
or fallback success. The long fallback in later-rows may be indivisible and
fault; no expected success is authored for that case.

Canvas color assertions use unsigned ARGB in an Integer. The file's semitransparent
pixel is observation-only because decode/premultiplication must actually be seen.
GCREATEFROMFILE defaults to `resources/service-pixels.png` via ContentDir when
called with `"service-pixels.png"`; relative=1 would instead depend on the reference
process CWD (the isolated per-case game directory). Resource
requests and load failures must be retained. Negative-y bounds are deliberately
not asserted: both fixed GraphicsGetColorMethod versions duplicate their x<0
check and omit a direct y<0 check.

## Pointer entry

Menu 98 invokes S04_CASE_POINTER. Click the actual visible button with script
value 41 at its input wait. Record pointer position in logical viewport pixels
and hover script value. MOUSEB returns **String** (`"41"`), not a physical mouse
button number or the DOM caption. Its value is stored in RESULTS:70. Sample x/y
and hover via the real service, including client bottom-origin MOUSEY mapping.
Reference CLI headless input injection does not establish an actual OS pointer
hover, so this entry has no fabricated deterministic headless oracle case.

## Not covered by just loading this fixture

Native Firefox/Safari and Tauri, stale epoch/request/revision replies, async
decode cancellation, viewport scroll/resize/leave/blur, replay resource limits,
declared corrupt resources, and no mounted canvas require the real-client driver
actions described in the staging `CAPTURE_PROTOCOL.md`. Core memory-host or TUI
execution cannot supply real HTML measurements. No fixed-width fallback backend
is permitted. Mock provider unit tests remain separately labelled synthetic.

The Python oracle driver requires Rust evidence first and currently
accepts only `profile.services=[]`. That compatibility-policy field is distinct
from frontend negotiated capabilities; a real frontend can legitimately retain
the empty policy list while advertising services. Its comparison fields do not
include service request/reply traces. The existing Rust `snake-observations`
harness does not provide HTML/canvas/pointer services. Real capture entry points
are `scripts/snake-service-oracle-chromium.mjs`, native browser compatibility
and Tauri runners with `--snake-service-oracle --capture-config ... --case ...`;
the core `tools/snake-compatibility-oracle/frontend_capture.py` adapter validates
the actual packets and typed watches before comparison. These sources are not yet
executed acceptance evidence. Menu17 and fault-inaccessible watches can produce
nonzero capture status and must retain their raw trace; adapter success is not a
comparison verdict. Freeze effective fixture/configuration identities before both
engine runs; never edit a capture to make a validator accept it.
