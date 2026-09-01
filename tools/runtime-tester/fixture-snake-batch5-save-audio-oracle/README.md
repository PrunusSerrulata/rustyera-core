# Snake Batch 5 Save and Audio Oracle

This fixture freezes the Batch 5.0 reference contract. It is copied to a
disposable directory before every run and must never write to either reference
repository or to the snake TW checkout.

The headless reference CLI covers Binary, ERAZIP/GZip, and Text ordinary and
GLOBAL saves and integer/string scalar and 1D/2D/3D arrays plus character data.
Binary and ERAZIP also persist Map/XML/DataTable objects; the fixed Text writer's
actual omission of those extension objects is a separately asserted outcome.
The fixed snake reference also refuses its own ordinary Text output with
`Unexpected end of save data`; that result is frozen as a reference limitation,
while original-reference Text and GLOBAL Text outcomes are recorded separately.
Separate damaged inputs record Float,
unknown-tag, truncation, bad-header, and decompression-limit behavior. Float is
accepted by the fixed reference, but remains an explicitly unsupported target
for RustyEra Batch 5; unknown and oversized inputs remain refusal cases.

The CLI sound factory is intentionally a no-op. It freezes language signatures
and parameter-level return codes only. `gui_audio.erb`, a generated WAV, and an
isolated Wine GUI run freeze actual pause/resume/stop, channel allocation,
position, rate, volume, and pitch-flag behavior. The ERB API has no Seek action.

The GUI must use a task-isolated Wine prefix. Necessary components may be copied
from `~/.wine` without following symlinks, but the source prefix is never used
directly and `drive_c/eratw-sub-modding` is excluded. A persisted symlink audit
must prove that no copied link resolves to snake TW or the user's daily game
directory before launch.

The NAudio build also requires `SoundTouch_x64.dll` beside `Emuera.exe`. Copy it
as a regular file from the fixed local snake client/provider set, never through
the daily-game symlink, and persist its source and target SHA-256 identities in
the GUI process evidence.

Raw captures, generated saves, the isolated GUI project, and Wine prefix live
under `.audit/batch-5/5.0/` and are ignored. `oracle.json` contains only the
stable normalized golden and source/input identities needed to reproduce it.
