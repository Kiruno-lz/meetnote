use crate::{
    assistant_ui::{
        AssistantController, AssistantMutationSummary, AssistantRunResult, AssistantScopeMode,
        AssistantTaskState, AssistantUiError, PendingReviewState,
    },
    controller::RootView,
    i18n::{text as tr, Locale},
    theme::{self, Tokens},
};
use contracts::{AgentScene, DocumentProjection};
use gpui::{div, prelude::*, px, Context, IntoElement};
use gpui_component::{
    button::{Button, ButtonVariants},
    scroll::ScrollableElement as _,
    Disableable,
};

pub(crate) fn render(root: &mut RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let tokens = *theme::tokens(cx);
    let selected = root
        .ui
        .selected_project
        .as_ref()
        .and_then(|id| root.projects.iter().find(|project| project.id() == id));
    let document_summary = match root.assistant.scope_mode {
        AssistantScopeMode::CurrentDocument => selected
            .map(|document| {
                format!(
                    "{} · {} · r{}",
                    document.title(),
                    document.id().as_str(),
                    document.revision()
                )
            })
            .unwrap_or_else(|| tr(root.ui.locale, "assistant_no_document").into()),
        AssistantScopeMode::AllVisibleDocuments => format!(
            "{} · {}",
            tr(root.ui.locale, "assistant_all_visible_documents"),
            root.projects.len()
        ),
    };
    let connection_summary = root
        .saved_assistant()
        .active_connection()
        .map(|profile| {
            format!(
                "{} · {} · {}",
                profile.name,
                profile.model,
                if profile.is_local() {
                    tr(root.ui.locale, "assistant_local_connection")
                } else {
                    tr(root.ui.locale, "assistant_remote_connection")
                }
            )
        })
        .unwrap_or_else(|| tr(root.ui.locale, "assistant_connection_required").into());
    let write_mode = root.saved_assistant().write_mode;
    let task = task_label(root.ui.locale, &root.assistant.task);
    let result = root.assistant.result.clone();
    let pending = root.assistant.pending.clone();

    div()
        .debug_selector(|| "assistant-page".into())
        .flex_1()
        .min_h_0()
        .bg(tokens.content)
        .p_6()
        .flex()
        .flex_col()
        .gap_4()
        .child(
            div()
                .text_2xl()
                .font_weight(gpui::FontWeight::BOLD)
                .child(tr(root.ui.locale, "assistant")),
        )
        .child(div().flex().flex_wrap().gap_2().children([
            scene_button(AgentScene::Ask, root.assistant.scene, root.ui.locale, cx),
            scene_button(
                AgentScene::Organize,
                root.assistant.scene,
                root.ui.locale,
                cx,
            ),
            scene_button(
                AgentScene::Partition,
                root.assistant.scene,
                root.ui.locale,
                cx,
            ),
            scene_button(
                AgentScene::Extract,
                root.assistant.scene,
                root.ui.locale,
                cx,
            ),
            scene_button(AgentScene::Move, root.assistant.scene, root.ui.locale, cx),
        ]))
        .child(
            div()
                .rounded(tokens.radius.md)
                .border_1()
                .border_color(tokens.border)
                .bg(tokens.surface)
                .p_4()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(scope_button(
                            AssistantScopeMode::CurrentDocument,
                            root.assistant.scope_mode,
                            root.ui.locale,
                            cx,
                        ))
                        .child(scope_button(
                            AssistantScopeMode::AllVisibleDocuments,
                            root.assistant.scope_mode,
                            root.ui.locale,
                            cx,
                        )),
                )
                .child(info_line(
                    tr(root.ui.locale, "assistant_authorized_document"),
                    document_summary,
                    &tokens,
                ))
                .child(info_line(
                    tr(root.ui.locale, "assistant_projection"),
                    projection_label(root.assistant.projection).to_string(),
                    &tokens,
                ))
                .child(info_line(
                    tr(root.ui.locale, "assistant_connection"),
                    connection_summary,
                    &tokens,
                ))
                .child(
                    info_line(
                        tr(root.ui.locale, "assistant_write_mode"),
                        tr(
                            root.ui.locale,
                            match write_mode {
                                contracts::AgentWriteMode::Automatic => "assistant_automatic",
                                contracts::AgentWriteMode::ConfirmEach => "assistant_confirm_each",
                            },
                        )
                        .into(),
                        &tokens,
                    )
                    .debug_selector(move || {
                        match write_mode {
                            contracts::AgentWriteMode::Automatic => {
                                "assistant-write-mode-automatic"
                            }
                            contracts::AgentWriteMode::ConfirmEach => {
                                "assistant-write-mode-confirm-each"
                            }
                        }
                        .into()
                    }),
                )
                .child(div().text_xs().text_color(tokens.text_muted).child(tr(
                    root.ui.locale,
                    match root.assistant.scope_mode {
                        AssistantScopeMode::CurrentDocument => "assistant_scope_notice",
                        AssistantScopeMode::AllVisibleDocuments => {
                            "assistant_all_visible_scope_notice"
                        }
                    },
                ))),
        )
        .child(
            div()
                .id("assistant-input")
                .debug_selector(|| "assistant-input".into())
                .w_full()
                .min_h(px(92.))
                .p_4()
                .rounded(tokens.radius.md)
                .border_1()
                .border_color(tokens.accent)
                .bg(tokens.surface)
                .child(root.assistant_inputs.prompt.clone()),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    Button::new("assistant-submit")
                        .debug_selector(|| "assistant-submit".into())
                        .primary()
                        .label(tr(root.ui.locale, "send"))
                        .disabled(matches!(
                            root.assistant.task,
                            AssistantTaskState::Running { .. }
                                | AssistantTaskState::Cancelling { .. }
                        ))
                        .on_click(cx.listener(RootView::assistant_action)),
                )
                .child(
                    Button::new("assistant-cancel")
                        .secondary()
                        .label(tr(root.ui.locale, "cancel"))
                        .disabled(!matches!(
                            root.assistant.task,
                            AssistantTaskState::Running { .. }
                                | AssistantTaskState::Cancelling { .. }
                        ))
                        .on_click(cx.listener(RootView::cancel_assistant)),
                )
                .child(div().text_sm().text_color(tokens.text_muted).child(task)),
        )
        .child(
            div()
                .id("assistant-results")
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .flex()
                .flex_col()
                .gap_3()
                .children(
                    result.map(|result| render_result(result, root.ui.locale, cx, &root.assistant)),
                )
                .children(pending.map(|pending| {
                    render_pending(
                        pending,
                        root.assistant.pending_queue_len(),
                        root.ui.locale,
                        cx,
                    )
                }))
                .children((root.assistant.pending_overflow_count() > 0).then(|| {
                    render_pending_overflow(
                        root.assistant.pending_overflow_count(),
                        root.ui.locale,
                        &tokens,
                    )
                }))
                .child(
                    div()
                        .text_xs()
                        .text_color(tokens.text_subtle)
                        .child(root.ui.notice.clone()),
                ),
        )
}

fn scope_button(
    scope: AssistantScopeMode,
    selected: AssistantScopeMode,
    locale: Locale,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    let label = match scope {
        AssistantScopeMode::CurrentDocument => tr(locale, "assistant_current_document"),
        AssistantScopeMode::AllVisibleDocuments => tr(locale, "assistant_all_visible_documents"),
    };
    let mut button = Button::new(format!("assistant-scope-{label}"))
        .compact()
        .label(label);
    button = if scope == selected {
        button.primary()
    } else {
        button.secondary()
    };
    button
        .on_click(cx.listener(move |root, _, _, cx| root.select_assistant_scope(scope, cx)))
        .into_any_element()
}

fn scene_button(
    scene: AgentScene,
    selected: AgentScene,
    locale: Locale,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    let mut button = Button::new(format!("assistant-scene-{}", scene.skill_id()))
        .label(tr(locale, scene_key(scene)));
    button = if scene == selected {
        button.primary()
    } else {
        button.secondary()
    };
    button
        .on_click(cx.listener(move |root, _, _, cx| root.select_assistant_scene(scene, cx)))
        .into_any_element()
}

fn scene_key(scene: AgentScene) -> &'static str {
    match scene {
        AgentScene::Ask => "assistant_scene_ask",
        AgentScene::Organize => "assistant_scene_organize",
        AgentScene::Partition => "assistant_scene_partition",
        AgentScene::Extract => "assistant_scene_extract",
        AgentScene::Move => "assistant_scene_move",
    }
}

fn projection_label(projection: DocumentProjection) -> &'static str {
    match projection {
        DocumentProjection::Metadata => "metadata",
        DocumentProjection::Markdown => "markdown",
        DocumentProjection::Transcript => "transcript",
        DocumentProjection::SpeakerSegments => "speaker_segments",
        DocumentProjection::Complete => "complete",
    }
}

fn info_line(label: &'static str, value: String, tokens: &Tokens) -> gpui::Div {
    div()
        .flex()
        .gap_2()
        .text_sm()
        .child(div().text_color(tokens.text_muted).child(label))
        .child(value)
}

fn task_label(locale: Locale, task: &AssistantTaskState) -> String {
    match task {
        AssistantTaskState::Idle => tr(locale, "assistant_idle").into(),
        AssistantTaskState::Running { request_id } => {
            format!("{} · {request_id}", tr(locale, "assistant_running"))
        }
        AssistantTaskState::Cancelling { request_id } => {
            format!("{} · {request_id}", tr(locale, "assistant_cancelling"))
        }
        AssistantTaskState::AwaitingConfirmation => {
            tr(locale, "assistant_awaiting_confirmation").into()
        }
        AssistantTaskState::Completed => tr(locale, "assistant_completed").into(),
        AssistantTaskState::Partial(error) => {
            format!("{} · {error}", tr(locale, "assistant_partial"))
        }
        AssistantTaskState::Failed(error) => format!("{} · {error}", tr(locale, "failed")),
        AssistantTaskState::Cancelled => tr(locale, "assistant_cancelled").into(),
    }
}

fn render_result(
    result: AssistantRunResult,
    locale: Locale,
    cx: &mut Context<RootView>,
    controller: &AssistantController,
) -> gpui::AnyElement {
    let answer = result
        .answer
        .clone()
        .unwrap_or_else(|| tr(locale, "assistant_no_answer").into());
    let evidence = if result.evidence_segment_ids.is_empty() {
        tr(locale, "assistant_no_evidence").into()
    } else {
        result.evidence_segment_ids.join(", ")
    };
    let model_route = format!(
        "{} → {}",
        result.requested_model.as_deref().unwrap_or("—"),
        result.resolved_model.as_deref().unwrap_or("—")
    );
    let usage = result
        .usage
        .map(|usage| {
            format!(
                "{} / {} / {} · USD {}",
                usage.input_tokens,
                usage.output_tokens,
                usage.total_tokens,
                usage
                    .cost_microusd
                    .map(|cost| format!("{:.6}", cost as f64 / 1_000_000.0))
                    .unwrap_or_else(|| "—".into())
            )
        })
        .unwrap_or_else(|| "—".into());
    let diagnostics = result
        .diagnostics
        .as_ref()
        .map(|diagnostics| {
            format!(
                "{} · tools={} · retries={} · segments={} · input={}B · {}ms",
                diagnostics.path,
                diagnostics.tool_rounds,
                diagnostics.validation_retries,
                diagnostics.input_segments,
                diagnostics.input_bytes,
                diagnostics.elapsed_ms,
            )
        })
        .unwrap_or_else(|| "—".into());
    let render_model = result_render_model(&result);
    let tokens = *theme::tokens(cx);

    div()
        .rounded(tokens.radius.md)
        .border_1()
        .border_color(tokens.border)
        .p_4()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_lg().child(answer))
        .child(info_line(
            tr(locale, "assistant_evidence"),
            evidence,
            &tokens,
        ))
        .child(info_line(
            tr(locale, "assistant_model_route"),
            model_route,
            &tokens,
        ))
        .child(info_line(tr(locale, "assistant_usage"), usage, &tokens))
        .child(info_line(
            tr(locale, "assistant_diagnostics"),
            diagnostics,
            &tokens,
        ))
        .children(
            render_model
                .warnings
                .into_iter()
                .map(|warning| render_warning(warning, locale, &tokens)),
        )
        .children(render_model.mutations.into_iter().map(|mutation| {
            let mutation_id = mutation.mutation_id.clone();
            render_mutation(
                mutation,
                locale,
                cx,
                controller.is_mutation_undone(&mutation_id),
                controller.is_mutation_undo_in_flight(&mutation_id),
            )
        }))
        .into_any_element()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AssistantResultRenderModel {
    warnings: Vec<AssistantUiError>,
    mutations: Vec<AssistantMutationSummary>,
}

fn result_render_model(result: &AssistantRunResult) -> AssistantResultRenderModel {
    AssistantResultRenderModel {
        warnings: result.all_warnings(),
        mutations: result.all_mutations(),
    }
}

fn render_warning(warning: AssistantUiError, locale: Locale, tokens: &Tokens) -> gpui::AnyElement {
    div()
        .mt_2()
        .p_3()
        .rounded(tokens.radius.sm)
        .bg(tokens.warning_surface)
        .text_color(tokens.warning_text)
        .child(info_line(
            tr(locale, "assistant_warning"),
            format!("{}: {}", warning.code, warning.message),
            tokens,
        ))
        .into_any_element()
}

fn render_mutation(
    mutation: AssistantMutationSummary,
    locale: Locale,
    cx: &mut Context<RootView>,
    undone: bool,
    undo_in_flight: bool,
) -> gpui::AnyElement {
    let affected = mutation
        .affected_documents
        .iter()
        .map(|change| {
            format!(
                "{}: {:?} → {:?}{}",
                change.document_id,
                change.previous_revision,
                change.revision,
                if change.deleted { " (deleted)" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let gaps = if mutation.uncovered_segment_ids.is_empty() {
        "—".into()
    } else {
        mutation.uncovered_segment_ids.join(", ")
    };
    let mutation_id = mutation.mutation_id.clone();
    let tokens = *theme::tokens(cx);
    let undo_label = if undone {
        tr(locale, "assistant_undone")
    } else if undo_in_flight {
        tr(locale, "assistant_undoing")
    } else {
        tr(locale, "assistant_undo")
    };

    div()
        .mt_2()
        .p_3()
        .rounded(tokens.radius.sm)
        .bg(tokens.raised)
        .flex()
        .flex_col()
        .gap_2()
        .child(info_line(
            tr(locale, "assistant_mutation"),
            mutation.mutation_id,
            &tokens,
        ))
        .child(info_line(
            tr(locale, "assistant_affected"),
            affected,
            &tokens,
        ))
        .child(info_line(tr(locale, "assistant_uncovered"), gaps, &tokens))
        .child(
            div()
                .id(format!("assistant-undo-{mutation_id}"))
                .debug_selector({
                    let selector = mutation_id.clone();
                    move || format!("assistant-undo-{selector}")
                })
                .child(
                    Button::new(format!("assistant-undo-button-{mutation_id}"))
                        .secondary()
                        .label(undo_label)
                        .disabled(undone || undo_in_flight)
                        .on_click(cx.listener(move |root, _, _window, cx| {
                            root.undo_assistant_mutation_for(mutation_id.clone(), cx)
                        })),
                ),
        )
        .into_any_element()
}

fn render_pending(
    state: PendingReviewState,
    queued_count: usize,
    locale: Locale,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    let tokens = *theme::tokens(cx);
    let busy = matches!(
        &state,
        PendingReviewState::Accepting(_) | PendingReviewState::Rejecting(_)
    );
    let (PendingReviewState::Awaiting(pending)
    | PendingReviewState::Accepting(pending)
    | PendingReviewState::Rejecting(pending)) = state
    else {
        let label = match state {
            PendingReviewState::Rejected => tr(locale, "assistant_rejected"),
            PendingReviewState::Expired => tr(locale, "assistant_expired"),
            _ => unreachable!(),
        };
        return div()
            .p_3()
            .text_color(tokens.text_muted)
            .child(label)
            .into_any_element();
    };
    let fields = pending
        .field_diffs
        .iter()
        .map(|diff| {
            format!(
                "{} · {}: {} → {}",
                diff.document_id,
                diff.field,
                diff.before.as_deref().unwrap_or("∅"),
                diff.after.as_deref().unwrap_or("∅")
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let tree = pending
        .tree_changes
        .iter()
        .map(|change| {
            format!(
                "{}: {:?}/#{:?} → {:?}/#{:?}",
                change.document_id,
                change.previous_parent_id,
                change.previous_sibling_order,
                change.new_parent_id,
                change.new_sibling_order
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let ranges = pending
        .ranges
        .iter()
        .map(|range| {
            format!(
                "{}: {}..{} [{}]",
                range.document_id.as_deref().unwrap_or("new"),
                range.start_segment_id,
                range.end_segment_id,
                range.evidence_segment_ids.join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let gaps = pending.uncovered_segment_ids.join(", ");

    div()
        .rounded(tokens.radius.md)
        .border_1()
        .border_color(tokens.warning)
        .p_4()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_lg().child(tr(locale, "assistant_pending")))
        .child(info_line(
            tr(locale, "assistant_pending_queue"),
            queued_count.to_string(),
            &tokens,
        ))
        .child(info_line(
            tr(locale, "assistant_affected"),
            pending.affected_document_ids.join(", "),
            &tokens,
        ))
        .child(info_line(
            tr(locale, "assistant_field_diff"),
            empty_as_dash(fields),
            &tokens,
        ))
        .child(info_line(
            tr(locale, "assistant_tree_change"),
            empty_as_dash(tree),
            &tokens,
        ))
        .child(info_line(
            tr(locale, "assistant_range"),
            empty_as_dash(ranges),
            &tokens,
        ))
        .child(info_line(
            tr(locale, "assistant_uncovered"),
            empty_as_dash(gaps),
            &tokens,
        ))
        .child(
            div()
                .flex()
                .gap_2()
                .child(
                    Button::new("assistant-confirm")
                        .primary()
                        .label(tr(locale, "assistant_accept"))
                        .disabled(busy)
                        .on_click(cx.listener(RootView::accept_assistant_pending)),
                )
                .child(
                    Button::new("assistant-reject")
                        .secondary()
                        .label(tr(locale, "assistant_reject"))
                        .disabled(busy)
                        .on_click(cx.listener(RootView::reject_assistant_pending)),
                ),
        )
        .into_any_element()
}

fn render_pending_overflow(count: usize, locale: Locale, tokens: &Tokens) -> gpui::AnyElement {
    div()
        .id("assistant-pending-overflow")
        .debug_selector(|| "assistant-pending-overflow".into())
        .p_3()
        .child(info_line(
            tr(locale, "assistant_pending_overflow"),
            count.to_string(),
            tokens,
        ))
        .into_any_element()
}

fn empty_as_dash(value: String) -> String {
    if value.is_empty() {
        "—".into()
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn warning(code: &str) -> AssistantUiError {
        AssistantUiError {
            code: code.into(),
            message: format!("message for {code}"),
            retryable: true,
        }
    }

    fn mutation(id: &str) -> AssistantMutationSummary {
        AssistantMutationSummary {
            mutation_id: id.into(),
            affected_documents: Vec::new(),
            undo_token: format!("undo-{id}"),
            uncovered_segment_ids: Vec::new(),
        }
    }

    #[test]
    fn result_render_model_keeps_all_warnings_and_mutations_for_individual_actions() {
        let result = AssistantRunResult {
            answer: Some("answer".into()),
            warning: Some(warning("primary")),
            warnings: vec![warning("secondary")],
            evidence_segment_ids: Vec::new(),
            usage: None,
            requested_model: None,
            resolved_model: None,
            mutation: Some(mutation("primary-mutation")),
            mutations: vec![mutation("first"), mutation("second")],
            pending: None,
            pending_reviews: Vec::new(),
            diagnostics: None,
        };

        let render_model = result_render_model(&result);
        assert_eq!(render_model.warnings.len(), 2);
        assert_eq!(
            render_model
                .mutations
                .into_iter()
                .map(|mutation| mutation.mutation_id)
                .collect::<Vec<_>>(),
            ["first", "second", "primary-mutation"]
        );
    }

    #[test]
    fn partial_task_label_preserves_warning_code_for_non_visual_regression() {
        let label = task_label(
            Locale::EnUs,
            &AssistantTaskState::Partial(warning("PARTIAL_RESULT")),
        );
        assert!(label.contains("partially"));
        assert!(label.contains("PARTIAL_RESULT"));
    }
}
