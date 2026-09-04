# 批次 1：交付提交与验证绑定

本页记录收尾提交；先前1A–1C及1D基础功能提交见[实施记录](SNAKE_EMUERA_IMPLEMENTATION_LOG.md#batch-1)。
分项提交只移动Git索引，不修改经过验证的最终工作树。没有逐个中间commit重跑测试；
验证针对各修复完成后的组合源码，源hash、产物和首次/定向结果均保留。

## 最终产品绑定

| 组件 | 提交 | 绑定与验证 |
|---|---|---|
| core | `e919d3719a2b0f5394c545783caa27289dcd7f7d`（收尾工具代码；后续文档提交不改产品crate） | 产品契约/pin仍为`b8b5bee45d1a7d3fc31f4df42dcbe0048422794a`，crates内容一致 |
| Web | `e3633311233df4a502faa41b32d8807c8c38de33` | 所有core Git依赖、rev和发布锁均绑定b8b5bee；Browser/WASM与Tauri分别验证 |
| TUI | `ad5c018b7c73bac441a9064d3339a174eff7dcfa` | 同样绑定b8b5bee；源码/打包库的数据断面与缺服务诊断通过 |

WASM SHA256 `bbe455923aca722a49c8f4dde3cc35498393455eae7f3a301b2f9d9d439205bf`；
最新Tauri SHA256 `e6355b5425d1e25926871b1f4f9cecea4a3d80147dd81a966c3b3f58e77bc6be`。
Tauri矩阵16项保留旧7c109b产物绑定、18项绑定e6355b；没有把旧捕获伪装成新binary运行。
Tauri生命周期与组合的fe664历史绑定也保留。最终源码提交与捕获时dirty源码按文件内容对应，
不因仅提交Git历史而重复构建。C ABI/TUI产物明细见1D实施记录与repair36证据。

没有C函数表布局变更、产品版本号调整、推送或主线合并。根CHANGELOG_PENDING只记录已验证
产品功能与修复，未把下述测试/构建流程改进写成功能。

## 本次收尾的core分项提交

### `d15211f61d4e9bffa2c21c5df937d931f3c11003` — fix(tester): expose completed work during coverage preparation

Publish comparator, graph and symbol projection progress without treating time as progress. Preserve stable ordering and complete symbol records.

Verified with targeted coverage/watchdog checks, scale fixtures and the authorized TW repeat4 report; no full suite rerun.

### `67eb72f0529b1a82ae34c735d26a92836a563c75` — fix(tester): allow four unchanged samples only while loading

Apply the user-approved four-sample rule only to explicit loading phases. Execution and report work retain two-sample stall detection.

Verified with focused policy and real supervised stall tests, plus authorized TW repeat4.

### `10a8d31296a06c5e9d3073f3e7f4e7a2a1a217e8` — test(services): derive split budgets from observed font units

Keep the combined client fixture valid with fallback fonts while retaining a separate explicit no-progress rejection case.

Verified by the completed browser and native service and Batch1 combination runs.

### `cca7caedff6b53100bda82ae1e3c8bf19a157d7c` — test(services): keep the real hover target stable after input

Remove the accepted input row before pointer sampling so assertions observe the original visible target. Do not synthesize pointer state.

Verified by native service and all four client lifecycle runs.

### `2528dff81b4b1d88125221313b2701db06d9a283` — fix(oracle): validate decimal policy versions without rewriting evidence

Accept canonical Web BigInt version encoding with bounded integer checks while preserving the raw identity used for protocol correlation.

Verified by targeted policy and real frontend evidence checks.

### `b2baf3a3c24f531caf18964f9506b22ea6bf3e79` — fix(oracle): match frontend full-path inventory ordering

Sort relative path strings by UTF-16 code units to match the JavaScript producer, including non-BMP paths.

Verified by the focused inventory regression and frontend adapters.

### `59b07cdc95235c50d9cd2e6befb6de1c7bbd558b` — fix(oracle): validate recorded capability prefixes on recompare

Accept only the exact optional capabilities handshake before load and reject extra or reordered steps.

Verified by focused malformed-prefix cases and offline service comparisons.

### `8820d5167cca2ed8e4940676f3addb4d72c00392` — fix(oracle): compare typed state at script error boundaries

Compare requested watches and actual termination on failed executions. Preserve diagnostic incomparability; timeout, quit and limits never become successful rejection.

Verified by targeted scalar-type/error regressions and fixed-reference service comparisons.

### `e919d3719a2b0f5394c545783caa27289dcd7f7d` — fix(oracle): normalize only optional debug reference wire fields

Allow omitted Option fields and protocol integer encodings without weakening typed values, indices, generations, correlation or epochs.

Verified by targeted tamper cases and native/Firefox/Safari frontend capture adapters.


## 本次收尾的Web分项提交

### `926043e0842ab9de10b19a4f72cf645d69a8add4` — chore: bind the verified core postmortem contract

The native and browser hosts must consume the same committed core contract. Synchronize the full revision in the workspace dependencies, lockfile and core pin without changing product versions.

Scope: Cargo.lock, Cargo.toml, rustyera-core.rev.

No file-specific passing gate has been asserted for this split from the available result metadata.

These index-only splits were not independently tested. Existing checks apply to their recorded source tree, not automatically to this intermediate commit. See the batch implementation log for dynamic acceptance and remaining blockers.

### `83e187e29f25e3fabce7cfa34db9452618656ba3` — fix: measure HTML without waiting for background paint

WKWebView suspended the animation-frame callback while the document was hidden, although fonts were already loaded. Measure after Vue, fonts and media settle, preserve synchronous layout reads and strict revision checks, and retain phase-specific deadline diagnostics.

Scope: src/components/htmlMeasurementProjection.ts, src/platform/htmlMeasurement.ts, tests/htmlBoxLayout.test.ts.

Previously recorded checks on the integrated working tree:
- validation/runs/repair46-html-format/result.json: exit 0 (node node_modules/prettier/bin/prettier.cjs --check src/components/htmlMeasurementProjection.ts src/platform/htmlMeasurement.ts tests/htmlBoxLayout.test.ts).
- validation/runs/repair46-html-eslint/result.json: exit 0 (node node_modules/eslint/bin/eslint.js src/components/htmlMeasurementProjection.ts src/platform/htmlMeasurement.ts tests/htmlBoxLayout.test.ts).
- validation/runs/repair46-html-tests/result.json: exit 0 (node node_modules/vitest/vitest.mjs run tests/htmlBoxLayout.test.ts tests/canvasReplay.test.ts).

These index-only splits were not independently tested. Existing checks apply to their recorded source tree, not automatically to this intermediate commit. See the batch implementation log for dynamic acceptance and remaining blockers.

### `aa417ae6a101c4d0054b90354f240e4dc95ac706` — fix: reject background and stale boundary pointer events

Background input and layout-driven pointerout events could replace the last valid cursor position. Reject hidden or unfocused input and preserve the last real move across internal boundary events while blur and window exit still clear it.

Scope: src/platform/pointerObservation.ts, tests/runtimeServices.test.ts.

Previously recorded checks on the integrated working tree:
- validation/runs/repair69-runtime-viewport-format/result.json: exit 0 (npx prettier --check src/stores/runtimeViewport.ts tests/runtimeServices.test.ts).
- validation/runs/repair69-runtime-viewport-eslint/result.json: exit 0 (npx eslint src/stores/runtimeViewport.ts tests/runtimeServices.test.ts).
- validation/runs/repair69-runtime-services-vitest/result.json: exit 0 (npx vitest run tests/runtimeServices.test.ts).

These index-only splits were not independently tested. Existing checks apply to their recorded source tree, not automatically to this intermediate commit. See the batch implementation log for dynamic acceptance and remaining blockers.

### `86c80724ae73082f7230ee21f229f9fa5217026e` — fix: permit exact debug inspection after a runtime fault

Faulted runtimes still need real typed watch inspection for diagnosis. Permit the existing pause path in the faulted phase, preserving the original fault, stop identity and non-interactive state after inspection.

Scope: src/stores/runtimeStore.ts, tests/runtimeStore-debug-presentation-reload.cases.ts.

Previously recorded checks on the integrated working tree:
- validation/runs/repair64-runtime-debug-format/result.json: exit 0 (npx prettier --check src/stores/runtimeDebugRequests.ts src/stores/runtimeStore.ts tests/runtimeDebugRequests.test.ts tests/runtimeStore-debug-presentation-reload.cases.ts).
- validation/runs/repair64-runtime-debug-eslint/result.json: exit 0 (npx eslint src/stores/runtimeDebugRequests.ts src/stores/runtimeStore.ts tests/runtimeDebugRequests.test.ts tests/runtimeStore-debug-presentation-reload.cases.ts).
- validation/runs/repair38-format/result.json: exit 0 (node node_modules/prettier/bin/prettier.cjs --check scripts/tauri-build-cache.mjs scripts/tauri-test.mjs scripts/tauri-test-support.mjs scripts/snake-service-lifecycle-test-support.mjs src/platform/pointerObservation.ts src/stores/runtimeStore.ts src/testing/runtimeEvidence.ts tests/runtimeServices.test.ts tests/testingControl.test.ts tests/tauriTestSupport.test.js).

These index-only splits were not independently tested. Existing checks apply to their recorded source tree, not automatically to this intermediate commit. See the batch implementation log for dynamic acceptance and remaining blockers.

### `319f960ddb51b2048c05c73a7a60460c7439e67d` — fix: register debug requests before handling early replies

The native pump can deliver a correlated reply before submitDebug returns its message ID. Bound early replies to in-flight submissions, register before completing them, release waiters on reset, and preserve receive-order presentation and newer-stop protection.

Scope: src/stores/runtimeDebugRequests.ts, src/stores/runtimeStore.ts, tests/runtimeDebugRequests.test.ts, tests/runtimeStore-debug-presentation-reload.cases.ts.

Previously recorded checks on the integrated working tree:
- validation/runs/repair64-runtime-debug-format/result.json: exit 0 (npx prettier --check src/stores/runtimeDebugRequests.ts src/stores/runtimeStore.ts tests/runtimeDebugRequests.test.ts tests/runtimeStore-debug-presentation-reload.cases.ts).
- validation/runs/repair64-runtime-debug-eslint/result.json: exit 0 (npx eslint src/stores/runtimeDebugRequests.ts src/stores/runtimeStore.ts tests/runtimeDebugRequests.test.ts tests/runtimeStore-debug-presentation-reload.cases.ts).
- validation/runs/repair64-runtime-debug-requests-vitest/result.json: exit 0 (npx vitest run tests/runtimeDebugRequests.test.ts).

These index-only splits were not independently tested. Existing checks apply to their recorded source tree, not automatically to this intermediate commit. See the batch implementation log for dynamic acceptance and remaining blockers.

### `ab7367b5a8142b04ee086c71eb7652b95fbefdf1` — fix: recover projection identity after explicit rejection

Core can reject a transient newer observation while retaining its predecessor. Keep bounded submitted candidates, invalidate them while newer work is pending, restore only after explicit rejection, and conservatively invalidate all in-flight revisions after transport failure.

Scope: src/stores/runtimeViewport.ts, tests/runtimeServices.test.ts.

Previously recorded checks on the integrated working tree:
- validation/runs/repair69-runtime-viewport-format/result.json: exit 0 (npx prettier --check src/stores/runtimeViewport.ts tests/runtimeServices.test.ts).
- validation/runs/repair69-runtime-viewport-eslint/result.json: exit 0 (npx eslint src/stores/runtimeViewport.ts tests/runtimeServices.test.ts).
- validation/runs/repair69-runtime-services-vitest/result.json: exit 0 (npx vitest run tests/runtimeServices.test.ts).

These index-only splits were not independently tested. Existing checks apply to their recorded source tree, not automatically to this intermediate commit. See the batch implementation log for dynamic acceptance and remaining blockers.

### `46fe905e09836a092511d103c003982025361766` — test: add an explicit trusted macOS WebDriver input provider

Synthetic DOM actions cannot validate native pointer and keyboard services. Bind an opt-in source overlay to immutable inventories and drive the actual session WKWebView/window, including logical-screen cursor synchronization, focus guards and a trusted-input probe.

Scope: scripts/tauri-native-webdriver-support.mjs, scripts/tauri-test.mjs, tests/tauri/native-input.spec.mjs, tests/tauriTestSupport.test.js, tools/tauri-native-webdriver/LICENSE.upstream, tools/tauri-native-webdriver/README.md, tools/tauri-native-webdriver/native-input.patch, tools/tauri-native-webdriver/original-inventory.json, tools/tauri-native-webdriver/overlay-manifest.json, tools/tauri-native-webdriver/overrides/src/platform/executor.rs, tools/tauri-native-webdriver/overrides/src/platform/macos.rs, tools/tauri-native-webdriver/overrides/src/platform/macos_input.rs, tools/tauri-native-webdriver/overrides/src/platform/mod.rs, tools/tauri-native-webdriver/overrides/src/server/handlers/window.rs, tools/tauri-native-webdriver/prepare_provider.py.

Previously recorded checks on the integrated working tree:
- validation/runs/repair67-native-foreground-vitest/result.json: exit 0 (npx vitest run tests/tauriTestSupport.test.js -t native foreground).
- validation/runs/repair62-tauri-format/result.json: exit 0 (node node_modules/prettier/bin/prettier.cjs --check src/platform/tauriBridge.ts scripts/snake-service-capture-client.mjs scripts/web-test.mjs tests/tauriBridge.test.ts tests/tauriTestSupport.test.js .agents/skills/test-rustyera-web/references/test-cli.md).
- validation/runs/repair62-tauri-eslint/result.json: exit 0 (node node_modules/eslint/bin/eslint.js src/platform/tauriBridge.ts scripts/snake-service-capture-client.mjs scripts/web-test.mjs tests/tauriBridge.test.ts tests/tauriTestSupport.test.js).

These index-only splits were not independently tested. Existing checks apply to their recorded source tree, not automatically to this intermediate commit. See the batch implementation log for dynamic acceptance and remaining blockers.

### `12153c3e329377f38dc2acc2e555d7aacbb31494` — test: reuse only verified official Tauri build artifacts

Rebuilding the same native host for every isolated fixture wastes acceptance time. Fingerprint the official build inputs and executable, provide runtime project paths after session creation, and reject strict cache misses or source, environment and artifact drift.

Scope: .agents/skills/test-rustyera-web/references/tauri-e2e.md, scripts/tauri-build-cache.mjs, scripts/tauri-test.mjs, tests/tauriTestSupport.test.js.

Previously recorded checks on the integrated working tree:
- validation/runs/repair67-native-foreground-vitest/result.json: exit 0 (npx vitest run tests/tauriTestSupport.test.js -t native foreground).
- validation/runs/repair62-tauri-format/result.json: exit 0 (node node_modules/prettier/bin/prettier.cjs --check src/platform/tauriBridge.ts scripts/snake-service-capture-client.mjs scripts/web-test.mjs tests/tauriBridge.test.ts tests/tauriTestSupport.test.js .agents/skills/test-rustyera-web/references/test-cli.md).
- validation/runs/repair62-tauri-eslint/result.json: exit 0 (node node_modules/eslint/bin/eslint.js src/platform/tauriBridge.ts scripts/snake-service-capture-client.mjs scripts/web-test.mjs tests/tauriBridge.test.ts tests/tauriTestSupport.test.js).

These index-only splits were not independently tested. Existing checks apply to their recorded source tree, not automatically to this intermediate commit. See the batch implementation log for dynamic acceptance and remaining blockers.

### `35e058ae6d4dbcda38a56c66b5daf64c596a35b1` — test: stop retrying rejected native WebDriver commands

The standalone service enabled ten HTTP retries after connection, masking native failures and delaying the watchdog. Disable command retries once the session exists and retain a bounded connection timeout.

Scope: scripts/tauri-test.mjs.

Previously recorded checks on the integrated working tree:
- validation/runs/repair59-diagnosis-nodecheck/result.json: exit 0 (node --check scripts/tauri-test.mjs).
- validation/runs/repair59-diagnosis-format/result.json: exit 0 (node node_modules/prettier/bin/prettier.cjs --check src/testing/serviceLifecycle.ts src/platform/tauriBridge.ts scripts/tauri-test.mjs tests/testingControl.test.ts tests/tauriBridge.test.ts).
- validation/runs/repair59-diagnosis-eslint/result.json: exit 0 (node node_modules/eslint/bin/eslint.js src/testing/serviceLifecycle.ts src/platform/tauriBridge.ts scripts/tauri-test.mjs tests/testingControl.test.ts tests/tauriBridge.test.ts).

These index-only splits were not independently tested. Existing checks apply to their recorded source tree, not automatically to this intermediate commit. See the batch implementation log for dynamic acceptance and remaining blockers.

### `b8b848bf876a521d93d234fbeaa100bd606da71e` — test: establish the current native window before input

Native input is correctly rejected when another window owns focus. Switch to the session current handle and observe actual document visibility/focus at startup and after archive export; retain read-only foreground diagnostics on failure.

Scope: scripts/tauri-test-support.mjs, scripts/tauri-test.mjs, tests/tauri/snake-service-oracle.spec.mjs, tests/tauriTestSupport.test.js.

Previously recorded checks on the integrated working tree:
- validation/runs/repair74-grid-format/result.json: exit 0 (npx prettier --check tests/tauri/snake-service-oracle.spec.mjs src/styles.css).
- validation/runs/repair74-grid-spec-eslint/result.json: exit 0 (npx eslint tests/tauri/snake-service-oracle.spec.mjs).
- validation/runs/repair74-grid-spec-nodecheck/result.json: exit 0 (node --check tests/tauri/snake-service-oracle.spec.mjs).

These index-only splits were not independently tested. Existing checks apply to their recorded source tree, not automatically to this intermediate commit. See the batch implementation log for dynamic acceptance and remaining blockers.

### `31ccdd7e65f1c1e63f3f3da9a72056fd666ac858` — test: activate the selected browser before lifecycle input

A restored Safari or Firefox window may remain behind another application despite a valid WebDriver handle. Activate only the selected browser, switch its actual automation window, and wait for visible document focus without synthetic window.focus calls.

Scope: scripts/browser-compat-test.mjs, scripts/web-test-lib.mjs, tests/webTestLib.test.js.

Previously recorded checks on the integrated working tree:
- validation/runs/repair49-browser-foreground-nodecheck-browser-compat/result.json: exit 0 (node --check scripts/browser-compat-test.mjs).
- validation/runs/repair49-browser-foreground-nodecheck-web-test-lib/result.json: exit 0 (node --check scripts/web-test-lib.mjs).
- validation/runs/repair49-browser-foreground-format/result.json: exit 0 (node node_modules/prettier/bin/prettier.cjs --check scripts/web-test-lib.mjs scripts/browser-compat-test.mjs tests/webTestLib.test.js).

These index-only splits were not independently tested. Existing checks apply to their recorded source tree, not automatically to this intermediate commit. See the batch implementation log for dynamic acceptance and remaining blockers.

### `0198070e7423a9125baafcc9a3595fe9ea4753bb` — test: accept an existing Chromium executable

Acceptance must reuse the installed browser instead of downloading another Chromium. Add an explicit executable path with existence validation and preserve default selection when the option is absent.

Scope: .agents/skills/test-rustyera-web/references/test-cli.md, scripts/web-test.mjs.

Previously recorded checks on the integrated working tree:
- validation/runs/repair62-tauri-nodecheck-webtest/result.json: exit 0 (node --check scripts/web-test.mjs).
- validation/runs/repair62-tauri-format/result.json: exit 0 (node node_modules/prettier/bin/prettier.cjs --check src/platform/tauriBridge.ts scripts/snake-service-capture-client.mjs scripts/web-test.mjs tests/tauriBridge.test.ts tests/tauriTestSupport.test.js .agents/skills/test-rustyera-web/references/test-cli.md).
- validation/runs/repair62-tauri-eslint/result.json: exit 0 (node node_modules/eslint/bin/eslint.js src/platform/tauriBridge.ts scripts/snake-service-capture-client.mjs scripts/web-test.mjs tests/tauriBridge.test.ts tests/tauriTestSupport.test.js).

These index-only splits were not independently tested. Existing checks apply to their recorded source tree, not automatically to this intermediate commit. See the batch implementation log for dynamic acceptance and remaining blockers.

### `a5285088f6497f7a69d1b54b48caaf6ec64a8563` — test: correlate pointer queries with independent DOM samples

MOUSEX, MOUSEY and MOUSEB issue separate requests and native layout can move the pointer between them. Freeze independent DOM observations at each real sample boundary, bind request/session/revisions, decode actual CBOR bytes strictly, and retain bounded diagnostics without changing wire record order.

Scope: scripts/snake-service-lifecycle-chromium.mjs, scripts/snake-service-lifecycle-test-support.mjs, scripts/snake-services-test-support.mjs, scripts/tauri-test-support.mjs, src/stores/runtimeStore.ts, src/testing/runtimeEvidence.ts, tests/tauriTestSupport.test.js, tests/testingControl.test.ts.

Previously recorded checks on the integrated working tree:
- validation/runs/repair67-native-foreground-vitest/result.json: exit 0 (npx vitest run tests/tauriTestSupport.test.js -t native foreground).
- validation/runs/repair64-runtime-debug-format/result.json: exit 0 (npx prettier --check src/stores/runtimeDebugRequests.ts src/stores/runtimeStore.ts tests/runtimeDebugRequests.test.ts tests/runtimeStore-debug-presentation-reload.cases.ts).
- validation/runs/repair64-runtime-debug-eslint/result.json: exit 0 (npx eslint src/stores/runtimeDebugRequests.ts src/stores/runtimeStore.ts tests/runtimeDebugRequests.test.ts tests/runtimeStore-debug-presentation-reload.cases.ts).

These index-only splits were not independently tested. Existing checks apply to their recorded source tree, not automatically to this intermediate commit. See the batch implementation log for dynamic acceptance and remaining blockers.

### `9549bdfc34b2eba670a7dc0c5da036441aec2e89` — test: observe native prompt readiness before pointer actions

Native setValue completion does not itself prove that the expected prompt value and focus are observable. Read the actual enabled input and document focus before continuing the existing real pointer and keyboard actions.

Scope: scripts/snake-service-lifecycle-test-support.mjs, tests/tauriTestSupport.test.js.

Previously recorded checks on the integrated working tree:
- validation/runs/repair67-native-foreground-vitest/result.json: exit 0 (npx vitest run tests/tauriTestSupport.test.js -t native foreground).
- validation/runs/repair62-tauri-format/result.json: exit 0 (node node_modules/prettier/bin/prettier.cjs --check src/platform/tauriBridge.ts scripts/snake-service-capture-client.mjs scripts/web-test.mjs tests/tauriBridge.test.ts tests/tauriTestSupport.test.js .agents/skills/test-rustyera-web/references/test-cli.md).
- validation/runs/repair62-tauri-eslint/result.json: exit 0 (node node_modules/eslint/bin/eslint.js src/platform/tauriBridge.ts scripts/snake-service-capture-client.mjs scripts/web-test.mjs tests/tauriBridge.test.ts tests/tauriTestSupport.test.js).

These index-only splits were not independently tested. Existing checks apply to their recorded source tree, not automatically to this intermediate commit. See the batch implementation log for dynamic acceptance and remaining blockers.

### `b4100bd72f1a17ff151d8adcf0ed9ee9cfcef4b9` — test: bind blur probes to their actual native window handles

The high-level window helper could choose a stale handle or check main-window focus before closing the probe. Use the native creation result, require trusted blur, close only that auxiliary window, and confirm restored focus afterwards.

Scope: scripts/snake-service-lifecycle-chromium.mjs, scripts/snake-service-lifecycle-test-support.mjs, tests/tauriTestSupport.test.js.

Previously recorded checks on the integrated working tree:
- validation/runs/repair67-native-foreground-vitest/result.json: exit 0 (npx vitest run tests/tauriTestSupport.test.js -t native foreground).
- validation/runs/repair62-tauri-format/result.json: exit 0 (node node_modules/prettier/bin/prettier.cjs --check src/platform/tauriBridge.ts scripts/snake-service-capture-client.mjs scripts/web-test.mjs tests/tauriBridge.test.ts tests/tauriTestSupport.test.js .agents/skills/test-rustyera-web/references/test-cli.md).
- validation/runs/repair62-tauri-eslint/result.json: exit 0 (node node_modules/eslint/bin/eslint.js src/platform/tauriBridge.ts scripts/snake-service-capture-client.mjs scripts/web-test.mjs tests/tauriBridge.test.ts tests/tauriTestSupport.test.js).

These index-only splits were not independently tested. Existing checks apply to their recorded source tree, not automatically to this intermediate commit. See the batch implementation log for dynamic acceptance and remaining blockers.

### `f6c8117ebcfcb94a22513a0893b6e602b5ea73bd` — test: establish real viewport focus before PageUp

Compatibility warning overlays could intercept the click intended to focus the viewport. Dismiss warnings through their real controls, require the viewport as activeElement, and retain bounded trusted key/scroll evidence that distinguishes cancellation, retargeting and scroll rebound.

Scope: scripts/snake-service-lifecycle-chromium.mjs, scripts/snake-service-lifecycle-test-support.mjs, tests/tauriTestSupport.test.js.

Previously recorded checks on the integrated working tree:
- validation/runs/repair67-native-foreground-vitest/result.json: exit 0 (npx vitest run tests/tauriTestSupport.test.js -t native foreground).
- validation/runs/repair62-tauri-format/result.json: exit 0 (node node_modules/prettier/bin/prettier.cjs --check src/platform/tauriBridge.ts scripts/snake-service-capture-client.mjs scripts/web-test.mjs tests/tauriBridge.test.ts tests/tauriTestSupport.test.js .agents/skills/test-rustyera-web/references/test-cli.md).
- validation/runs/repair62-tauri-eslint/result.json: exit 0 (node node_modules/eslint/bin/eslint.js src/platform/tauriBridge.ts scripts/snake-service-capture-client.mjs scripts/web-test.mjs tests/tauriBridge.test.ts tests/tauriTestSupport.test.js).

These index-only splits were not independently tested. Existing checks apply to their recorded source tree, not automatically to this intermediate commit. See the batch implementation log for dynamic acceptance and remaining blockers.

### `e13ba1925a422a271d360ae7138b183e057e6ed6` — test: wait for a fresh lifecycle session after restart

Old output and an old ready prompt remain visible while restart is scheduled. Capture the previous composite session identity before confirmation and require a new session, no project loading and a real integer wait before continuing.

Scope: scripts/snake-service-lifecycle-races.mjs, tests/tauriTestSupport.test.js.

Previously recorded checks on the integrated working tree:
- validation/runs/repair67-native-foreground-vitest/result.json: exit 0 (npx vitest run tests/tauriTestSupport.test.js -t native foreground).
- validation/runs/repair62-tauri-format/result.json: exit 0 (node node_modules/prettier/bin/prettier.cjs --check src/platform/tauriBridge.ts scripts/snake-service-capture-client.mjs scripts/web-test.mjs tests/tauriBridge.test.ts tests/tauriTestSupport.test.js .agents/skills/test-rustyera-web/references/test-cli.md).
- validation/runs/repair62-tauri-eslint/result.json: exit 0 (node node_modules/eslint/bin/eslint.js src/platform/tauriBridge.ts scripts/snake-service-capture-client.mjs scripts/web-test.mjs tests/tauriBridge.test.ts tests/tauriTestSupport.test.js).

These index-only splits were not independently tested. Existing checks apply to their recorded source tree, not automatically to this intermediate commit. See the batch implementation log for dynamic acceptance and remaining blockers.

### `c2de49267568f8c4f82e0d03a640aae5cec33182` — test: configure one-use native diagnosis export paths

Reusable builds cannot embed per-case export paths; otherwise the unattended archive export opens a native save dialog. Consume a test-only normalized runtime destination once, retain the fixed test fallback, and preserve real native identity inspection and archive writes.

Scope: scripts/tauri-test.mjs, src/platform/tauriBridge.ts, src/testing/serviceLifecycle.ts, tests/tauriBridge.test.ts, tests/testingControl.test.ts.

Previously recorded checks on the integrated working tree:
- validation/runs/repair62-tauri-format/result.json: exit 0 (node node_modules/prettier/bin/prettier.cjs --check src/platform/tauriBridge.ts scripts/snake-service-capture-client.mjs scripts/web-test.mjs tests/tauriBridge.test.ts tests/tauriTestSupport.test.js .agents/skills/test-rustyera-web/references/test-cli.md).
- validation/runs/repair62-tauri-eslint/result.json: exit 0 (node node_modules/eslint/bin/eslint.js src/platform/tauriBridge.ts scripts/snake-service-capture-client.mjs scripts/web-test.mjs tests/tauriBridge.test.ts tests/tauriTestSupport.test.js).
- validation/runs/repair62-tauri-vitest/result.json: exit 0 (node node_modules/vitest/vitest.mjs run tests/tauriBridge.test.ts tests/tauriTestSupport.test.js).

These index-only splits were not independently tested. Existing checks apply to their recorded source tree, not automatically to this intermediate commit. See the batch implementation log for dynamic acceptance and remaining blockers.

### `380e97e6bf58a03a8799b443698f3b664f020858` — fix: preserve export evidence before worker buffer transfer

The archive worker transfers ownership of project and replay buffers, so later evidence reads can fail on detached views. Preserve only the bounded test evidence before transfer and publish it only after the native archive commit succeeds.

Scope: src/platform/tauriBridge.ts, tests/tauriBridge.test.ts.

Previously recorded checks on the integrated working tree:
- validation/runs/repair62-tauri-format/result.json: exit 0 (node node_modules/prettier/bin/prettier.cjs --check src/platform/tauriBridge.ts scripts/snake-service-capture-client.mjs scripts/web-test.mjs tests/tauriBridge.test.ts tests/tauriTestSupport.test.js .agents/skills/test-rustyera-web/references/test-cli.md).
- validation/runs/repair62-tauri-eslint/result.json: exit 0 (node node_modules/eslint/bin/eslint.js src/platform/tauriBridge.ts scripts/snake-service-capture-client.mjs scripts/web-test.mjs tests/tauriBridge.test.ts tests/tauriTestSupport.test.js).
- validation/runs/repair62-tauri-vitest/result.json: exit 0 (node node_modules/vitest/vitest.mjs run tests/tauriBridge.test.ts tests/tauriTestSupport.test.js).

These index-only splits were not independently tested. Existing checks apply to their recorded source tree, not automatically to this intermediate commit. See the batch implementation log for dynamic acceptance and remaining blockers.

### `7aa48aad2249dbfe60adfe95fa02034430f58749` — test: fail when identity export ends without evidence

A completed diagnosis error cannot later produce the committed project identity required by a capture. Report that failure immediately while preserving the actual result instead of waiting for the no-progress watchdog.

Scope: scripts/snake-service-capture-client.mjs, tests/tauriTestSupport.test.js.

Previously recorded checks on the integrated working tree:
- validation/runs/repair67-native-foreground-vitest/result.json: exit 0 (npx vitest run tests/tauriTestSupport.test.js -t native foreground).
- validation/runs/repair62-tauri-nodecheck-capture/result.json: exit 0 (node --check scripts/snake-service-capture-client.mjs).
- validation/runs/repair62-tauri-format/result.json: exit 0 (node node_modules/prettier/bin/prettier.cjs --check src/platform/tauriBridge.ts scripts/snake-service-capture-client.mjs scripts/web-test.mjs tests/tauriBridge.test.ts tests/tauriTestSupport.test.js .agents/skills/test-rustyera-web/references/test-cli.md).

These index-only splits were not independently tested. Existing checks apply to their recorded source tree, not automatically to this intermediate commit. See the batch implementation log for dynamic acceptance and remaining blockers.

### `23a721f1b6381f3aea51bd97c8e49209044f700d` — fix: prevent prompt content from widening viewport grids

Implicit grid columns used prompt min-content width and changed the viewport from 325 to 327 pixels inside a 320-pixel native window. Use minmax(0, 1fr) columns for both containers and assert real viewport width stability before, during and after native prompt submission.

Scope: src/styles.css, tests/tauri/snake-service-oracle.spec.mjs.

Previously recorded checks on the integrated working tree:
- validation/runs/repair74-grid-format/result.json: exit 0 (npx prettier --check tests/tauri/snake-service-oracle.spec.mjs src/styles.css).
- validation/runs/repair74-grid-spec-eslint/result.json: exit 0 (npx eslint tests/tauri/snake-service-oracle.spec.mjs).
- validation/runs/repair74-grid-spec-nodecheck/result.json: exit 0 (node --check tests/tauri/snake-service-oracle.spec.mjs).

These index-only splits were not independently tested. Existing checks apply to their recorded source tree, not automatically to this intermediate commit. See the batch implementation log for dynamic acceptance and remaining blockers.

### `e3633311233df4a502faa41b32d8807c8c38de33` — chore: preserve required blank context in native patch files

Scope the trailing-whitespace attribute to the native input unified diff. Its single-space blank context lines are required patch syntax, not product source whitespace.

Validation: repair78 targeted attribute and historical/worktree whitespace checks passed; the patch SHA remains unchanged. No provider code, runtime artifact or test input changed.

## 根更新日志

`40fdea805c2fac3065a69fe541b8dd6265046efb`：只追加五项已验证产品功能/修复；未记录构建、测试或流程调整。
