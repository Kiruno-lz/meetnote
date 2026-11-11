use application::desktop_assistant::{
    AssistantRunResult, AssistantScopeMode, AssistantTaskState, PendingReviewState,
};
use contracts::AgentScene;
use rune::{El, col, row};
use rune_kit::{Button, ButtonVariant, Kit, Markdown, TextInput};

use crate::controls::{caption, card, label, scroll, select, title};
use crate::workspace_host::assistant::ReviewDecision;
use crate::{MeetNote, Route};

const SCENES: [AgentScene; 5] = [
    AgentScene::Ask,
    AgentScene::Organize,
    AgentScene::Partition,
    AgentScene::Extract,
    AgentScene::Move,
];

pub(super) fn page(s: &MeetNote, k: &Kit, overlays: &mut Vec<El>) -> El {
    let host = &s.workspace_host.assistant;
    let controller = &host.controller;
    let mut content = col()
        .w_full()
        .gap(14.0)
        .padding(24.0)
        .child(title(k, s.tr("assistant"), 30.0))
        .child(caption(k, s.tr("assistant_description")));
    if let Some(prompt) = controller.sent_request() {
        content = content.child(card(k).child(label(k, prompt)));
    }
    if let Some(result) = &controller.result {
        if let Some(answer) = &result.answer {
            content = content.child(
                card(k)
                    .child(Markdown::new(answer).build(k))
                    .key("live-assistant-answer"),
            );
        }
        content = content.children(result_details(s, k, result));
        for warning in result.all_warnings() {
            content = content.child(label(k, warning.message));
        }
        for mutation in result.all_mutations() {
            let id = mutation.mutation_id.clone();
            let documents = mutation
                .affected_documents
                .iter()
                .map(|document| document.document_id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let undone = controller.is_mutation_undone(&id);
            let undo_key = format!("live-assistant-undo-{id}");
            content = content.child(
                card(k)
                    .child(caption(
                        k,
                        format!(
                            "{} · {documents}",
                            if undone {
                                s.tr("assistant_undone")
                            } else {
                                s.tr("assistant_receipt_written")
                            }
                        ),
                    ))
                    .child(
                        Button::new(s.tr("assistant_undo"))
                            .variant(ButtonVariant::Outline)
                            .disabled(undone || s.workspace_host.busy())
                            .on_press_tx(move |s: &mut MeetNote, tx| s.undo_live_assistant(&id, tx))
                            .build(k)
                            .key(undo_key),
                    ),
            );
        }
    }
    if let Some(pending) = &controller.pending {
        content = content.child(match pending {
            PendingReviewState::Awaiting(review)
            | PendingReviewState::Accepting(review)
            | PendingReviewState::Rejecting(review) => {
                let mut body = card(k)
                    .child(title(k, s.tr("assistant_review_changes"), 18.0))
                    .child(caption(
                        k,
                        format!("Documents · {}", review.affected_document_ids.join(", ")),
                    ));
                for diff in &review.field_diffs {
                    body = body.child(label(
                        k,
                        format!(
                            "{} · {}\nBefore: {}\nAfter: {}",
                            diff.document_id,
                            diff.field,
                            diff.before
                                .as_deref()
                                .unwrap_or(s.tr("assistant_empty_value")),
                            diff.after
                                .as_deref()
                                .unwrap_or(s.tr("assistant_empty_value")),
                        ),
                    ));
                }
                for change in &review.tree_changes {
                    body = body.child(label(
                        k,
                        format!(
                            "{} · Parent {} → {} · Order {} → {}",
                            change.document_id,
                            change.previous_parent_id.as_deref().unwrap_or("Root"),
                            change.new_parent_id.as_deref().unwrap_or("Root"),
                            change
                                .previous_sibling_order
                                .map_or_else(|| "—".into(), |value| value.to_string()),
                            change
                                .new_sibling_order
                                .map_or_else(|| "—".into(), |value| value.to_string()),
                        ),
                    ));
                }
                for range in &review.ranges {
                    body = body.child(caption(
                        k,
                        format!(
                            "{} · {} → {}",
                            range.document_id.as_deref().unwrap_or("Source"),
                            range.start_segment_id,
                            range.end_segment_id,
                        ),
                    ));
                }
                if !review.uncovered_segment_ids.is_empty() {
                    body = body.child(caption(
                        k,
                        format!(
                            "Uncovered segments · {}",
                            review.uncovered_segment_ids.join(", ")
                        ),
                    ));
                }
                body.child(
                    row()
                        .gap(8.0)
                        .child(
                            Button::new(s.tr("assistant_accept"))
                                .disabled(s.workspace_host.busy())
                                .on_press_tx(|s: &mut MeetNote, tx| {
                                    s.decide_live_review(ReviewDecision::Confirm, tx)
                                })
                                .build(k)
                                .key("live-assistant-approve"),
                        )
                        .child(
                            Button::new(s.tr("assistant_reject"))
                                .variant(ButtonVariant::Outline)
                                .disabled(s.workspace_host.busy())
                                .on_press_tx(|s: &mut MeetNote, tx| {
                                    s.decide_live_review(ReviewDecision::Reject, tx)
                                })
                                .build(k)
                                .key("live-assistant-reject"),
                        ),
                )
            }
            PendingReviewState::Expired => caption(k, s.tr("assistant_expired")),
            PendingReviewState::Rejected => caption(k, s.tr("assistant_rejected")),
        });
    }
    match &controller.task {
        AssistantTaskState::Failed(error) | AssistantTaskState::Partial(error) => {
            content = content.child(label(k, error.to_string()));
        }
        AssistantTaskState::Cancelled => {
            content = content.child(caption(k, s.tr("assistant_cancelled")))
        }
        AssistantTaskState::Running { .. } => {
            let status = controller
                .progress
                .as_ref()
                .map(|progress| progress.stage.as_str())
                .unwrap_or(s.tr("assistant_waiting"));
            content = content.child(caption(k, status));
        }
        AssistantTaskState::Cancelling { .. } => {
            content = content.child(caption(k, s.tr("assistant_cancelling")))
        }
        _ => {}
    }
    let configured = host.settings.enabled && host.settings.active_connection_id.is_some();
    if !configured {
        content = content.child(
            card(k)
                .child(label(k, s.tr("assistant_connection_required")))
                .child(
                    Button::new(s.tr("assistant_open_settings"))
                        .variant(ButtonVariant::Outline)
                        .on_press(|s: &mut MeetNote| s.navigate(Route::Settings))
                        .build(k),
                ),
        );
    }
    let all_documents = controller.scope_mode == AssistantScopeMode::AllVisibleDocuments;
    let scope = if all_documents {
        format!("{} documents", s.documents.document_states().count())
    } else {
        s.documents
            .selected_state()
            .map_or(s.tr("assistant_no_document").into(), |document| {
                document.title().into()
            })
    };
    let submit = configured
        && s.documents.has_selection()
        && !s.workspace_host.busy()
        && !host.awaiting_review();
    let composer = card(k)
        .mx(24.0)
        .mb(14.0)
        .child(
            row()
                .gap(8.0)
                .flex_wrap()
                .items_center()
                .child(select(
                    s,
                    k,
                    overlays,
                    ("live-assistant-scene", s.tr("assistant_scene")),
                    &[
                        s.tr("assistant_scene_ask"),
                        s.tr("assistant_scene_organize"),
                        s.tr("assistant_scene_partition"),
                        s.tr("assistant_scene_extract"),
                        s.tr("assistant_scene_move"),
                    ],
                    SCENES
                        .iter()
                        .position(|scene| *scene == controller.scene)
                        .unwrap_or_default(),
                    |s, index| {
                        s.workspace_host
                            .assistant
                            .controller
                            .select_scene(SCENES[index])
                    },
                ))
                .child(select(
                    s,
                    k,
                    overlays,
                    ("live-assistant-scope", s.tr("assistant_scope")),
                    &[
                        s.tr("assistant_current_document"),
                        s.tr("assistant_all_visible_documents"),
                    ],
                    usize::from(all_documents),
                    |s, index| {
                        s.workspace_host.assistant.controller.scope_mode = if index == 0 {
                            AssistantScopeMode::CurrentDocument
                        } else {
                            AssistantScopeMode::AllVisibleDocuments
                        }
                    },
                ))
                .child(caption(k, scope)),
        )
        .child(
            row()
                .gap(8.0)
                .items_center()
                .child(
                    TextInput::new(&host.prompt)
                        .label(s.tr("assistant_prompt"))
                        .placeholder(s.tr("assistant_prompt_placeholder"))
                        .on_edit(|s: &mut MeetNote| &mut s.workspace_host.assistant.prompt)
                        .on_submit(MeetNote::run_live_assistant)
                        .build(k)
                        .key("live-assistant-input")
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(
                    if matches!(
                        controller.task,
                        AssistantTaskState::Running { .. } | AssistantTaskState::Cancelling { .. }
                    ) {
                        Button::new(s.tr("cancel"))
                            .on_press_tx(MeetNote::cancel_live_assistant)
                            .build(k)
                    } else {
                        Button::new(s.tr("send"))
                            .disabled(!submit || host.prompt.text.trim().is_empty())
                            .on_press_tx(MeetNote::run_live_assistant)
                            .build(k)
                            .key("live-assistant-send")
                    },
                ),
        );
    col()
        .key("assistant-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(scroll(k, "live-assistant-scroll", content))
        .child(composer)
}

fn result_details(s: &MeetNote, k: &Kit, result: &AssistantRunResult) -> Vec<El> {
    let mut details = Vec::new();
    let line = |key, value| caption(k, format!("{} · {value}", s.tr(key))).selectable(true);
    if !result.evidence_segment_ids.is_empty() {
        details.push(line(
            "assistant_evidence",
            result.evidence_segment_ids.join(", "),
        ));
    }
    let model = match (
        result.requested_model.as_deref(),
        result.resolved_model.as_deref(),
    ) {
        (Some(requested), Some(resolved)) if requested != resolved => {
            Some(format!("{requested} → {resolved}"))
        }
        (Some(model), _) | (None, Some(model)) => Some(model.to_owned()),
        (None, None) => None,
    };
    if let Some(model) = model {
        details.push(line("assistant_model_route", model));
    }
    if let Some(usage) = result.usage {
        let mut counts = format!(
            "{} / {} / {}",
            usage.input_tokens, usage.output_tokens, usage.total_tokens
        );
        if let Some(cost) = usage.cost_microusd {
            counts.push_str(&format!(" · USD {:.6}", cost as f64 / 1_000_000.0));
        }
        details.push(line("assistant_usage", counts));
    }
    if let Some(diagnostics) = &result.diagnostics {
        details.push(line(
            "assistant_diagnostics",
            format!(
                "{} · tools={} · retries={} · segments={} · input={}B · {}ms",
                diagnostics.path,
                diagnostics.tool_rounds,
                diagnostics.validation_retries,
                diagnostics.input_segments,
                diagnostics.input_bytes,
                diagnostics.elapsed_ms,
            ),
        ));
    }
    details
}
