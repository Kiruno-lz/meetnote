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
});


