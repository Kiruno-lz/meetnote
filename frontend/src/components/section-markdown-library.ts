import type { AppState } from "../stores/ui-state.ts";
import { saveState } from "../stores/ui-state.ts";
import { navigate } from "../main.ts";
import { injectMeetingItemStyles, renderMeetingItemHtml } from "./meeting-record-item.ts";
import { updateMeetingTitle, deleteMeeting, isTitleValid } from "../utils/meeting-management.ts";

export function renderMarkdownLibrary(root: HTMLElement, state: AppState): void {
  injectMeetingItemStyles();

  const visibleMeetings = state.pipelineRunning && state.activeMeetingId
    ? state.meetings.filter((m) => m.id !== state.activeMeetingId)
    : state.meetings;
  const hasItems = visibleMeetings.length > 0;

  const listHtml = hasItems
    ? visibleMeetings
        .map((m) => renderMeetingItemHtml(m, "inline"))
        .join("")
    : `<div class="empty-state">尚无保存的 Markdown</div>`;

  root.innerHTML = `
    <div class="section">
      <div class="section-header">
        <h2>Markdown 库</h2>
      </div>
      <div id="library-list">${listHtml}</div>
      <div style="margin-top:24px;">
        <button class="btn btn-secondary" id="btn-home">返回首页</button>
      </div>
    </div>
  `;

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
        renderMarkdownLibrary(root, state);
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
        renderMarkdownLibrary(root, state);
      }
    });
  });

  root.querySelector("#btn-home")?.addEventListener("click", () => navigate("project-list"));
}
