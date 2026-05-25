import type { AppState } from "../stores/ui-state.ts";
import { saveState } from "../stores/ui-state.ts";

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

export function renderSettings(root: HTMLElement, state: AppState): void {
  root.innerHTML = `
    <div class="section">
      <div class="section-header">
        <h2>设置</h2>
      </div>

      <!-- Setting: Transcription Language -->
      <div class="card" style="margin-bottom:12px;">
        <div style="display:flex;justify-content:space-between;align-items:flex-start;gap:24px;">
          <div style="flex:1;min-width:0;">
            <div style="font-weight:600;color:#e0e0e0;margin-bottom:4px;">转录目标语言</div>
            <div style="font-size:12px;color:#888;line-height:1.5;">
              选择音频主要语言。中文（zh）支持中英混合转录。
            </div>
          </div>
          <div style="flex-shrink:0;">
            <select id="setting-language" style="background:#1a1a1a;border:1px solid #3a3a3a;border-radius:6px;color:#e0e0e0;padding:6px 10px;font-size:13px;outline:none;cursor:pointer;transition:border-color 0.15s;">
              <option value="zh"${state.transcriptionLanguage === "zh" ? " selected" : ""}>中文（zh）</option>
              <option value="en"${state.transcriptionLanguage === "en" ? " selected" : ""}>英语（en）</option>
              <option value="ja"${state.transcriptionLanguage === "ja" ? " selected" : ""}>日语（ja）</option>
              <option value="auto"${state.transcriptionLanguage === "auto" ? " selected" : ""}>自动检测（auto）</option>
            </select>
          </div>
        </div>
      </div>

      <!-- Setting: Transcription Device -->
      <div class="card" style="margin-bottom:12px;">
        <div style="display:flex;justify-content:space-between;align-items:flex-start;gap:24px;">
          <div style="flex:1;min-width:0;">
            <div style="font-weight:600;color:#e0e0e0;margin-bottom:4px;">转录加速</div>
            <div style="font-size:12px;color:#888;line-height:1.5;">
              Metal GPU 需要 Apple Silicon Mac（M 系列芯片）。自动模式会优先使用 Metal GPU。
            </div>
          </div>
          <div style="flex-shrink:0;">
            <select id="setting-device" style="background:#1a1a1a;border:1px solid #3a3a3a;border-radius:6px;color:#e0e0e0;padding:6px 10px;font-size:13px;outline:none;cursor:pointer;transition:border-color 0.15s;">
              <option value="auto"${state.transcriptionDevice === "auto" ? " selected" : ""}>自动（auto）</option>
              <option value="cpu"${state.transcriptionDevice === "cpu" ? " selected" : ""}>CPU</option>
              <option value="mps"${state.transcriptionDevice === "mps" ? " selected" : ""}>Metal GPU（mps）</option>
            </select>
          </div>
        </div>
      </div>

      <!-- Setting: Markdown Library Path -->
      <div class="card" style="margin-bottom:12px;">
        <div style="margin-bottom:8px;">
          <div style="font-weight:600;color:#e0e0e0;margin-bottom:4px;">Markdown 库保存位置</div>
          <div style="font-size:12px;color:#888;line-height:1.5;margin-bottom:12px;">
            生成的 Markdown 文件将保存到此目录。
          </div>
        </div>
        <div style="display:flex;gap:8px;align-items:center;">
          <input
            id="setting-library-path"
            type="text"
            value="${escapeHtml(state.markdownLibraryPath)}"
            placeholder="~/Documents/meetnote"
            style="flex:1;background:#1a1a1a;border:1px solid #3a3a3a;border-radius:6px;color:#e0e0e0;padding:7px 10px;font-size:13px;font-family:monospace;outline:none;transition:border-color 0.15s;"
          />
          <button id="btn-save-library-path" class="btn btn-secondary btn-sm" style="flex-shrink:0;">保存</button>
        </div>
        <div id="library-path-status" style="margin-top:6px;font-size:12px;color:#66bb6a;display:none;">已保存</div>
      </div>

      <!-- Version info footer -->
      <div style="margin-top:32px;padding-top:16px;border-top:1px solid #2a2a2a;color:#555;font-size:12px;">
        meetnote · 设置会自动保存到本地存储
      </div>
    </div>
  `;

  // Language select
  const langSelect = root.querySelector<HTMLSelectElement>("#setting-language");
  langSelect?.addEventListener("change", () => {
    state.transcriptionLanguage = langSelect.value;
    saveState(state);
  });

  // Device select
  const deviceSelect = root.querySelector<HTMLSelectElement>("#setting-device");
  deviceSelect?.addEventListener("change", () => {
    state.transcriptionDevice = deviceSelect.value;
    saveState(state);
  });

  // Markdown library path — focus style
  const pathInput = root.querySelector<HTMLInputElement>("#setting-library-path");
  pathInput?.addEventListener("focus", () => {
    if (pathInput) pathInput.style.borderColor = "#4a9eff";
  });
  pathInput?.addEventListener("blur", () => {
    if (pathInput) pathInput.style.borderColor = "#3a3a3a";
  });

  // Save button
  const saveBtn = root.querySelector<HTMLButtonElement>("#btn-save-library-path");
  const statusEl = root.querySelector<HTMLElement>("#library-path-status");

  function saveLibraryPath(): void {
    if (!pathInput) return;
    const newPath = pathInput.value.trim() || "~/Documents/meetnote";
    pathInput.value = newPath;
    state.markdownLibraryPath = newPath;
    saveState(state);
    if (statusEl) {
      statusEl.style.display = "block";
      setTimeout(() => {
        if (statusEl) statusEl.style.display = "none";
      }, 2000);
    }
  }

  saveBtn?.addEventListener("click", saveLibraryPath);
  pathInput?.addEventListener("keydown", (e) => {
    if (e.key === "Enter") saveLibraryPath();
  });
}
