import { test, expect } from "@playwright/test";

test.describe("model management", () => {
  test("navigate to model management shows ASR model list", async ({ page }) => {
    await page.addInitScript(() => {
      try {
        localStorage.clear();
      } catch {}
    });
    await page.goto("/");

    // Click model management nav button
    await page.locator(".nav-btn").filter({ hasText: "模型管理" }).click();

    // Should show the section heading
    await expect(page.locator("h2").filter({ hasText: "模型管理" })).toBeVisible();

    // Should show at least one model card (mock data in non-Tauri mode)
    await expect(page.locator(".card").first()).toBeVisible({ timeout: 3000 });

    // The app element itself should not be empty
    await expect(page.locator("#app")).not.toBeEmpty();
  });

  test("download progress bar updates with camelCase fields from Rust backend", async ({ page }) => {
    // ── Intercept Tauri API modules before load ────────────────────────────────
    // Replace @tauri-apps/api/core and @tauri-apps/api/event with stub modules
    // that expose a shared event-bus window.__tauriTestBus for test-driving.

    // core.js stub: invoke dispatches commands through the test bus
    await page.route("**/node_modules/@tauri-apps/api/core.js", (route) => {
      route.fulfill({
        contentType: "application/javascript",
        body: `
export async function invoke(cmd, args) {
  const bus = window.__tauriTestBus;
  if (bus && bus.onInvoke) return bus.onInvoke(cmd, args);
  return undefined;
}
export function transformCallback(fn) { return fn; }
`,
      });
    });

    // event.js stub: listen/unlisten go through the test bus
    await page.route("**/node_modules/@tauri-apps/api/event.js", (route) => {
      route.fulfill({
        contentType: "application/javascript",
        body: `
export async function listen(event, handler) {
  const bus = window.__tauriTestBus;
  if (!bus) return () => {};
  bus.listeners = bus.listeners || {};
  bus.listeners[event] = bus.listeners[event] || [];
  bus.listeners[event].push(handler);
  return () => {
    if (bus.listeners && bus.listeners[event]) {
      bus.listeners[event] = bus.listeners[event].filter(h => h !== handler);
    }
  };
}
export async function emit(event, payload) {
  const bus = window.__tauriTestBus;
  if (!bus || !bus.listeners || !bus.listeners[event]) return;
  for (const h of bus.listeners[event]) h({ payload });
}
`,
      });
    });

    // Inject __TAURI_INTERNALS__ and set up the test bus before page scripts run
    await page.addInitScript(() => {
      try { localStorage.clear(); } catch {}

      // Make isTauri = true in the frontend code
      (window as any).__TAURI_INTERNALS__ = { version: "2.0.0" };

      // Shared test bus for controlling mock behaviour
      (window as any).__tauriTestBus = {
        listeners: {} as Record<string, Array<(e: { payload: unknown }) => void>>,
        onInvoke: (cmd: string, _args: unknown) => {
          if (cmd === "list_cached_models") return Promise.resolve([]);
          if (cmd === "get_active_model_id") return Promise.resolve("");
          if (cmd === "check_library_model_installed") return Promise.resolve(false);
          if (cmd === "start_model_download") {
            // Fire camelCase progress events after a short delay
            const bus = (window as any).__tauriTestBus;
            setTimeout(() => {
              const handlers = bus.listeners?.["model-download-progress"] ?? [];
              for (const h of handlers) {
                h({
                  payload: {
                    modelId: "whisper-base",
                    downloadedBytes: 2_000_000,
                    totalBytes: 148_000_000,
                    speedBps: 1_048_576,   // 1 MB/s
                    status: "downloading",
                  },
                });
              }
            }, 150);
            setTimeout(() => {
              const bus2 = (window as any).__tauriTestBus;
              const handlers = bus2.listeners?.["model-download-progress"] ?? [];
              for (const h of handlers) {
                h({
                  payload: {
                    modelId: "whisper-base",
                    downloadedBytes: 14_800_000,
                    totalBytes: 148_000_000,
                    speedBps: 2_097_152,   // 2 MB/s
                    status: "downloading",
                  },
                });
              }
            }, 500);
            return Promise.resolve(undefined);
          }
          return Promise.resolve(undefined);
        },
      };
    });

    await page.goto("/");

    // Navigate to model management
    await page.locator(".nav-btn").filter({ hasText: "模型管理" }).click();
    await expect(page.locator("h2").filter({ hasText: "模型管理" })).toBeVisible();

    // Wait for whisper-base card to render (Tauri mode: no mock banner, real empty list)
    const whisperBaseCard = page.locator("#model-card-whisper-base");
    await expect(whisperBaseCard).toBeVisible({ timeout: 5000 });

    // The download button should be present (model not installed since list_cached_models returns [])
    const downloadBtn = whisperBaseCard.locator(".btn-download");
    await expect(downloadBtn).toBeVisible({ timeout: 3000 });

    // Click Download — triggers start_model_download invoke which fires progress events
    await downloadBtn.click();

    // Progress bar container should become visible
    const progressDiv = page.locator("#model-progress-whisper-base");
    await expect(progressDiv).toBeVisible({ timeout: 3000 });

    // Speed readout must show a non-zero value (proves camelCase speedBps arrived correctly)
    const speedEl = page.locator("#pbar-speed-whisper-base");
    await expect(speedEl).not.toHaveText("— B/s", { timeout: 5000 });
    await expect(speedEl).not.toHaveText("0 B/s", { timeout: 5000 });

    const speedText = await speedEl.textContent();
    expect(speedText).toMatch(/MB\/s/);

    // Detail (downloaded / total) must not be "0 B / 0 B"
    const detailEl = page.locator("#pbar-detail-whisper-base");
    const detailText = await detailEl.textContent();
    expect(detailText).not.toBe("0 B / 0 B");

    // Progress bar fill must be > 0%
    const pbarFill = page.locator("#pbar-whisper-base");
    const widthStyle = await pbarFill.getAttribute("style");
    expect(widthStyle).not.toContain("width: 0%");

    console.log(`✓ Speed readout: "${speedText}"`);
    console.log(`✓ Detail readout: "${detailText}"`);
    console.log(`✓ Progress bar style: "${widthStyle}"`);
  });

  test("new ASR models render with backend badges and library-managed download passes correct params", async ({ page }) => {
    // Stub Tauri core/event modules (same pattern as the camelCase test above)
    await page.route("**/node_modules/@tauri-apps/api/core.js", (route) => {
      route.fulfill({
        contentType: "application/javascript",
        body: `
export async function invoke(cmd, args) {
  const bus = window.__tauriTestBus;
  if (bus && bus.onInvoke) return bus.onInvoke(cmd, args);
  return undefined;
}
export function transformCallback(fn) { return fn; }
`,
      });
    });
    await page.route("**/node_modules/@tauri-apps/api/event.js", (route) => {
      route.fulfill({
        contentType: "application/javascript",
        body: `
export async function listen(event, handler) {
  const bus = window.__tauriTestBus;
  if (!bus) return () => {};
  bus.listeners = bus.listeners || {};
  bus.listeners[event] = bus.listeners[event] || [];
  bus.listeners[event].push(handler);
  return () => {};
}
export async function emit() {}
`,
      });
    });

    await page.addInitScript(() => {
      try { localStorage.clear(); } catch {}
      (window as any).__TAURI_INTERNALS__ = { version: "2.0.0" };
      // Record the args of every start_model_download call for later assertions
      (window as any).__downloadCalls = [];
      (window as any).__tauriTestBus = {
        listeners: {} as Record<string, Array<(e: { payload: unknown }) => void>>,
        onInvoke: (cmd: string, args: any) => {
          if (cmd === "list_cached_models") return Promise.resolve([]);
          if (cmd === "get_active_model_id") return Promise.resolve("");
          // Report all library-managed models as NOT installed so the download button shows
          if (cmd === "check_library_model_installed") return Promise.resolve(false);
          if (cmd === "start_model_download") {
            (window as any).__downloadCalls.push({ cmd, args });
            return Promise.resolve(undefined);
          }
          return Promise.resolve(undefined);
        },
      };
    });

    await page.goto("/");
    await page.locator(".nav-btn").filter({ hasText: "模型管理" }).click();
    await expect(page.locator("h2").filter({ hasText: "模型管理" })).toBeVisible();

    // Section heading should now read "ASR 模型" (was "Whisper 模型")
    await expect(page.locator("h3").filter({ hasText: "ASR 模型" })).toBeVisible({ timeout: 5000 });

    // ── Each of the three new models must render its own card ──────────────────
    const paraformerCard = page.locator("#model-card-paraformer-zh");
    const qwenCard = page.locator("#model-card-qwen3-asr-1\\.7b");
    const mimoCard = page.locator("#model-card-mimo-v2\\.5-asr");
    await expect(paraformerCard).toBeVisible({ timeout: 5000 });
    await expect(qwenCard).toBeVisible();
    await expect(mimoCard).toBeVisible();

    // ── Backend badges must be present on each card ────────────────────────────
    await expect(paraformerCard.getByText("paraformer", { exact: true })).toBeVisible();
    await expect(qwenCard.getByText("qwen3-asr", { exact: true })).toBeVisible();
    await expect(mimoCard.getByText("mimo", { exact: true })).toBeVisible();

    // ── MiMo requires GPU → must show the GPU warning badge ────────────────────
    // exact:true targets the badge <span>需要 GPU</span>, not the description text
    // "…（需要 GPU）" which also contains the phrase.
    await expect(mimoCard.getByText("需要 GPU", { exact: true })).toBeVisible();

    // ── Clicking download on Paraformer must invoke start_model_download with
    //    downloadMethod="library-managed" and backend="paraformer" ──────────────
    await paraformerCard.locator(".btn-download").click();
    await page.waitForTimeout(300);

    const calls = await page.evaluate(() => (window as any).__downloadCalls);
    expect(calls.length).toBeGreaterThan(0);
    const paraCall = calls.find((c: any) => c.args?.modelId === "paraformer-zh");
    expect(paraCall, "start_model_download should be called for paraformer-zh").toBeTruthy();
    expect(paraCall.args.downloadMethod).toBe("library-managed");
    expect(paraCall.args.backend).toBe("paraformer");

    console.log(`✓ paraformer download call args: ${JSON.stringify(paraCall.args)}`);
  });

  test("library-managed models pass modelSource and UI updates on starting status", async ({ page }) => {
    // Stub Tauri core/event modules
    await page.route("**/node_modules/@tauri-apps/api/core.js", (route) => {
      route.fulfill({
        contentType: "application/javascript",
        body: `
export async function invoke(cmd, args) {
  const bus = window.__tauriTestBus;
  if (bus && bus.onInvoke) return bus.onInvoke(cmd, args);
  return undefined;
}
export function transformCallback(fn) { return fn; }
`,
      });
    });
    await page.route("**/node_modules/@tauri-apps/api/event.js", (route) => {
      route.fulfill({
        contentType: "application/javascript",
        body: `
export async function listen(event, handler) {
  const bus = window.__tauriTestBus;
  if (!bus) return () => {};
  bus.listeners = bus.listeners || {};
  bus.listeners[event] = bus.listeners[event] || [];
  bus.listeners[event].push(handler);
  return () => {
    if (bus.listeners && bus.listeners[event]) {
      bus.listeners[event] = bus.listeners[event].filter(h => h !== handler);
    }
  };
}
export async function emit(event, payload) {
  const bus = window.__tauriTestBus;
  if (!bus || !bus.listeners || !bus.listeners[event]) return;
  for (const h of bus.listeners[event]) h({ payload });
}
`,
      });
    });

    await page.addInitScript(() => {
      try { localStorage.clear(); } catch {}
      (window as any).__TAURI_INTERNALS__ = { version: "2.0.0" };
      (window as any).__downloadCalls = [];
      (window as any).__tauriTestBus = {
        listeners: {} as Record<string, Array<(e: { payload: unknown }) => void>>,
        onInvoke: (cmd: string, args: any) => {
          if (cmd === "list_cached_models") return Promise.resolve([]);
          if (cmd === "get_active_model_id") return Promise.resolve("");
          if (cmd === "check_library_model_installed") return Promise.resolve(false);
          if (cmd === "start_model_download") {
            (window as any).__downloadCalls.push({ cmd, args });
            // Simulate library-managed download: emit "starting" then "downloading" then "completed"
            const bus = (window as any).__tauriTestBus;
            const modelId = args.modelId;
            setTimeout(() => {
              const handlers = bus.listeners?.["model-download-progress"] ?? [];
              for (const h of handlers) {
                h({
                  payload: {
                    modelId,
                    downloadedBytes: 0,
                    totalBytes: 100_000_000,
                    speedBps: 0,
                    status: "starting",  // Library-managed download initial status
                  },
                });
              }
            }, 100);
            setTimeout(() => {
              const handlers = bus.listeners?.["model-download-progress"] ?? [];
              for (const h of handlers) {
                h({
                  payload: {
                    modelId,
                    downloadedBytes: 50_000_000,
                    totalBytes: 100_000_000,
                    speedBps: 0,
                    status: "downloading",
                  },
                });
              }
            }, 300);
            setTimeout(() => {
              const handlers = bus.listeners?.["model-download-progress"] ?? [];
              for (const h of handlers) {
                h({
                  payload: {
                    modelId,
                    downloadedBytes: 100_000_000,
                    totalBytes: 100_000_000,
                    speedBps: 0,
                    status: "completed",
                  },
                });
              }
            }, 600);
            return Promise.resolve(undefined);
          }
          if (cmd === "set_active_model_id") return Promise.resolve(undefined);
          return Promise.resolve(undefined);
        },
      };
    });

    await page.goto("/");
    await page.locator(".nav-btn").filter({ hasText: "模型管理" }).click();
    await expect(page.locator("h2").filter({ hasText: "模型管理" })).toBeVisible();

    // ── Qwen3 ASR 1.7B: download must pass modelSource ──────────────────────
    const qwenCard = page.locator("#model-card-qwen3-asr-1\\.7b");
    await expect(qwenCard).toBeVisible({ timeout: 5000 });

    const qwenDownloadBtn = qwenCard.locator(".btn-download");
    await expect(qwenDownloadBtn).toBeVisible({ timeout: 3000 });
    await qwenDownloadBtn.click();

    // Wait for "starting" event to arrive (100ms delay in mock)
    await page.waitForTimeout(200);

    // Progress bar must become visible even though status is "starting"
    const qwenProgressDiv = page.locator("#model-progress-qwen3-asr-1\\.7b");
    await expect(qwenProgressDiv).toBeVisible({ timeout: 3000 });

    // Verify modelSource was passed in the invoke call
    const calls = await page.evaluate(() => (window as any).__downloadCalls);
    const qwenCall = calls.find((c: any) => c.args?.modelId === "qwen3-asr-1.7b");
    expect(qwenCall, "start_model_download should be called for qwen3-asr-1.7b").toBeTruthy();
    expect(qwenCall.args.downloadMethod).toBe("library-managed");
    expect(qwenCall.args.backend).toBe("qwen3-asr");
    expect(qwenCall.args.modelSource).toBe("Qwen/Qwen3-ASR-1.7B");

    console.log(`✓ qwen3-asr download call args: ${JSON.stringify(qwenCall.args)}`);

    // Wait for "downloading" event (300ms) to verify progress updates
    await page.waitForTimeout(400);
    const qwenDetail = page.locator("#pbar-detail-qwen3-asr-1\\.7b");
    const detailText = await qwenDetail.textContent();
    expect(detailText).not.toBe("0 B / 0 B");

    // Wait for "completed" event (600ms) to verify card switches to installed
    await page.waitForTimeout(500);

    // After completed, should show "切换使用" button (not "下载")
    const useBtn = qwenCard.locator(".btn-use-model");
    await expect(useBtn).toBeVisible({ timeout: 3000 });

    // ── MiMo V2.5 ASR: download must also pass modelSource ──────────────────
    const mimoCard = page.locator("#model-card-mimo-v2\\.5-asr");
    await expect(mimoCard).toBeVisible({ timeout: 5000 });

    const mimoDownloadBtn = mimoCard.locator(".btn-download");
    await expect(mimoDownloadBtn).toBeVisible({ timeout: 3000 });
    await mimoDownloadBtn.click();

    await page.waitForTimeout(800); // Wait for all mock events

    const allCalls = await page.evaluate(() => (window as any).__downloadCalls);
    const mimoCall = allCalls.find((c: any) => c.args?.modelId === "mimo-v2.5-asr");
    expect(mimoCall, "start_model_download should be called for mimo-v2.5-asr").toBeTruthy();
    expect(mimoCall.args.downloadMethod).toBe("library-managed");
    expect(mimoCall.args.backend).toBe("mimo");
    expect(mimoCall.args.modelSource).toBe("XiaomiMiMo/MiMo-V2.5-ASR");

    console.log(`✓ mimo download call args: ${JSON.stringify(mimoCall.args)}`);
  });

  test("whisper download still works after library-managed download completes", async ({ page }) => {
    // This test verifies that the progress listener survives across different
    // model downloads — a library-managed model completing should not break
    // subsequent Whisper HTTP download progress updates.

    await page.route("**/node_modules/@tauri-apps/api/core.js", (route) => {
      route.fulfill({
        contentType: "application/javascript",
        body: `
export async function invoke(cmd, args) {
  const bus = window.__tauriTestBus;
  if (bus && bus.onInvoke) return bus.onInvoke(cmd, args);
  return undefined;
}
export function transformCallback(fn) { return fn; }
`,
      });
    });
    await page.route("**/node_modules/@tauri-apps/api/event.js", (route) => {
      route.fulfill({
        contentType: "application/javascript",
        body: `
export async function listen(event, handler) {
  const bus = window.__tauriTestBus;
  if (!bus) return () => {};
  bus.listeners = bus.listeners || {};
  bus.listeners[event] = bus.listeners[event] || [];
  bus.listeners[event].push(handler);
  return () => {
    if (bus.listeners && bus.listeners[event]) {
      bus.listeners[event] = bus.listeners[event].filter(h => h !== handler);
    }
  };
}
export async function emit(event, payload) {
  const bus = window.__tauriTestBus;
  if (!bus || !bus.listeners || !bus.listeners[event]) return;
  for (const h of bus.listeners[event]) h({ payload });
}
`,
      });
    });

    let downloadSequence: Array<{ modelId: string; status: string }> = [];

    await page.addInitScript(() => {
      try { localStorage.clear(); } catch {}
      (window as any).__TAURI_INTERNALS__ = { version: "2.0.0" };
      (window as any).__downloadCalls = [];
      (window as any).__tauriTestBus = {
        listeners: {} as Record<string, Array<(e: { payload: unknown }) => void>>,
        onInvoke: (cmd: string, args: any) => {
          if (cmd === "list_cached_models") return Promise.resolve([]);
          if (cmd === "get_active_model_id") return Promise.resolve("");
          if (cmd === "check_library_model_installed") return Promise.resolve(false);
          if (cmd === "start_model_download") {
            (window as any).__downloadCalls.push({ cmd, args });
            const bus = (window as any).__tauriTestBus;
            const modelId = args.modelId;
            const isLibraryManaged = args.downloadMethod === "library-managed";

            if (isLibraryManaged) {
              // Simulate quick library download: starting → completed
              setTimeout(() => {
                const handlers = bus.listeners?.["model-download-progress"] ?? [];
                for (const h of handlers) {
                  h({ payload: { modelId, downloadedBytes: 0, totalBytes: 100_000_000, speedBps: 0, status: "starting" } });
                }
              }, 100);
              setTimeout(() => {
                const handlers = bus.listeners?.["model-download-progress"] ?? [];
                for (const h of handlers) {
                  h({ payload: { modelId, downloadedBytes: 100_000_000, totalBytes: 100_000_000, speedBps: 0, status: "completed" } });
                }
              }, 300);
            } else {
              // Simulate Whisper HTTP download: downloading → completed
              setTimeout(() => {
                const handlers = bus.listeners?.["model-download-progress"] ?? [];
                for (const h of handlers) {
                  h({ payload: { modelId, downloadedBytes: 5_000_000, totalBytes: 148_000_000, speedBps: 1_048_576, status: "downloading" } });
                }
              }, 150);
              setTimeout(() => {
                const handlers = bus.listeners?.["model-download-progress"] ?? [];
                for (const h of handlers) {
                  h({ payload: { modelId, downloadedBytes: 148_000_000, totalBytes: 148_000_000, speedBps: 0, status: "completed" } });
                }
              }, 500);
            }
            return Promise.resolve(undefined);
          }
          if (cmd === "set_active_model_id") return Promise.resolve(undefined);
          return Promise.resolve(undefined);
        },
      };
    });

    await page.goto("/");
    await page.locator(".nav-btn").filter({ hasText: "模型管理" }).click();
    await expect(page.locator("h2").filter({ hasText: "模型管理" })).toBeVisible();

    // Step 1: Start a library-managed download (paraformer)
    const paraformerCard = page.locator("#model-card-paraformer-zh");
    await expect(paraformerCard).toBeVisible({ timeout: 5000 });
    await paraformerCard.locator(".btn-download").click();

    // Wait for library download to complete
    await page.waitForTimeout(500);

    // Verify paraformer is now installed
    const paraformerUseBtn = paraformerCard.locator(".btn-use-model");
    await expect(paraformerUseBtn).toBeVisible({ timeout: 3000 });
    console.log("✓ Paraformer library download completed, card shows '切换使用'");

    // Step 2: Now start a Whisper HTTP download
    const whisperBaseCard = page.locator("#model-card-whisper-base");
    await expect(whisperBaseCard).toBeVisible({ timeout: 3000 });

    // Whisper should still have download button (not installed)
    const whisperDownloadBtn = whisperBaseCard.locator(".btn-download");
    await expect(whisperDownloadBtn).toBeVisible({ timeout: 3000 });
    await whisperDownloadBtn.click();

    // Progress bar should appear for Whisper
    const whisperProgressDiv = page.locator("#model-progress-whisper-base");
    await expect(whisperProgressDiv).toBeVisible({ timeout: 3000 });

    // Speed readout should update (proves the listener is still active)
    const speedEl = page.locator("#pbar-speed-whisper-base");
    await expect(speedEl).not.toHaveText("— B/s", { timeout: 5000 });

    // Wait for Whisper download to complete
    await page.waitForTimeout(600);

    // Whisper should now show as installed
    const whisperUseBtn = whisperBaseCard.locator(".btn-use-model");
    await expect(whisperUseBtn).toBeVisible({ timeout: 3000 });

    console.log("✓ Whisper download works correctly after library-managed download");
  });
});


