use crate::{
    assistant_ui::{
        AssistantController, AssistantMutationSummary, AssistantRunResult, AssistantScopeMode,
        AssistantTaskState, AssistantUiError, PendingReviewState,
    },
    controller::RootView,
    controls::{Button, RadioGroup, Region, Scrollbar},
    i18n::{text as tr, Locale},
    theme::{self, BlurLevel, Tokens},
};
use contracts::{AgentScene, DocumentProjection};
use gpui::{div, prelude::*, px, Context, FocusHandle, IntoElement, Pixels, SharedString};

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
    let task_selector = task_selector(&root.assistant.task);
    let job_progress = root
        .assistant
        .job_progress
        .map(|progress| job_progress_label(root.ui.locale, progress));
    let result = root.assistant.result.clone();
    let render_model = result.as_ref().map(result_render_model);
    let (warnings, receipts) = render_model
        .map(|model| (model.warnings, model.mutations))
        .unwrap_or_default();
    let pending = root.assistant.pending.clone().map(|state| PendingCard {
        state,
        queued: root.assistant.pending_queue_len(),
        position: root.assistant.pending_position(),
    });
    let busy = root.assistant_busy();
    let cancellable = root.assistant_cancellable();
    let retry = root.assistant.retry_prompt().is_some();
    let review_focus = root.review_focus.clone();
    let scroll = root.scroll.assistant_results.clone();
    // Results, diffs and pending cards stay on the opaque plane and scroll
    // under the frosted input bar; the padding lets the last card clear it.
    let results = div()
        .id("assistant-results")
        .debug_selector(|| "assistant-results".into())
        .size_full()
        .overflow_y_scroll()
        .track_scroll(&scroll)
        .pb(input_bar_height(&tokens) + tokens.spacing.xl + tokens.spacing.md)
        .flex()
        .flex_col()
        .gap_3()
        .children(result.map(|result| render_result(result, warnings, root.ui.locale, cx)))
        .children(
            (!receipts.is_empty())
                .then(|| render_receipts(receipts, root.ui.locale, cx, &root.assistant)),
        )
        .children(pending.map(|pending| render_pending(pending, &review_focus, root.ui.locale, cx)))
        .children((root.assistant.pending_overflow_count() > 0).then(|| {
            render_pending_overflow(
                root.assistant.pending_overflow_count(),
                root.ui.locale,
                &tokens,
            )
        }));
    let input_bar = div()
        .flex()
        .flex_col()
        .gap(tokens.spacing.xs)
        .child(
            div()
                .h(tokens.control_height.lg)
                .flex()
                .items_center()
                .gap(tokens.spacing.sm)
                .child(
                    div()
                        .debug_selector(|| "assistant-input".into())
                        .flex_1()
                        .min_w_0()
                        .child(root.assistant_inputs.prompt.clone()),
                )
                .child(
                    Button::new("assistant-submit", tr(root.ui.locale, "send"))
                        .debug_selector(|| "assistant-submit".into())
                        .primary()
                        .disabled(busy)
                        .on_click(cx.listener(RootView::assistant_action)),
                )
                .child(
                    Button::new("assistant-cancel", tr(root.ui.locale, "cancel"))
                        .debug_selector(|| "assistant-cancel".into())
                        .secondary()
                        .disabled(!cancellable)
                        .on_click(cx.listener(RootView::cancel_assistant)),
                )
                .children(retry.then(|| {
                    Button::new("assistant-retry", tr(root.ui.locale, "assistant_retry"))
                        .debug_selector(|| "assistant-retry".into())
                        .secondary()
                        .disabled(busy)
                        .on_click(cx.listener(RootView::retry_assistant_run))
                })),
        )
        .child(
            div()
                .h(STATUS_HEIGHT)
                .px(tokens.spacing.xs)
                .flex()
                .items_center()
                .gap(tokens.spacing.md)
                .text_xs()
                .text_color(tokens.text_muted)
                .child(
                    div()
                        .debug_selector(move || task_selector.into())
                        .child(task),
                )
                .children(job_progress.map(|progress| {
                    div()
                        .debug_selector(|| "assistant-job-progress".into())
                        .child(progress)
                })),
        );

    div()
        .debug_selector(|| "assistant-page".into())
        .flex_1()
        .min_w_0()
        .min_h_0()
        // No bottom padding: the results scroll to the plane's edge, and the
        // input bar floats above it.
        .pt_6()
        .px_6()
        .flex()
        .flex_col()
        .gap_4()
        .child(
            div()
                .text_2xl()
                .font_weight(gpui::FontWeight::BOLD)
                .child(tr(root.ui.locale, "assistant")),
        )
        .child(
            RadioGroup::new("assistant-scene", tr(root.ui.locale, "assistant_scene"))
                .debug_selector(|| "assistant-scene-group".into())
                .flex()
                .flex_wrap()
                .gap_2()
                .options(
                    [
                        AgentScene::Ask,
                        AgentScene::Organize,
                        AgentScene::Partition,
                        AgentScene::Extract,
                        AgentScene::Move,
                    ]
                    .map(|scene| {
                        (
                            scene == root.assistant.scene,
                            scene_button(scene, root.ui.locale, cx),
                        )
                    }),
                ),
        )
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
                    RadioGroup::new("assistant-scope", tr(root.ui.locale, "assistant_scope"))
                        .debug_selector(|| "assistant-scope-group".into())
                        .flex()
                        .gap_2()
                        .options(
                            [
                                AssistantScopeMode::CurrentDocument,
                                AssistantScopeMode::AllVisibleDocuments,
                            ]
                            .map(|scope| {
                                (
                                    scope == root.assistant.scope_mode,
                                    scope_button(scope, root.ui.locale, cx),
                                )
                            }),
                        ),
                )
                .child(info_line(
                    tr(root.ui.locale, "assistant_authorized_document"),
                    document_summary,
                    &tokens,
                ))
                .child(info_line(
                    tr(root.ui.locale, "assistant_projection"),
                    tr(root.ui.locale, projection_key(root.assistant.projection)).into(),
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
                .debug_selector(|| "assistant-body".into())
                .relative()
                .flex_1()
                .min_h_0()
                // The scrollbar stays outside the fade.
                .child(theme::scroll_faded(&scroll, tokens.content, results))
                .child(Scrollbar::vertical("assistant-results-scrollbar", &scroll))
                .child(
                    div()
                        .debug_selector(|| "assistant-input-bar".into())
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom(tokens.spacing.xl)
                        .child(theme::frosted(
                            tokens.radius.lg,
                            BlurLevel::Menu,
                            div().p(tokens.spacing.sm).child(theme::layered(input_bar)),
                        )),
                ),
        )
}

/// The status line under the prompt in the input bar.
const STATUS_HEIGHT: Pixels = px(16.);

/// The input bar's height: its padding and border around the prompt row and
/// the status line.
fn input_bar_height(tokens: &Tokens) -> Pixels {
    tokens.control_height.lg + tokens.spacing.xs + STATUS_HEIGHT + tokens.spacing.sm * 2. + px(2.)
}

fn scope_button(scope: AssistantScopeMode, locale: Locale, cx: &mut Context<RootView>) -> Button {
    let label = match scope {
        AssistantScopeMode::CurrentDocument => tr(locale, "assistant_current_document"),
        AssistantScopeMode::AllVisibleDocuments => tr(locale, "assistant_all_visible_documents"),
    };
    let name = match scope {
        AssistantScopeMode::CurrentDocument => "current",
        AssistantScopeMode::AllVisibleDocuments => "all",
    };
    Button::new(format!("assistant-scope-{label}"), label)
        .debug_selector(move || format!("assistant-scope-{name}"))
        .compact()
        .on_click(cx.listener(move |root, _, _, cx| root.select_assistant_scope(scope, cx)))
}

fn scene_button(scene: AgentScene, locale: Locale, cx: &mut Context<RootView>) -> Button {
    Button::new(
        format!("assistant-scene-{}", scene.skill_id()),
        tr(locale, scene_key(scene)),
    )
    .debug_selector(move || format!("assistant-scene-{}", scene.skill_id()))
    .on_click(cx.listener(move |root, _, _, cx| root.select_assistant_scene(scene, cx)))
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

fn projection_key(projection: DocumentProjection) -> &'static str {
    match projection {
        DocumentProjection::Metadata => "projection_metadata",
        DocumentProjection::Markdown => "projection_markdown",
        DocumentProjection::Transcript => "projection_transcript",
        DocumentProjection::SpeakerSegments => "projection_speaker_segments",
        DocumentProjection::Complete => "projection_complete",
    }
}

/// A revision for display: `r3`, or a dash when there is none.
fn revision_text(revision: Option<u64>) -> String {
    revision.map_or_else(|| "—".into(), |revision| format!("r{revision}"))
}

fn info_line(label: &'static str, value: String, tokens: &Tokens) -> gpui::Div {
    div()
        .flex()
        .gap_2()
        .text_sm()
        .child(div().text_color(tokens.text_muted).child(label))
        .child(value)
}

/// The task label's selector, for tests to find the state the page shows.
fn task_selector(task: &AssistantTaskState) -> &'static str {
    match task {
        AssistantTaskState::Idle => "assistant-task-idle",
        AssistantTaskState::Running { .. } => "assistant-task-running",
        AssistantTaskState::Cancelling { .. } => "assistant-task-cancelling",
        AssistantTaskState::AwaitingConfirmation => "assistant-task-awaiting",
        AssistantTaskState::Completed => "assistant-task-completed",
        AssistantTaskState::Partial(_) => "assistant-task-partial",
        AssistantTaskState::Failed(_) => "assistant-task-failed",
        AssistantTaskState::Cancelled => "assistant-task-cancelled",
    }
}

/// The run's job progress on the shared runtime.
fn job_progress_label(locale: Locale, progress: runtime::Progress) -> String {
    let assistant = tr(locale, "assistant");
    match progress.total {
        Some(total) => crate::i18n::message(
            locale,
            "job_progress_total",
            &format!("{assistant}|{}|{total}", progress.completed),
        ),
        None => crate::i18n::message(
            locale,
            "job_progress",
            &format!("{assistant}|{}", progress.completed),
        ),
    }
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

/// The position of the receipts among the results of a run that wrote:
/// after its result, before its pending review.
pub(crate) const RECEIPTS_ITEM: usize = 1;

/// The run's answer, evidence, model route, usage, diagnostics and every
/// warning it returned.
fn render_result(
    result: AssistantRunResult,
    warnings: Vec<AssistantUiError>,
    locale: Locale,
    cx: &mut Context<RootView>,
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
    let tokens = *theme::tokens(cx);

    div()
        .debug_selector(|| "assistant-result".into())
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
            warnings
                .into_iter()
                .enumerate()
                .map(|(index, warning)| render_warning(index, warning, locale, &tokens)),
        )
        .into_any_element()
}

/// Every write the run applied, each with its own undo entry; kept after
/// the run failed or was cancelled.
fn render_receipts(
    mutations: Vec<AssistantMutationSummary>,
    locale: Locale,
    cx: &mut Context<RootView>,
    controller: &AssistantController,
) -> gpui::AnyElement {
    let tokens = *theme::tokens(cx);
    div()
        .debug_selector(|| "assistant-receipts".into())
        .rounded(tokens.radius.md)
        .border_1()
        .border_color(tokens.border)
        .p_3()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_sm()
                .text_color(tokens.text_muted)
                .child(crate::i18n::message(
                    locale,
                    "assistant_receipts",
                    &mutations.len().to_string(),
                )),
        )
        .children(mutations.into_iter().map(|mutation| {
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

/// A warning of the run, marked `assistant-warning-<index>` in the order
/// the result lists them.
fn render_warning(
    index: usize,
    warning: AssistantUiError,
    locale: Locale,
    tokens: &Tokens,
) -> gpui::AnyElement {
    div()
        .debug_selector(move || format!("assistant-warning-{index}"))
        .px_3()
        .py_2()
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

/// A receipt of a write the run applied: the mutation, the documents it
/// changed and the segments it left uncovered on one line, with its own
/// undo entry beside it.
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
                "{}: {} → {}{}",
                change.document_id,
                revision_text(change.previous_revision),
                revision_text(change.revision),
                if change.deleted {
                    format!(" ({})", tr(locale, "assistant_deleted"))
                } else {
                    String::new()
                }
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
    let receipt = format!("assistant-receipt-{mutation_id}");
    let undo = format!("assistant-undo-{mutation_id}");
    let undone_marker = format!("assistant-undone-{mutation_id}");

    div()
        .debug_selector(move || receipt.clone())
        .px_3()
        .py_2()
        .rounded(tokens.radius.sm)
        .bg(tokens.raised)
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .child(info_line(
                    tr(locale, "assistant_mutation"),
                    mutation.mutation_id,
                    &tokens,
                ))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_x_4()
                        .text_xs()
                        .text_color(tokens.text_muted)
                        .child(format!("{} {affected}", tr(locale, "assistant_affected")))
                        .child(format!("{} {gaps}", tr(locale, "assistant_uncovered"))),
                ),
        )
        .child(
            div()
                .id(SharedString::from(undo.clone()))
                .debug_selector(move || undo.clone())
                .flex_shrink_0()
                .child(
                    Button::new(format!("assistant-undo-button-{mutation_id}"), undo_label)
                        .when(undone, |this| {
                            this.debug_selector(move || undone_marker.clone())
                        })
                        .secondary()
                        .compact()
                        .disabled(undone || undo_in_flight)
                        .on_click(cx.listener(move |root, _, _window, cx| {
                            root.undo_assistant_mutation_for(mutation_id.clone(), cx)
                        })),
                ),
        )
        .into_any_element()
}

/// The review the pending card shows, with its place in the run.
struct PendingCard {
    state: PendingReviewState,
    queued: usize,
    position: Option<(usize, usize)>,
}

/// The awaiting review: the run is paused before this write. The card
/// holds `focus`, which a decision or expiry moves to the next review; both
/// decisions are disabled while one is in flight.
fn render_pending(
    card: PendingCard,
    focus: &FocusHandle,
    locale: Locale,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    let tokens = *theme::tokens(cx);
    let PendingCard {
        state,
        queued,
        position,
    } = card;
    let decision = match &state {
        PendingReviewState::Accepting(_) => Some("assistant-review-accepting"),
        PendingReviewState::Rejecting(_) => Some("assistant-review-rejecting"),
        _ => None,
    };
    let (accept_label, reject_label) = match &state {
        PendingReviewState::Accepting(_) => (
            tr(locale, "assistant_accepting"),
            tr(locale, "assistant_reject"),
        ),
        PendingReviewState::Rejecting(_) => (
            tr(locale, "assistant_accept"),
            tr(locale, "assistant_rejecting"),
        ),
        _ => (
            tr(locale, "assistant_accept"),
            tr(locale, "assistant_reject"),
        ),
    };
    let (PendingReviewState::Awaiting(pending)
    | PendingReviewState::Accepting(pending)
    | PendingReviewState::Rejecting(pending)) = state
    else {
        let (label, selector) = match state {
            PendingReviewState::Rejected => (
                tr(locale, "assistant_rejected"),
                "assistant-pending-rejected",
            ),
            PendingReviewState::Expired => {
                (tr(locale, "assistant_expired"), "assistant-pending-expired")
            }
            _ => unreachable!(),
        };
        return div()
            .debug_selector(move || selector.into())
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
            let parent = |parent: &Option<String>| {
                parent
                    .clone()
                    .unwrap_or_else(|| tr(locale, "assistant_tree_root").into())
            };
            let order =
                |order: Option<u32>| order.map_or_else(|| "—".into(), |order| format!("#{order}"));
            format!(
                "{}: {}/{} → {}/{}",
                change.document_id,
                parent(&change.previous_parent_id),
                order(change.previous_sibling_order),
                parent(&change.new_parent_id),
                order(change.new_sibling_order)
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
                range
                    .document_id
                    .as_deref()
                    .unwrap_or(tr(locale, "assistant_new_document")),
                range.start_segment_id,
                range.end_segment_id,
                range.evidence_segment_ids.join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let gaps = pending.uncovered_segment_ids.join(", ");
    let busy = decision.is_some();
    let card_selector = format!("assistant-pending-{}", pending.confirmation_id);
    let expires = crate::i18n::message(
        locale,
        "assistant_pending_expires",
        &crate::components::utc_time(pending.expires_at_ms),
    );

    Region::new(
        SharedString::from(card_selector.clone()),
        tr(locale, "assistant_pending"),
        focus,
    )
    .debug_selector(move || card_selector.clone())
    .rounded(tokens.radius.md)
    .border_1()
    .border_color(tokens.warning)
    .p_4()
    .flex()
    .flex_col()
    .gap_2()
    .child(
        div()
            .flex()
            .items_center()
            .gap_3()
            .child(div().text_lg().child(tr(locale, "assistant_pending")))
            .children(position.map(|(position, total)| {
                div()
                    .debug_selector(move || {
                        format!("assistant-pending-position-{position}-of-{total}")
                    })
                    .text_sm()
                    .text_color(tokens.text_muted)
                    .child(crate::i18n::message(
                        locale,
                        "assistant_pending_position",
                        &format!("{position}|{total}"),
                    ))
            }))
            .child(
                div()
                    .debug_selector(|| "assistant-pending-expires".into())
                    .text_sm()
                    .text_color(tokens.text_muted)
                    .child(expires),
            ),
    )
    .child(
        div()
            .debug_selector(|| "assistant-pending-paused".into())
            .text_sm()
            .text_color(tokens.warning_text)
            .child(tr(locale, "assistant_pending_paused")),
    )
    .child(info_line(
        tr(locale, "assistant_pending_queue"),
        queued.to_string(),
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
            .when_some(decision, |this, decision| {
                this.debug_selector(move || decision.into())
            })
            .flex()
            .gap_2()
            .child(
                Button::new("assistant-confirm", accept_label)
                    .debug_selector(|| "assistant-confirm".into())
                    .primary()
                    .disabled(busy)
                    .on_click(cx.listener(RootView::accept_assistant_pending)),
            )
            .child(
                Button::new("assistant-reject", reject_label)
                    .debug_selector(|| "assistant-reject".into())
                    .secondary()
                    .disabled(busy)
                    .on_click(cx.listener(RootView::reject_assistant_pending)),
            ),
    )
    .into_any_element()
}

/// The reviews of the run beyond the queue limit: not listed, not written.
fn render_pending_overflow(count: usize, locale: Locale, tokens: &Tokens) -> gpui::AnyElement {
    div()
        .id("assistant-pending-overflow")
        .debug_selector(|| "assistant-pending-overflow".into())
        .px_3()
        .py_2()
        .rounded(tokens.radius.sm)
        .bg(tokens.warning_surface)
        .text_sm()
        .text_color(tokens.warning_text)
        .child(
            div()
                .debug_selector(move || format!("assistant-pending-overflow-{count}"))
                .child(crate::i18n::message(
                    locale,
                    "assistant_pending_overflow",
                    &count.to_string(),
                )),
        )
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
