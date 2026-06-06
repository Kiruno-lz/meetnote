import type { AppState } from "../stores/ui-state.ts";
import { saveState } from "../stores/ui-state.ts";
import { navigate } from "../main.ts";
import { injectMeetingItemStyles, renderMeetingItemHtml } from "./meeting-record-item.ts";
import { updateMeetingTitle, deleteMeeting, isTitleValid } from "../utils/meeting-management.ts";

export function renderMarkdownLibrary(root: HTMLElement, state: AppState): void {
  injectMeetingItemStyles();

  const hasItems = state.meetings.length > 0;

  root.innerHTML = `
    <div class="section">
      <div class="section-header">
        <h2>Markdown 库</h2>
      </div>
      <div id="search-bar" style="margin-bottom:12px;">
        <input
          type="text"
          id="search-input"
          placeholder="搜索标题、日期、发言人、关键词..."
          style="width:100%; padding:8px 12px; border:1px solid #333; border-radius:6px; background:#1a1a1a; color:#e0e0e0; font-size:14px; outline:none; box-sizing:border-box;"
        />
      </div>
      <div id="library-list"></div>
      <div style="margin-top:24px;">
        <button class="btn btn-secondary" id="btn-home">返回首页</button>
      </div>
    </div>
  `;

  function getFilteredMeetings(query: string): typeof state.meetings {
    const visibleMeetings = state.pipelineRunning && state.activeMeetingId
      ? state.meetings.filter((m) => m.id !== state.activeMeetingId)
      : state.meetings;

    if (!query.trim()) return visibleMeetings;

    const q = query.toLowerCase().trim();

    return visibleMeetings.filter((m) => {
      // 1. Title match
      if (m.title.toLowerCase().includes(q)) return true;

      // 2. Date match
      if (m.createdAt.toLowerCase().includes(q)) return true;

      // 3. Speaker match
      const speakerLabels = m.speakerProfiles.map((s) => s.label.toLowerCase()).join(" ");
      if (speakerLabels.includes(q)) return true;

      // 4. Content/keyword match (search inside projectSnapshot and markdownContent)
      try {
        const project = JSON.parse(m.projectSnapshot) as Record<string, unknown>;
        const transcriptText = Array.isArray(project.transcript)
          ? (project.transcript as Array<{ text?: string }>).map((s) => s.text ?? "").join(" ")
          : "";
        if (transcriptText.toLowerCase().includes(q)) return true;

        if (m.markdownContent && m.markdownContent.toLowerCase().includes(q)) return true;
      } catch {
        // skip content search if parse fails
      }

      return false;
    });
  }

  function renderList(filtered: typeof state.meetings): void {
    const listEl = root.querySelector<HTMLElement>("#library-list");
    if (!listEl) return;

    const hasItems = filtered.length > 0;
    listEl.innerHTML = hasItems
      ? filtered.map((m) => renderMeetingItemHtml(m, "inline")).join("")
      : `<div class="empty-state">${
          root.querySelector<HTMLInputElement>("#search-input")?.value?.trim()
            ? "无匹配的会议记录"
            : "尚无保存的 Markdown"
        }</div>`;
  }

  function attachCardEvents(): void {
    // Click on card body → navigate to deverbalization-preview
    root.querySelectorAll<HTMLElement>(".meeting-item-body").forEach((el) => {
      el.addEventListener("click", () => {
        const id = el.dataset["id"];
        if (!id) return;
        state.activeMeetingId = id;
        state.activeSection = "deverbalization-preview";
        state.deverPreviewBackTo = "markdown-library";
        saveState(state);
        navigate("deverbalization-preview");
      });
    });

    // Attach edit button events (inline rename)
    root.querySelectorAll<HTMLButtonElement>(".btn-edit").forEach((btn) => {
      btn.addEventListener("click", (event) => {
        event.stopPropagation();
        if (!document.contains(btn)) return;
        const id = btn.dataset["id"];
        if (!id) return;
        const meeting = state.meetings.find((m) => m.id === id);
        if (!meeting) return;

        const card = root.querySelector<HTMLElement>(`.meeting-item[data-id="${CSS.escape(id)}"]`);
        if (!card) return;
        const body = card.querySelector<HTMLElement>(".meeting-item-body");
        if (!body) return;
        const actionsDiv = card.querySelector<HTMLElement>(".meeting-item-actions");
        const editBtn = actionsDiv?.querySelector<HTMLButtonElement>(".btn-edit");

        // Switch to editing state
        card.classList.add("editing");
        if (editBtn) {
          editBtn.className = "icon-btn confirm btn-confirm-edit";
          editBtn.title = "确认";
          editBtn.setAttribute("aria-label", "确认");
          editBtn.innerHTML = `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M2 7l4 4 6-6"/></svg>`;
        }

        // Replace only the title div with an inline input, preserving the date div
        const titleDiv = body.querySelector<HTMLElement>(".meeting-title");
        const dateDiv = body.querySelector<HTMLElement>(".meeting-date");

        const input = document.createElement("input");
        input.type = "text";
        input.className = "edit-title-input";
        input.value = meeting.title;
        input.style.cssText =
          "flex:1; padding:3px 8px; font-size:14px; border:1px solid #4a9eff; border-radius:4px; width:100%; background:#1a1a1a; color:#e0e0e0; outline:none;";

        if (titleDiv) {
          titleDiv.replaceWith(input);
        } else {
          body.insertBefore(input, dateDiv ?? null);
        }
        input.focus();
        input.select();

        function restoreTitleDiv(titleText: string): void {
          const restoredTitle = document.createElement("div");
          restoredTitle.className = "meeting-title";
          restoredTitle.textContent = titleText;
          const editInput = body!.querySelector<HTMLInputElement>(".edit-title-input");
          if (editInput) {
            editInput.replaceWith(restoredTitle);
          }
        }

        function restoreEditBtn(): void {
          card!.classList.remove("editing");
          const confirmBtn = actionsDiv?.querySelector<HTMLButtonElement>(".btn-confirm-edit");
          if (confirmBtn) {
            confirmBtn.className = "icon-btn btn-edit";
            confirmBtn.title = "重命名";
            confirmBtn.setAttribute("aria-label", "重命名");
            confirmBtn.innerHTML = `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M10 2l2 2-7 7H3V9l7-7z"/></svg>`;
            confirmBtn.dataset["id"] = id;
          }
        }

        function submitEdit(): void {
          const newTitle = input.value;
          if (!isTitleValid(newTitle)) {
            input.classList.remove("shake");
            void input.offsetWidth;
            input.classList.add("shake");
            return;
          }
          state.meetings = updateMeetingTitle(state.meetings, id!, newTitle);
          saveState(state);

          // Re-render with current search query preserved
          const searchInput = root.querySelector<HTMLInputElement>("#search-input");
          const query = searchInput?.value ?? "";
          renderList(getFilteredMeetings(query));
          attachCardEvents();
        }

        input.addEventListener("keydown", (e) => {
          if (e.key === "Enter") submitEdit();
          if (e.key === "Escape") {
            restoreTitleDiv(meeting.title);
            restoreEditBtn();
          }
        });

        // Attach confirm click on the swapped button
        if (editBtn) {
          editBtn.onclick = (e) => {
            e.stopPropagation();
            submitEdit();
          };
        }
      });
    });

    // Attach delete button events
    root.querySelectorAll<HTMLButtonElement>(".btn-delete").forEach((btn) => {
      btn.addEventListener("click", (event) => {
        event.stopPropagation();
        const id = btn.dataset["id"];
        if (!id) return;
        const meeting = state.meetings.find((m) => m.id === id);
        if (!meeting) return;

        if (confirm(`确认删除「${meeting.title}」吗？`)) {
          state.meetings = deleteMeeting(state.meetings, id);
          if (state.activeMeetingId === id) {
            state.activeMeetingId = null;
          }
          saveState(state);

          // Re-render with current search query preserved
          const searchInput = root.querySelector<HTMLInputElement>("#search-input");
          const query = searchInput?.value ?? "";
          renderList(getFilteredMeetings(query));
          attachCardEvents();
        }
      });
    });
  }

  // Attach search input listener (real-time filtering)
  const searchInput = root.querySelector<HTMLInputElement>("#search-input");
  searchInput?.addEventListener("input", () => {
    const query = searchInput.value;
    renderList(getFilteredMeetings(query));
    attachCardEvents();
  });

  // Initial render
  renderList(getFilteredMeetings(""));
  attachCardEvents();

  root.querySelector("#btn-home")?.addEventListener("click", () => navigate("project-list"));
}
