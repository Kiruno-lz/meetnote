use std::time::Duration;

use rune::{El, TextEdit, Tx, col, el, row};
use rune_kit::{
    Badge, BadgeVariant, Button, ButtonVariant, Kit, Markdown, Progress, TextInput, glyphs,
};

use crate::controls::{action, caption, card, eyebrow, label, page_header, scroll, select, title};
use crate::{Confirm, MeetNote};

mod live;

const SUGGESTIONS: [&str; 2] = ["Summarize this document", "Draft a recovery action item"];

enum RunState {
    Idle,
    Running(f64),
    Answer,
    Pending,
    Rejected,
    Applied { before: String, after: String },
    Undone,
    Cancelled,
}

pub struct Assistant {
    prompt: TextEdit,
    request: String,
    target: String,
    target_title: String,
    scope: usize,
    request_scope: usize,
    state: RunState,
}

impl Default for Assistant {
    fn default() -> Self {
        Self {
            prompt: TextEdit::new(""),
            request: String::new(),
            target: "product".into(),
            target_title: "Product sync".into(),
            scope: 0,
            request_scope: 0,
            state: RunState::Idle,
        }
    }
}

impl Assistant {
    pub fn sample_review(&mut self) {
        self.request = SUGGESTIONS[1].into();
        self.state = RunState::Pending;
    }

    fn poll(&mut self, now: f64, tx: &mut Tx) -> bool {
        let RunState::Running(until) = self.state else {
            return false;
        };
        if now < until {
            tx.wake_after(Duration::from_secs_f64(until - now));
            return false;
        }
        self.state = if self.request.to_lowercase().contains("action") {
            RunState::Pending
        } else {
            RunState::Answer
        };
        tx.scroll_to_bottom("assistant-scroll");
        true
    }
}

impl MeetNote {
    pub fn poll_assistant(&mut self, now: f64, tx: &mut Tx) -> bool {
        let answered = self.assistant.poll(now, tx);
        if answered && !self.settings.confirm_writes {
            self.apply_assistant(tx);
        }
        answered
    }

    fn send_assistant(&mut self, tx: &mut Tx) {
        let a = &mut self.assistant;
        let input = a.prompt.text.trim();
        if input.is_empty() || matches!(a.state, RunState::Running(_) | RunState::Pending) {
            return;
        }
        let (id, title) = self.documents.selection();
        a.target = id.into();
        a.target_title = title.into();
        a.request_scope = a.scope;
        a.request = input.into();
        a.prompt.clear();
        a.state = RunState::Running(self.now + 0.6);
        tx.wake_after(Duration::from_millis(600));
    }

    pub fn apply_assistant(&mut self, tx: &mut Tx) {
        if matches!(self.assistant.state, RunState::Pending)
            && let Some((before, after)) = self.documents.append_action(&self.assistant.target)
        {
            self.assistant.state = RunState::Applied { before, after };
            self.notify("Action item added", tx);
        }
    }

    fn undo_assistant(&mut self, tx: &mut Tx) {
        if let RunState::Applied { before, after } = &self.assistant.state {
            if self
                .documents
                .restore_notes(&self.assistant.target, before, after)
            {
                self.assistant.state = RunState::Undone;
                self.notify("Assistant change undone", tx);
            } else {
                self.notify("Notes changed after this action", tx);
            }
        }
    }
}

pub fn page(s: &MeetNote, k: &Kit, overlays: &mut Vec<El>) -> El {
    if s.workspace_host.live.is_some() {
        return live::page(s, k, overlays);
    }
    let a = &s.assistant;
    let (_, selected_title) = s.documents.selection();
    let intro = row()
        .items_center()
        .justify_between()
        .gap(14.0)
        .child(
            page_header(
                k,
                "ASSISTANT  /  WORKSPACE",
                "Assistant",
                "Ask about a document or review a proposed change.",
            )
            .grow(1.0)
            .min_w(0.0),
        )
        .child(
            Badge::new("Sample connection")
                .variant(BadgeVariant::Accent)
                .build(k),
        );
    let transcript = col().w_full().gap(18.0).padding(24.0).child(intro)
        .child(match a.state {
            RunState::Idle => {
                let mut choices = row().gap(8.0).flex_wrap();
                for (index, suggestion) in SUGGESTIONS.into_iter().enumerate() {
                    choices = choices.child(action(k, &format!("suggestion-{index}"), suggestion, ButtonVariant::Outline, move |s| {
                        s.assistant.prompt.replace_text(suggestion);
                        s.assistant.prompt.focused = true;
                    }));
                }
                card(k).child(eyebrow(k, "START HERE"))
                    .child(title(k, "Work with your notes", 20.0))
                    .child(caption(k, "The preview returns a sample response and can apply an action item to the in-memory notes."))
                    .child(choices)
            }
            _ => response(s, k),
        });
    let busy = matches!(a.state, RunState::Running(_) | RunState::Pending);
    let composer = card(k)
        .gap(9.0)
        .padding(12.0)
        .child(
            row()
                .items_center()
                .justify_between()
                .gap(10.0)
                .child(caption(
                    k,
                    if a.scope == 0 {
                        format!("Scope · {selected_title}")
                    } else {
                        format!("Workspace context · Writes to {selected_title}")
                    },
                ))
                .child(select(
                    s,
                    k,
                    overlays,
                    ("assistant-scope", "Assistant scope"),
                    &["Selected document", "Workspace context"],
                    a.scope,
                    |s, index| s.assistant.scope = index,
                )),
        )
        .child(
            row()
                .w_full()
                .items_center()
                .gap(8.0)
                .child(
                    TextInput::new(&a.prompt)
                        .placeholder("Ask about this workspace…")
                        .label("Assistant prompt")
                        .width(None)
                        .leading_icon(&glyphs::SPARKLE)
                        .on_edit(|s: &mut MeetNote| &mut s.assistant.prompt)
                        .on_submit(MeetNote::send_assistant)
                        .build(k)
                        .key("assistant-prompt")
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(
                    Button::new("Send")
                        .icon(&glyphs::ARROW_RIGHT)
                        .variant(ButtonVariant::Submit)
                        .disabled(busy || a.prompt.text.trim().is_empty())
                        .on_press_tx(MeetNote::send_assistant)
                        .build(k)
                        .key("assistant-send"),
                ),
        );
    let composer = if s.settings.glass {
        composer.backdrop_blur(8.0).fill(k.c.layer_01.alpha(0.88))
    } else {
        composer
    };
    col()
        .key("assistant-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(scroll(k, "assistant-scroll", transcript))
        .child(col().px(24.0).py(14.0).child(composer))
}

fn response(s: &MeetNote, k: &Kit) -> El {
    let a = &s.assistant;
    let content = match &a.state {
        RunState::Running(_) => col()
            .gap(12.0)
            .child(
                Progress::new(None)
                    .label("Preparing sample response")
                    .build(k),
            )
            .child(action(
                k,
                "assistant-cancel",
                "Cancel",
                ButtonVariant::Outline,
                |s| {
                    s.assistant.state = RunState::Cancelled;
                },
            )),
        RunState::Answer => {
            let scope = if a.request_scope == 0 {
                format!("Selected document · {}", a.target_title)
            } else {
                "Sample workspace context".into()
            };
            col().gap(8.0).child(caption(k, scope))
                .child(Markdown::new("### Sample response\n\n- Review the pilot checklist.\n- Share the recovery walkthrough.")
                    .selectable(true).build(k))
        }
        RunState::Pending => col()
            .gap(12.0)
            .child(
                Badge::new("Review required")
                    .variant(BadgeVariant::Accent)
                    .build(k),
            )
            .child(title(k, "Append an action item", 19.0))
            .child(caption(k, format!("{} · Saved notes", a.target_title)))
            .child(label(k, "+ Share the workspace recovery checklist."))
            .child(
                row()
                    .gap(8.0)
                    .child(action(
                        k,
                        "assistant-accept",
                        "Review and apply",
                        ButtonVariant::Submit,
                        |s| {
                            s.confirm = Some(Confirm::ApplyAssistant);
                        },
                    ))
                    .child(action(
                        k,
                        "assistant-reject",
                        "Reject",
                        ButtonVariant::Outline,
                        |s| {
                            s.assistant.state = RunState::Rejected;
                        },
                    )),
            ),
        RunState::Applied { .. } => row()
            .items_center()
            .gap(12.0)
            .child(label(k, "Action item appended."))
            .child(
                Button::new("Undo")
                    .variant(ButtonVariant::Outline)
                    .on_press_tx(MeetNote::undo_assistant)
                    .build(k)
                    .key("assistant-undo"),
            ),
        RunState::Rejected => caption(k, "Proposed change rejected."),
        RunState::Undone => caption(k, "The original notes were restored."),
        RunState::Cancelled => caption(k, "Sample request cancelled."),
        RunState::Idle => el(),
    };
    col()
        .w_full()
        .gap(12.0)
        .child(card(k).child(eyebrow(k, "YOU")).child(label(k, &a.request)))
        .child(
            card(k)
                .child(eyebrow(k, "MEETNOTE ASSISTANT"))
                .child(content),
        )
}
