# Snake service lifecycle client fixture

Source fixture for real Browser/WASM and Tauri/WebView tests. Seed123456; clock
2026-01-01T00:00:00Z. Canvas741 is never printed. Immediate red/blue queries and an HTML query
following output retain marker `4294901760/4278190335/1`, without platform-specific text goldens.

Six visible-input stages observe button41, a nonbutton point, outside the viewport, actual resize,
PageUp scrolling, and a trusted native window blur followed by return to the original input without
a new pointer event. The last stage must read `0/0/` (String MOUSEB empty). The label is deliberately
not the script value. The 120 plain rows supply scrollable content without large assets.

## Pending service races

Startup input90 loads the bounded `resources/lifecycle-gate.png` into canvas743 and immediately
calls GGETCOLOR. No canvas is mounted in the display. The real-host runner enables a test-build-only
loopback resource stream for exactly this resource ID, length and SHA256. Normal bridge.readResource
still performs its existing authorization and content checks, then the test hook independently
verifies SHA256 against the server's unchanged fixture bytes. Production builds cannot configure it.

The server sends only the original PNG signature and IHDR (33 bytes), withholding all IDAT data.
The unchanged renderer calls the real HTMLImageElement.decode(). Its actual start, abort and final
settlement are bounded read-only observations; no decoder, measurement, service reply or pixel is
substituted. This observes physical decoding waiting for bytes, not a delay after decoding. The
server is limited to one request per arm, 1MiB, 64×64 pixels, a random loopback-only endpoint and a
15-second per-stream limit; the fixture is only 2×1 pixels/71 bytes. No fixed sleep creates a race.

The driver requires both actual canvas ServiceRequest/no reply and actual unfinished decode before
performing each visible UI action. It exercises two separate cases:

1. Restart the current project through its menu/confirmation while the decode is pending. Observe
   real abort and a new epoch; fresh pure-color canvas/HTML requests must complete while the old
   PNG bytes remain withheld. Then release the original bytes and require one late settlement and
   no old ServiceResponse.
2. Start another pending decode, then open the distinct `fixture-snake-service-lifecycle-next`
   project through the real menu/confirmation and host project loader. Its different game code,
   script marker, fresh canvas response and `lifecycle-next.png` decode with a newer resource
   generation must complete before the old bytes are released. No restart is relabeled as a
   project switch. The new image has independent project ownership despite using the same tiny
   source PNG bytes.

The ledger is observational. Tests never call a private store or synthesize runtime input/replies.
Raw full snapshots and stream events are retained; after release, a stale reply is an error, not an
ignored assertion. Server/socket cleanup runs in finally on success and failure. The existing
5-second full DOM/runtime watchdog remains active; equal normalized snapshots fail immediately.

## Executable entries (prepared, not executed by the implementation agent)

From the Web worktree, after all required static gates:

- Chromium: `node scripts/snake-service-lifecycle-chromium.mjs --project ../rustyera-core/tools/runtime-tester/fixture-snake-service-lifecycle --replacement-project ../rustyera-core/tools/runtime-tester/fixture-snake-service-lifecycle-next --chromium-executable /absolute/path/to/existing/chromium --output /absolute/new/evidence-directory`.
  Uses a real headful Chromium window, an explicitly existing executable and isolated browser
  contexts. There is no install/download path. Selection uses the actual directory input via
  Playwright FileChooser, with the filesystem picker capability disabled for this fixture only.
- Native Firefox/Safari: `npm run test:browser-compat -- --browser firefox --snake-service-lifecycle --project ../rustyera-core/tools/runtime-tester/fixture-snake-service-lifecycle --replacement-project ../rustyera-core/tools/runtime-tester/fixture-snake-service-lifecycle-next` (replace firefox with safari).
- Tauri: `npm run test:tauri -- --spec tests/tauri/snake-service-lifecycle.spec.mjs --project ../rustyera-core/tools/runtime-tester/fixture-snake-service-lifecycle --replacement-project ../rustyera-core/tools/runtime-tester/fixture-snake-service-lifecycle-next`.
  The runner copies both projects. Only the existing WebDriver test configuration allows loopback
  image URLs; production CSP is unchanged. The test picker queue selects a real second directory
  via the ordinary Rust open_project command, not mock IPC.

The ordinary `snake-service-lifecycle.json` fixed scenario is only the pointer/draw/scroll subset;
its sixth input does not claim native blur or pending races. It cannot substitute for the above
real lifecycle runner.

## Honest host blockers and manual focus entry

Native WebDriver support for creating/switching/closing a separate window and producing trusted
blur varies. A rejected command, absence of trusted blur, or missing restored focus is reported as
`window-blur` blocked and causes the final lifecycle result to fail even when later race evidence
is available. Tauri is never replaced by Chromium. No host has been executed/accepted by adding
these sources. If a host cannot perform real focus automation, the outstanding manual procedure
is to stop at pointer input5, hover the script button, activate another actual OS application,
return without moving the pointer, and submit Enter. Record trusted blur/unchanged pointer-event
history plus `SNAKE_LIFECYCLE_POINTER_5=0/0/` and the uninterrupted full snapshots; a manual result
must be separately attributed. The driver does not dispatch synthetic blur/focus events to pass.

Failure to observe a pending request, actual decode start, actual abort, late settlement, healthy
new epoch or newer resource generation leaves that requirement incomplete. A stream/resource/CSP
failure is a real failed run; no unsupported host is silently accepted.
