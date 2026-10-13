use crate::{
    assistant_ui::{
        AssistantController, AssistantDocumentChange, AssistantMutationSummary, AssistantRunResult,
        AssistantScopeMode, AssistantTaskState, AssistantUiError, PendingReviewState,
    },
    controller::RootView,
    controls::{Button, NavEntry, NavList, RadioGroup, Region, Scrollbar},
    i18n::{text as tr, Locale},
    theme::{self, BlurLevel, Tokens},
};
use contracts::AgentScene;
use gpui::{
    div, prelude::*, px, Context, FocusHandle, IntoElement, MouseButton, Pixels, SharedString,
};

pub(crate) fn render(root: &mut RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let tokens = *theme::tokens(cx);
    let selected = root
        .ui
        .selected_project
        .as_ref()
        .and_then(|id| root.projects.iter().find(|project| project.id() == id));
    let context = context_parts(
        root.ui.locale,
        root.assistant.scope_mode,
        selected.map(|document| document.title()),
        root.projects.len(),
        root.saved_assistant()
            .active_connection()
            .map(|profile| (profile.name.as_str(), profile.is_local())),
    );
    let write_mode = root.saved_assistant().write_mode;
    let status = shows_status(&root.assistant.task);
    let task = task_label(root.ui.locale, &root.assistant.task);
    let task_selector = task_selector(&root.assistant.task);
    // The run's steps, while it runs and the runtime knows how many.
    let job_progress = root
        .assistant
        .job_progress
        .filter(|_| matches!(root.assistant.task, AssistantTaskState::Running { .. }))
        .and_then(job_progress_text);
    let titles: DocumentTitles = root
        .projects
        .iter()
        .map(|document| {
            (
                document.id().as_str().to_owned(),
                document.title().to_owned(),
            )
        })
        .collect();
    let result = root.assistant.result.clone();
    let render_model = result.as_ref().map(result_render_model);
    let (warnings, receipts) = render_model
        .map(|model| (model.warnings, model.mutations))
        .unwrap_or_default();
    let pending = root.assistant.pending.clone().map(|state| PendingCard {
        state,
        position: root.assistant.pending_position(),
    });
    let busy = root.assistant_busy();
    let cancellable = root.assistant_cancellable();
    let retry = root.assistant.retry_prompt().is_some();
    let review_focus = root.review_focus.clone();
    let scroll = root.scroll.assistant_results.clone();
    // Results, diffs and pending cards scroll on the opaque plane above
    // the input bar. Its space belongs to the viewport's surrounding layout,
    // so keyboard focus and pointer scrolling reveal the same visible area.
    let results = div()
        .id("assistant-results")
        .debug_selector(|| "assistant-results".into())
        .size_full()
        .overflow_y_scroll()
        .track_scroll(&scroll)
        .flex()
        .flex_col()
        .gap_3()
        .children(
            root.assistant
                .sent_request()
                .map(|request| render_request(request.to_owned(), &tokens)),
        )
        .children(match &root.assistant.task {
            AssistantTaskState::Failed(error) => {
                Some(render_error(error, retry, busy, root.ui.locale, cx))
            }
            _ => None,
        })
        .children(result.map(|result| render_result(result, warnings, root.ui.locale, cx)))
        .children(
            (!receipts.is_empty())
                .then(|| render_receipts(receipts, &titles, root.ui.locale, cx, &root.assistant)),
        )
        .children(
            pending
                .map(|pending| render_pending(pending, &titles, &review_focus, root.ui.locale, cx)),
        )
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
                ),
        )
        .when(status, |bar| {
            bar.child(
                div()
                    .debug_selector(|| "assistant-status".into())
                    .h(STATUS_HEIGHT)
                    .line_height(STATUS_HEIGHT)
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
            )
        });

    div()
        .debug_selector(|| "assistant-page".into())
        .flex_1()
        .min_w_0()
        .min_h_0()
        // No bottom padding: the body below holds the space under the
        // floating input bar.
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
        .child(scenes(root, &tokens, cx))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(tokens.spacing.xs)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(tokens.spacing.md)
                        .child(
                            RadioGroup::new(
                                "assistant-scope",
                                tr(root.ui.locale, "assistant_scope"),
                            )
                            .debug_selector(|| "assistant-scope-group".into())
                            .flex_shrink_0()
                            .flex()
                            .gap_1()
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
                        .child(context_line(context, write_mode, root.ui.locale, &tokens)),
                )
                .child(
                    div()
                        .debug_selector(|| "assistant-privacy".into())
                        .text_xs()
                        .text_color(tokens.text_subtle)
                        .child(tr(
                            root.ui.locale,
                            match root.assistant.scope_mode {
                                AssistantScopeMode::CurrentDocument => "assistant_scope_notice",
                                AssistantScopeMode::AllVisibleDocuments => {
                                    "assistant_all_visible_scope_notice"
                                }
                            },
                        )),
                ),
        )
        .child(
            div()
                .debug_selector(|| "assistant-body".into())
                .relative()
                .flex_1()
                .min_h_0()
                // Reserve the bar and the gap above it outside the scroll
                // viewport; growing status text still keeps its bottom fixed.
                .pb(tokens.spacing.xl + input_bar_height(&tokens, status) + tokens.spacing.md)
                .child(
                    div()
                        .relative()
                        .size_full()
                        // The scrollbar stays outside the fade.
                        .child(theme::scroll_faded(&scroll, tokens.content, results))
                        .child(Scrollbar::vertical("assistant-results-scrollbar", &scroll)),
                )
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

/// The input bar's height: its padding and border around the prompt row and,
/// with `status`, the status line under it.
fn input_bar_height(tokens: &Tokens, status: bool) -> Pixels {
    let status = if status {
        tokens.spacing.xs + STATUS_HEIGHT
    } else {
        px(0.)
    };
    tokens.control_height.lg + status + tokens.spacing.sm * 2. + px(2.)
}

/// Whether the input bar has a status to show: not while idle, where the
/// page has nothing to report.
fn shows_status(task: &AssistantTaskState) -> bool {
    !matches!(task, AssistantTaskState::Idle)
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

/// The scenes in tab order.
pub(crate) const SCENES: [AgentScene; 5] = [
    AgentScene::Ask,
    AgentScene::Organize,
    AgentScene::Partition,
    AgentScene::Extract,
    AgentScene::Move,
];

/// The scene tabs, quieter than the send button, with what the selected
/// scene does underneath.
fn scenes(root: &RootView, tokens: &Tokens, cx: &mut Context<RootView>) -> gpui::Div {
    let locale = root.ui.locale;
    let current = root.assistant.scene;
    let focus = &root.assistant_scene_focus;
    div()
        .flex()
        .flex_col()
        .gap(tokens.spacing.xs)
        .child(
            NavList::new("assistant-scene", tr(locale, "assistant_scene"))
                .debug_selector(|| "assistant-scene-group".into())
                .flex()
                .flex_wrap()
                .items_center()
                .gap(tokens.spacing.xs)
                .children(SCENES.iter().zip(focus).map(|(&scene, focus)| {
                    scene_tab(scene, scene == current, focus, locale, tokens, cx)
                })),
        )
        .child(
            div()
                .debug_selector(move || {
                    format!("assistant-scene-description-{}", current.skill_id())
                })
                .px(tokens.spacing.md)
                .text_sm()
                .text_color(tokens.text_muted)
                .child(tr(locale, scene_description_key(current))),
        )
}

/// A scene's tab. A press selects the scene and leaves focus where it was,
/// so typing in the prompt goes on.
fn scene_tab(
    scene: AgentScene,
    selected: bool,
    focus: &FocusHandle,
    locale: Locale,
    tokens: &Tokens,
    cx: &mut Context<RootView>,
) -> NavEntry {
    let id = scene.skill_id();
    let label = tr(locale, scene_key(scene));
    NavEntry::new(format!("assistant-scene-{id}"), label, focus)
        .debug_selector(move || format!("assistant-scene-{id}"))
        .selected(selected)
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .on_click(cx.listener(move |root, _, _, cx| root.select_assistant_scene(scene, cx)))
        .child(
            div()
                .when(selected, |label| {
                    label.debug_selector(move || format!("assistant-scene-selected-{id}"))
                })
                .text_color(if selected {
                    tokens.text
                } else {
                    tokens.text_muted
                })
                .child(label),
        )
}

/// One sentence on what a scene does with the documents in scope.
fn scene_description_key(scene: AgentScene) -> &'static str {
    match scene {
        AgentScene::Ask => "assistant_scene_ask_description",
        AgentScene::Organize => "assistant_scene_organize_description",
        AgentScene::Partition => "assistant_scene_partition_description",
        AgentScene::Extract => "assistant_scene_extract_description",
        AgentScene::Move => "assistant_scene_move_description",
    }
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

/// What a run would read and where it would send it: the documents in
/// scope, the connection and whether it stays on this machine. Ids and
/// revisions stay out of it.
fn context_parts(
    locale: Locale,
    scope: AssistantScopeMode,
    selected_title: Option<&str>,
    visible_documents: usize,
    connection: Option<(&str, bool)>,
) -> Vec<String> {
    let documents = match scope {
        AssistantScopeMode::CurrentDocument => selected_title
            .map(str::to_owned)
            .unwrap_or_else(|| tr(locale, "assistant_no_document").into()),
        AssistantScopeMode::AllVisibleDocuments => crate::i18n::message(
            locale,
            "assistant_document_count",
            &visible_documents.to_string(),
        ),
    };
    let mut parts = vec![documents];
    match connection {
        Some((name, local)) => parts.extend([
            name.to_owned(),
            tr(
                locale,
                if local {
                    "assistant_local_connection"
                } else {
                    "assistant_remote_connection"
                },
            )
            .to_owned(),
        ]),
        None => parts.push(tr(locale, "assistant_no_connection").into()),
    }
    parts
}

/// The context beside the scope: its parts and the write mode in muted
/// text, separated by dots. The first part, a document title, fades out
/// when it is too long; so does the connection name.
fn context_line(
    parts: Vec<String>,
    write_mode: contracts::AgentWriteMode,
    locale: Locale,
    tokens: &Tokens,
) -> gpui::Div {
    let (mode_key, mode_selector) = match write_mode {
        contracts::AgentWriteMode::Automatic => {
            ("assistant_automatic", "assistant-write-mode-automatic")
        }
        contracts::AgentWriteMode::ConfirmEach => (
            "assistant_confirm_each",
            "assistant-write-mode-confirm-each",
        ),
    };
    let separator = || {
        div()
            .flex_shrink_0()
            .text_color(tokens.text_subtle)
            .child("·")
    };
    let mut parts = parts.into_iter();
    let mut line = div()
        .debug_selector(|| "assistant-context".into())
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .gap(tokens.spacing.xs)
        .text_sm()
        .text_color(tokens.text_muted)
        .whitespace_nowrap();
    if let Some(first) = parts.next() {
        line = line.child(div().min_w_0().child(theme::tail_faded_label(
            "assistant-context-documents",
            first,
            tokens.content,
        )));
    }
    if let Some(connection) = parts.next() {
        line = line
            .child(separator())
            .child(div().min_w_0().child(theme::tail_faded_label(
                "assistant-context-connection",
                connection,
                tokens.content,
            )));
    }
    for part in parts {
        line = line
            .child(separator())
            .child(div().flex_shrink_0().child(part));
    }
    line.child(separator()).child(
        div()
            .debug_selector(move || mode_selector.into())
            .flex_shrink_0()
            .child(tr(locale, mode_key)),
    )
}

/// A revision for display: `r3`, or a dash when there is none.
fn revision_text(revision: Option<u64>) -> String {
    revision.map_or_else(|| "—".into(), |revision| format!("r{revision}"))
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

/// The run's steps on the shared runtime, once it knows how many there are.
fn job_progress_text(progress: runtime::Progress) -> Option<String> {
    progress
        .total
        .map(|total| format!("{} / {total}", progress.completed))
}

fn task_label(locale: Locale, task: &AssistantTaskState) -> String {
    match task {
        AssistantTaskState::Idle => String::new(),
        AssistantTaskState::Running { .. } => tr(locale, "assistant_running").into(),
        AssistantTaskState::Cancelling { .. } => tr(locale, "assistant_cancelling").into(),
        AssistantTaskState::AwaitingConfirmation => {
            tr(locale, "assistant_awaiting_confirmation").into()
        }
        AssistantTaskState::Completed => tr(locale, "assistant_completed").into(),
        AssistantTaskState::Partial(error) => format!(
            "{} · {}",
            tr(locale, "assistant_partial"),
            describe_error(locale, error).title
        ),
        AssistantTaskState::Failed(error) => format!(
            "{} · {}",
            tr(locale, "failed"),
            describe_error(locale, error).title
        ),
        AssistantTaskState::Cancelled => tr(locale, "assistant_cancelled").into(),
    }
}

/// The position of the receipts among the results of a run that wrote and
/// has no error to show: after its request and result, before its pending
/// review.
pub(crate) const RECEIPTS_ITEM: usize = 2;

/// The request of the run, quoted at the top of its results.
fn render_request(request: String, tokens: &Tokens) -> gpui::AnyElement {
    div()
        .debug_selector(|| "assistant-request".into())
        // The results overflow their scroll area; the quote keeps its line.
        .flex_shrink_0()
        .border_l_2()
        .border_color(tokens.border)
        .pl(tokens.spacing.md)
        .py(tokens.spacing.xs)
        .text_sm()
        .text_color(tokens.text_muted)
        .child(request)
        .into_any_element()
}

/// The run's answer and every warning it returned, with what the run
/// reported about itself (evidence, model, tokens, diagnostics) in small
/// print underneath.
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
    let details = result_details(locale, &result);
    let tokens = *theme::tokens(cx);

    div()
        .debug_selector(|| "assistant-result".into())
        .rounded(tokens.radius.md)
        .border_1()
        .border_color(tokens.border)
        .p_4()
        .flex()
        .flex_col()
        .gap_3()
        .child(div().text_base().child(answer))
        .children(
            warnings
                .into_iter()
                .enumerate()
                .map(|(index, warning)| render_warning(index, warning, locale, &tokens)),
        )
        .when(!details.is_empty(), |card| {
            card.child(
                div()
                    .debug_selector(|| "assistant-result-details".into())
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .text_xs()
                    .text_color(tokens.text_subtle)
                    .children(details.into_iter().map(|(label, value)| {
                        div()
                            .flex()
                            .gap_2()
                            .child(div().flex_shrink_0().child(label))
                            .child(div().min_w_0().text_color(tokens.text_muted).child(value))
                    })),
            )
        })
        .into_any_element()
}

/// What the run reported about itself, each with its label; what it
/// did not report is left out rather than shown as a dash.
fn result_details(locale: Locale, result: &AssistantRunResult) -> Vec<(&'static str, String)> {
    let mut details = Vec::new();
    if !result.evidence_segment_ids.is_empty() {
        details.push((
            tr(locale, "assistant_evidence"),
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
        details.push((tr(locale, "assistant_model_route"), model));
    }
    if let Some(usage) = result.usage {
        let mut text = format!(
            "{} / {} / {}",
            usage.input_tokens, usage.output_tokens, usage.total_tokens
        );
        if let Some(cost) = usage.cost_microusd {
            text.push_str(&format!(" · USD {:.6}", cost as f64 / 1_000_000.0));
        }
        details.push((tr(locale, "assistant_usage"), text));
    }
    if let Some(diagnostics) = &result.diagnostics {
        details.push((
            tr(locale, "assistant_diagnostics"),
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

/// Every write the run applied, each with its own undo entry; kept after
/// the run failed or was cancelled.
fn render_receipts(
    mutations: Vec<AssistantMutationSummary>,
    titles: &DocumentTitles,
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
                titles,
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
        .bg(tokens.raised)
        .child(problem(
            "icons/triangle-alert.svg",
            tokens.warning,
            &warning,
            locale,
            tokens,
        ))
        .into_any_element()
}

/// The error the task ended with: what happened and what to do, a way to
/// Assistant settings when that is where it is fixed, and the retry of a
/// run that can be sent again.
fn render_error(
    error: &AssistantUiError,
    retry: bool,
    busy: bool,
    locale: Locale,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    let tokens = *theme::tokens(cx);
    let opens_settings = describe_error(locale, error).opens_settings;
    div()
        .debug_selector(|| "assistant-error".into())
        .rounded(tokens.radius.md)
        .border_1()
        .border_color(tokens.border)
        .bg(tokens.surface)
        .p_4()
        .flex()
        .flex_col()
        .gap_3()
        .child(problem(
            "icons/circle-x.svg",
            tokens.danger,
            error,
            locale,
            &tokens,
        ))
        .when(opens_settings || retry, |card| {
            card.child(
                div()
                    // Under the text, past the icon.
                    .pl(PROBLEM_ICON + tokens.spacing.sm)
                    .flex()
                    .gap_2()
                    .when(opens_settings, |actions| {
                        actions.child(
                            Button::new(
                                "assistant-open-settings",
                                tr(locale, "assistant_open_settings"),
                            )
                            .debug_selector(|| "assistant-open-settings".into())
                            .secondary()
                            .compact()
                            .on_click(
                                cx.listener(|root, _, _, cx| root.open_assistant_settings(cx)),
                            ),
                        )
                    })
                    .when(retry, |actions| {
                        actions.child(
                            Button::new("assistant-retry", tr(locale, "assistant_retry"))
                                .debug_selector(|| "assistant-retry".into())
                                .secondary()
                                .compact()
                                .disabled(busy)
                                .on_click(cx.listener(RootView::retry_assistant_run)),
                        )
                    }),
            )
        })
        .into_any_element()
}

/// The size of a problem's icon.
const PROBLEM_ICON: Pixels = px(16.);

/// An error or warning in words: an icon in its semantic color, what
/// happened, what to do next, and the code in small print for
/// troubleshooting.
fn problem(
    icon: &'static str,
    color: gpui::Hsla,
    error: &AssistantUiError,
    locale: Locale,
    tokens: &Tokens,
) -> gpui::Div {
    let copy = describe_error(locale, error);
    div()
        .flex()
        .gap(tokens.spacing.sm)
        .child(
            div()
                .flex_shrink_0()
                .h(px(20.))
                .flex()
                .items_center()
                .child(gpui::svg().path(icon).size(PROBLEM_ICON).text_color(color)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(tokens.text)
                        .child(copy.title),
                )
                .children(
                    copy.hint
                        .map(|hint| div().text_sm().text_color(tokens.text_muted).child(hint)),
                )
                .child(
                    div()
                        .debug_selector(|| "assistant-error-details".into())
                        .text_xs()
                        .text_color(tokens.text_subtle)
                        .child(error_details(error)),
                ),
        )
}

/// What an error means for the person at the page, by its code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrorKind {
    Authentication,
    CredentialFile,
    ConnectionSetup,
    Disabled,
    Timeout,
    RateLimited,
    ProviderResponse,
    ModelUnsupported,
    ContextLimitUnknown,
    ContextTooLarge,
    StepLimit,
    InvalidChange,
    OutOfScope,
    DocumentChanged,
    TreeRule,
    WriteFailed,
    Expired,
    Unavailable,
    Interrupted,
    Cancelled,
}

impl ErrorKind {
    /// Whether Assistant settings are where the error is fixed.
    fn opens_settings(self) -> bool {
        matches!(
            self,
            Self::Authentication
                | Self::CredentialFile
                | Self::ConnectionSetup
                | Self::Disabled
                | Self::Timeout
                | Self::ProviderResponse
                | Self::ModelUnsupported
                | Self::ContextLimitUnknown
                | Self::ContextTooLarge
                | Self::StepLimit
        )
    }
}

/// Every code an Assistant run, decision or undo reports: the agent
/// contract's codes and the desktop service's own.
const KNOWN_ERRORS: [(&str, ErrorKind); 37] = [
    ("PROVIDER_AUTHENTICATION_FAILED", ErrorKind::Authentication),
    (
        "ASSISTANT_API_KEY_FILE_READ_FAILED",
        ErrorKind::CredentialFile,
    ),
    ("ASSISTANT_API_KEY_FILE_EMPTY", ErrorKind::CredentialFile),
    ("CONNECTION_NOT_CONFIGURED", ErrorKind::ConnectionSetup),
    ("ASSISTANT_SETTINGS_INVALID", ErrorKind::ConnectionSetup),
    ("ASSISTANT_DISABLED", ErrorKind::Disabled),
    ("PROVIDER_TIMEOUT", ErrorKind::Timeout),
    ("PROVIDER_RATE_LIMITED", ErrorKind::RateLimited),
    ("PROVIDER_RESPONSE_INVALID", ErrorKind::ProviderResponse),
    (
        "PROVIDER_CAPABILITY_UNSUPPORTED",
        ErrorKind::ModelUnsupported,
    ),
    ("CONTEXT_LIMIT_UNKNOWN", ErrorKind::ContextLimitUnknown),
    ("CONTEXT_LIMIT_EXCEEDED", ErrorKind::ContextTooLarge),
    ("TOOL_LOOP_LIMIT_REACHED", ErrorKind::StepLimit),
    ("TOOL_UNKNOWN", ErrorKind::InvalidChange),
    ("TOOL_NOT_ALLOWED", ErrorKind::InvalidChange),
    ("TOOL_ARGUMENTS_INVALID", ErrorKind::InvalidChange),
    ("SKILL_NOT_FOUND", ErrorKind::InvalidChange),
    ("DOCUMENT_FIELD_READ_ONLY", ErrorKind::InvalidChange),
    ("SEGMENT_NOT_FOUND", ErrorKind::InvalidChange),
    ("RANGE_INVALID_ORDER", ErrorKind::InvalidChange),
    ("RANGE_OUTSIDE_PARENT", ErrorKind::InvalidChange),
    ("RANGE_OVERLAP", ErrorKind::InvalidChange),
    ("RANGE_COVERAGE_GAP", ErrorKind::InvalidChange),
    ("WRITE_POLICY_REJECTED", ErrorKind::InvalidChange),
    ("DOCUMENT_ACCESS_DENIED", ErrorKind::OutOfScope),
    ("DOCUMENT_REVISION_CONFLICT", ErrorKind::DocumentChanged),
    ("DOCUMENT_NOT_FOUND", ErrorKind::DocumentChanged),
    ("TREE_PARENT_NOT_FOUND", ErrorKind::TreeRule),
    ("TREE_CYCLE", ErrorKind::TreeRule),
    ("TREE_MAX_DEPTH_EXCEEDED", ErrorKind::TreeRule),
    ("TREE_CROSS_AUDIO_ASSET", ErrorKind::TreeRule),
    ("WRITE_TRANSACTION_FAILED", ErrorKind::WriteFailed),
    ("IDEMPOTENCY_CONFLICT", ErrorKind::WriteFailed),
    ("WRITE_CONFIRMATION_EXPIRED", ErrorKind::Expired),
    ("ASSISTANT_RUNTIME_NOT_CONNECTED", ErrorKind::Unavailable),
    ("TASK_FAILED", ErrorKind::Interrupted),
    ("TASK_CANCELLED", ErrorKind::Cancelled),
];

fn error_title_key(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Authentication => "assistant_error_authentication",
        ErrorKind::CredentialFile => "assistant_error_credential_file",
        ErrorKind::ConnectionSetup => "assistant_error_connection",
        ErrorKind::Disabled => "assistant_error_disabled",
        ErrorKind::Timeout => "assistant_error_timeout",
        ErrorKind::RateLimited => "assistant_error_rate_limited",
        ErrorKind::ProviderResponse => "assistant_error_provider_response",
        ErrorKind::ModelUnsupported => "assistant_error_model_unsupported",
        ErrorKind::ContextLimitUnknown => "assistant_error_context_unknown",
        ErrorKind::ContextTooLarge => "assistant_error_context_exceeded",
        ErrorKind::StepLimit => "assistant_error_step_limit",
        ErrorKind::InvalidChange => "assistant_error_invalid_change",
        ErrorKind::OutOfScope => "assistant_error_out_of_scope",
        ErrorKind::DocumentChanged => "assistant_error_document_changed",
        ErrorKind::TreeRule => "assistant_error_tree_rule",
        ErrorKind::WriteFailed => "assistant_error_write_failed",
        ErrorKind::Expired => "assistant_error_expired",
        ErrorKind::Unavailable => "assistant_error_unavailable",
        ErrorKind::Interrupted => "assistant_error_interrupted",
        ErrorKind::Cancelled => "assistant_error_cancelled",
    }
}

fn error_hint_key(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Authentication => "assistant_error_authentication_hint",
        ErrorKind::CredentialFile => "assistant_error_credential_file_hint",
        ErrorKind::ConnectionSetup => "assistant_error_connection_hint",
        ErrorKind::Disabled => "assistant_error_disabled_hint",
        ErrorKind::Timeout => "assistant_error_timeout_hint",
        ErrorKind::RateLimited => "assistant_error_rate_limited_hint",
        ErrorKind::ProviderResponse => "assistant_error_provider_response_hint",
        ErrorKind::ModelUnsupported => "assistant_error_model_unsupported_hint",
        ErrorKind::ContextLimitUnknown => "assistant_error_context_unknown_hint",
        ErrorKind::ContextTooLarge => "assistant_error_context_exceeded_hint",
        ErrorKind::StepLimit => "assistant_error_step_limit_hint",
        ErrorKind::InvalidChange => "assistant_error_invalid_change_hint",
        ErrorKind::OutOfScope => "assistant_error_out_of_scope_hint",
        ErrorKind::DocumentChanged => "assistant_error_document_changed_hint",
        ErrorKind::TreeRule => "assistant_error_tree_rule_hint",
        ErrorKind::WriteFailed => "assistant_error_write_failed_hint",
        ErrorKind::Expired => "assistant_error_expired_hint",
        ErrorKind::Unavailable => "assistant_error_unavailable_hint",
        ErrorKind::Interrupted => "assistant_error_interrupted_hint",
        ErrorKind::Cancelled => "assistant_error_cancelled_hint",
    }
}

/// How the page words an error.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ErrorCopy {
    /// What happened.
    title: String,
    /// What to do next.
    hint: Option<&'static str>,
    /// Whether Assistant settings are where to do it.
    opens_settings: bool,
}

/// A known code reads as what happened and what to do next; an unknown one
/// falls back to the error's own message.
fn describe_error(locale: Locale, error: &AssistantUiError) -> ErrorCopy {
    match KNOWN_ERRORS.iter().find(|(code, _)| *code == error.code) {
        Some(&(_, kind)) => ErrorCopy {
            title: tr(locale, error_title_key(kind)).into(),
            hint: Some(tr(locale, error_hint_key(kind))),
            opens_settings: kind.opens_settings(),
        },
        None => ErrorCopy {
            title: match error.message.trim() {
                "" => tr(locale, "assistant_error_unknown").into(),
                message => message.into(),
            },
            hint: None,
            opens_settings: false,
        },
    }
}

/// The code, and the raw message of a known code, for troubleshooting.
fn error_details(error: &AssistantUiError) -> String {
    let known = KNOWN_ERRORS.iter().any(|(code, _)| *code == error.code);
    match error.message.trim() {
        message if known && !message.is_empty() => format!("{} · {message}", error.code),
        _ => error.code.clone(),
    }
}

/// A receipt of a write the run applied: what it changed, named by
/// document title, with the mutation and revisions as details for
/// troubleshooting, the segments it left out when there are any, and its
/// own undo entry beside it.
fn render_mutation(
    mutation: AssistantMutationSummary,
    titles: &DocumentTitles,
    locale: Locale,
    cx: &mut Context<RootView>,
    undone: bool,
    undo_in_flight: bool,
) -> gpui::AnyElement {
    let summary = receipt_summary(locale, &mutation, &|id| titles.get(id).cloned());
    let details = receipt_details(&mutation);
    let gaps = uncovered_text(locale, &mutation.uncovered_segment_ids);
    let mutation_id = mutation.mutation_id;
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
    let gaps_marker = format!("assistant-receipt-gaps-{mutation_id}");

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
                .gap(px(2.))
                .child(
                    div()
                        .text_sm()
                        .text_color(if undone {
                            tokens.text_muted
                        } else {
                            tokens.text
                        })
                        .child(summary),
                )
                .children(gaps.map(|gaps| {
                    div()
                        .debug_selector(move || gaps_marker.clone())
                        .text_xs()
                        .text_color(tokens.text_muted)
                        .child(gaps)
                }))
                .child(
                    div()
                        .text_xs()
                        .text_color(tokens.text_subtle)
                        .child(details),
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
                        .ghost()
                        .compact()
                        .disabled(undone || undo_in_flight)
                        .on_click(cx.listener(move |root, _, _window, cx| {
                            root.undo_assistant_mutation_for(mutation_id.clone(), cx)
                        })),
                ),
        )
        .into_any_element()
}

/// Document titles by id, to name documents instead of showing their ids.
type DocumentTitles = std::collections::HashMap<String, String>;

/// A document's title in the locale's quotes; its id while the list does
/// not have it yet.
fn quoted_title(locale: Locale, id: &str, title_of: &dyn Fn(&str) -> Option<String>) -> String {
    crate::i18n::message(
        locale,
        "assistant_quoted",
        &title_of(id).unwrap_or_else(|| id.to_owned()),
    )
}

/// What a write did, in words: the documents it created, updated and
/// deleted, each named by title.
fn receipt_summary(
    locale: Locale,
    mutation: &AssistantMutationSummary,
    title_of: &dyn Fn(&str) -> Option<String>,
) -> String {
    let (list_separator, phrase_separator) = match locale {
        Locale::ZhCn => ("、", "；"),
        Locale::EnUs => (", ", "; "),
    };
    let documents = |kept: fn(&AssistantDocumentChange) -> bool| {
        mutation
            .affected_documents
            .iter()
            .filter(|change| kept(change))
            .map(|change| quoted_title(locale, &change.document_id, title_of))
            .collect::<Vec<_>>()
            .join(list_separator)
    };
    let phrases = [
        (
            "assistant_receipt_created",
            documents(|change| !change.deleted && change.previous_revision.is_none()),
        ),
        (
            "assistant_receipt_updated",
            documents(|change| !change.deleted && change.previous_revision.is_some()),
        ),
        (
            "assistant_receipt_deleted",
            documents(|change| change.deleted),
        ),
    ]
    .into_iter()
    .filter(|(_, documents)| !documents.is_empty())
    .enumerate()
    .map(|(index, (key, documents))| {
        let phrase = crate::i18n::message(locale, key, &documents);
        match (locale, index) {
            // Only the first phrase of an English sentence is capitalized.
            (Locale::EnUs, 1..) => {
                let mut chars = phrase.chars();
                chars
                    .next()
                    .map(|first| first.to_lowercase().chain(chars).collect())
                    .unwrap_or_default()
            }
            _ => phrase,
        }
    })
    .collect::<Vec<String>>();
    if phrases.is_empty() {
        tr(locale, "assistant_receipt_written").into()
    } else {
        phrases.join(phrase_separator)
    }
}

/// The mutation and the revisions it moved each document between.
fn receipt_details(mutation: &AssistantMutationSummary) -> String {
    let revisions = mutation
        .affected_documents
        .iter()
        .map(|change| {
            format!(
                "{} → {}",
                revision_text(change.previous_revision),
                revision_text(change.revision)
            )
        })
        .collect::<Vec<_>>();
    if revisions.is_empty() {
        mutation.mutation_id.clone()
    } else {
        format!("{} · {}", mutation.mutation_id, revisions.join(", "))
    }
}

/// How many transcript segments a write left out of its documents; nothing
/// when it left none out.
fn uncovered_text(locale: Locale, segments: &[String]) -> Option<String> {
    (!segments.is_empty()).then(|| {
        crate::i18n::message(
            locale,
            "assistant_uncovered_count",
            &segments.len().to_string(),
        )
    })
}

/// The review the pending card shows, with its place in the run.
struct PendingCard {
    state: PendingReviewState,
    position: Option<(usize, usize)>,
}

/// The awaiting review: the run is paused before this write. The card
/// leads with what the write changes, old value struck through above the
/// new one, keeps its place in the queue and its expiry in small print, and
/// closes with the decisions at its right edge. It holds `focus`, which a
/// decision or expiry moves to the next review; both decisions are disabled
/// while one is in flight.
fn render_pending(
    card: PendingCard,
    titles: &DocumentTitles,
    focus: &FocusHandle,
    locale: Locale,
    cx: &mut Context<RootView>,
) -> gpui::AnyElement {
    let tokens = *theme::tokens(cx);
    let PendingCard { state, position } = card;
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
            .text_sm()
            .text_color(tokens.text_muted)
            .child(label)
            .into_any_element();
    };
    let title_of = |id: &str| titles.get(id).cloned();
    let busy = decision.is_some();
    let card_selector = format!("assistant-pending-{}", pending.confirmation_id);
    let expires = crate::i18n::message(
        locale,
        "assistant_pending_expires",
        &crate::components::utc_time(pending.expires_at_ms),
    );
    let affects = (pending.field_diffs.is_empty()
        && pending.tree_changes.is_empty()
        && pending.ranges.is_empty())
    .then(|| {
        crate::i18n::message(
            locale,
            "assistant_pending_affects",
            &pending
                .affected_document_ids
                .iter()
                .map(|id| quoted_title(locale, id, &title_of))
                .collect::<Vec<_>>()
                .join(match locale {
                    Locale::ZhCn => "、",
                    Locale::EnUs => ", ",
                }),
        )
    });
    let small = |text: String| div().text_xs().text_color(tokens.text_muted).child(text);

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
    .gap_3()
    .child(
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap_3()
                    .child(
                        div()
                            .debug_selector(|| "assistant-pending-title".into())
                            .text_base()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(tr(locale, "assistant_pending")),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .gap_1()
                            .text_xs()
                            .text_color(tokens.text_subtle)
                            .children(position.map(|(position, total)| {
                                div()
                                    .debug_selector(move || {
                                        format!("assistant-pending-position-{position}-of-{total}")
                                    })
                                    .child(crate::i18n::message(
                                        locale,
                                        "assistant_pending_position",
                                        &format!("{position}|{total}"),
                                    ))
                            }))
                            .when(position.is_some(), |meta| meta.child("·"))
                            .child(
                                div()
                                    .debug_selector(|| "assistant-pending-expires".into())
                                    .child(expires),
                            ),
                    ),
            )
            .child(
                div()
                    .debug_selector(|| "assistant-pending-paused".into())
                    .text_sm()
                    .text_color(tokens.text_muted)
                    .child(tr(locale, "assistant_pending_paused")),
            ),
    )
    .children(pending.field_diffs.iter().enumerate().map(|(index, diff)| {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(small(field_diff_label(locale, diff, &title_of)))
            .children(diff.before.clone().map(|before| {
                diff_value(before, false, &tokens)
                    .debug_selector(move || format!("assistant-diff-{index}-before"))
            }))
            .children(diff.after.clone().map(|after| {
                diff_value(after, true, &tokens)
                    .debug_selector(move || format!("assistant-diff-{index}-after"))
            }))
    }))
    .children(pending.tree_changes.iter().map(|change| {
        div()
            .text_sm()
            .child(tree_change_text(locale, change, &title_of))
    }))
    .children(
        pending
            .ranges
            .iter()
            .map(|range| small(range_text(locale, range, &title_of))),
    )
    .children(uncovered_text(locale, &pending.uncovered_segment_ids).map(small))
    .children(affects.map(|affects| {
        div()
            .debug_selector(|| "assistant-pending-affects".into())
            .text_sm()
            .child(affects)
    }))
    .child(
        div()
            .when_some(decision, |this, decision| {
                this.debug_selector(move || decision.into())
            })
            .flex()
            .justify_end()
            .gap_2()
            .child(
                Button::new("assistant-reject", reject_label)
                    .debug_selector(|| "assistant-reject".into())
                    .secondary()
                    .disabled(busy)
                    .on_click(cx.listener(RootView::reject_assistant_pending)),
            )
            .child(
                Button::new("assistant-confirm", accept_label)
                    .debug_selector(|| "assistant-confirm".into())
                    .primary()
                    .disabled(busy)
                    .on_click(cx.listener(RootView::accept_assistant_pending)),
            ),
    )
    .into_any_element()
}

/// A field's complete old or new value: the old one muted and struck through,
/// the new one in full color, each marked by a thin bar in its semantic color.
/// Long changes scroll with the review so nothing is hidden before approval.
fn diff_value(value: String, added: bool, tokens: &Tokens) -> gpui::Div {
    div()
        .border_l_2()
        .border_color(if added { tokens.success } else { tokens.danger })
        .pl(tokens.spacing.sm)
        .text_sm()
        .map(|value| {
            if added {
                value.text_color(tokens.text)
            } else {
                value.text_color(tokens.text_muted).line_through()
            }
        })
        .child(value)
}

/// The document and field a diff changes, both by name.
fn field_diff_label(
    locale: Locale,
    diff: &crate::assistant_ui::AssistantFieldDiff,
    title_of: &dyn Fn(&str) -> Option<String>,
) -> String {
    let field = FIELD_LABELS
        .iter()
        .find(|(name, _)| *name == diff.field)
        .map_or_else(|| diff.field.clone(), |(_, key)| tr(locale, key).to_owned());
    format!(
        "{} · {field}",
        quoted_title(locale, &diff.document_id, title_of)
    )
}

/// The labels of the document fields the capability reports by name.
const FIELD_LABELS: [(&str, &str); 5] = [
    ("title", "document_title"),
    ("tags", "document_tags"),
    ("summary", "document_summary"),
    ("markdown", "document_body"),
    ("document_status", "document_status"),
];

/// Where a tree change moves a document: under another parent, to the top
/// level, or to a new place among its siblings.
fn tree_change_text(
    locale: Locale,
    change: &crate::assistant_ui::AssistantTreeChange,
    title_of: &dyn Fn(&str) -> Option<String>,
) -> String {
    let document = quoted_title(locale, &change.document_id, title_of);
    match (
        change.previous_parent_id == change.new_parent_id,
        change.new_sibling_order,
    ) {
        (true, Some(order)) => crate::i18n::message_pair(
            locale,
            "assistant_tree_reordered",
            &document,
            &(order + 1).to_string(),
        ),
        _ => {
            let place = match &change.new_parent_id {
                Some(parent) => crate::i18n::message(
                    locale,
                    "assistant_tree_under",
                    &quoted_title(locale, parent, title_of),
                ),
                None => tr(locale, "assistant_tree_root").into(),
            };
            crate::i18n::message_pair(locale, "assistant_tree_moved", &document, &place)
        }
    }
}

/// The transcript segments a range covers and the document they go to.
fn range_text(
    locale: Locale,
    range: &crate::assistant_ui::AssistantRangePreview,
    title_of: &dyn Fn(&str) -> Option<String>,
) -> String {
    let target = range.document_id.as_deref().map_or_else(
        || tr(locale, "assistant_new_document").to_owned(),
        |id| quoted_title(locale, id, title_of),
    );
    format!(
        "{} {} – {} → {target}",
        tr(locale, "assistant_range"),
        range.start_segment_id,
        range.end_segment_id
    )
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

    fn change(id: &str, previous: Option<u64>, deleted: bool) -> AssistantDocumentChange {
        AssistantDocumentChange {
            document_id: id.into(),
            previous_revision: previous,
            revision: (!deleted).then_some(previous.map_or(1, |revision| revision + 1)),
            deleted,
        }
    }

    fn titles(id: &str) -> Option<String> {
        (id == "weekly").then(|| "产品周会".into())
    }

    #[test]
    fn a_receipt_names_what_it_changed_by_document_title() {
        let mut write = mutation("m");
        write.affected_documents = vec![change("weekly", Some(3), false)];
        assert_eq!(
            receipt_summary(Locale::ZhCn, &write, &titles),
            "已更新「产品周会」"
        );
        assert_eq!(
            receipt_summary(Locale::EnUs, &write, &titles),
            "Updated “产品周会”"
        );

        write.affected_documents = vec![
            change("topic-1", None, false),
            change("topic-2", None, false),
            change("weekly", Some(3), false),
        ];
        assert_eq!(
            receipt_summary(Locale::ZhCn, &write, &titles),
            "已新建「topic-1」、「topic-2」；已更新「产品周会」",
            "a document missing from the list keeps its id"
        );
        assert_eq!(
            receipt_summary(Locale::EnUs, &write, &titles),
            "Created “topic-1”, “topic-2”; updated “产品周会”"
        );

        write.affected_documents = vec![change("weekly", Some(3), true)];
        assert_eq!(
            receipt_summary(Locale::ZhCn, &write, &titles),
            "已删除「产品周会」"
        );
        write.affected_documents.clear();
        assert_eq!(receipt_summary(Locale::ZhCn, &write, &titles), "已写入变更");
        assert_eq!(
            receipt_summary(Locale::EnUs, &write, &titles),
            "Change written"
        );
    }

    #[test]
    fn a_receipt_keeps_ids_and_revisions_as_details_and_drops_empty_gaps() {
        let mut write = mutation("mutation-weekly-title");
        write.affected_documents = vec![change("weekly", Some(3), false)];
        assert_eq!(receipt_details(&write), "mutation-weekly-title · r3 → r4");
        assert_eq!(
            uncovered_text(Locale::ZhCn, &write.uncovered_segment_ids),
            None
        );

        write.affected_documents.push(change("topic", None, false));
        write.uncovered_segment_ids = vec!["seg-021".into()];
        assert_eq!(
            receipt_details(&write),
            "mutation-weekly-title · r3 → r4, — → r1"
        );
        assert_eq!(
            uncovered_text(Locale::ZhCn, &write.uncovered_segment_ids).as_deref(),
            Some("1 段转写未纳入")
        );
        write.uncovered_segment_ids.push("seg-022".into());
        assert_eq!(
            uncovered_text(Locale::EnUs, &write.uncovered_segment_ids).as_deref(),
            Some("2 transcript segments left out")
        );
    }

    fn tree_change(
        previous: (Option<&str>, Option<u32>),
        new: (Option<&str>, Option<u32>),
    ) -> crate::assistant_ui::AssistantTreeChange {
        crate::assistant_ui::AssistantTreeChange {
            document_id: "decisions".into(),
            previous_parent_id: previous.0.map(Into::into),
            new_parent_id: new.0.map(Into::into),
            previous_sibling_order: previous.1,
            new_sibling_order: new.1,
        }
    }

    fn pending_titles(id: &str) -> Option<String> {
        match id {
            "weekly" => Some("产品周会".into()),
            "decisions" => Some("决策与行动项".into()),
            _ => None,
        }
    }

    #[test]
    fn a_review_names_documents_and_fields_by_their_titles() {
        let diff = crate::assistant_ui::AssistantFieldDiff {
            document_id: "decisions".into(),
            field: "summary".into(),
            before: None,
            after: Some("确认发布".into()),
        };
        assert_eq!(
            field_diff_label(Locale::ZhCn, &diff, &pending_titles),
            "「决策与行动项」 · 总结"
        );
        for (_, key) in FIELD_LABELS {
            for locale in [Locale::ZhCn, Locale::EnUs] {
                assert!(!tr(locale, key).is_empty(), "{key} has a label");
            }
        }
        let unknown = crate::assistant_ui::AssistantFieldDiff {
            field: "speakerNote".into(),
            ..diff
        };
        assert_eq!(
            field_diff_label(Locale::EnUs, &unknown, &pending_titles),
            "“决策与行动项” · speakerNote",
            "a field without a label keeps its name"
        );

        let moved = tree_change((None, Some(1)), (Some("weekly"), Some(0)));
        assert_eq!(
            tree_change_text(Locale::ZhCn, &moved, &pending_titles),
            "「决策与行动项」移到「产品周会」下"
        );
        assert_eq!(
            tree_change_text(Locale::EnUs, &moved, &pending_titles),
            "Moves “决策与行动项” under “产品周会”"
        );
        let lifted = tree_change((Some("weekly"), Some(0)), (None, Some(2)));
        assert_eq!(
            tree_change_text(Locale::ZhCn, &lifted, &pending_titles),
            "「决策与行动项」移到顶层"
        );
        assert_eq!(
            tree_change_text(Locale::EnUs, &lifted, &pending_titles),
            "Moves “决策与行动项” to the top level"
        );
        let reordered = tree_change((Some("weekly"), Some(2)), (Some("weekly"), Some(0)));
        assert_eq!(
            tree_change_text(Locale::ZhCn, &reordered, &pending_titles),
            "「决策与行动项」调整到同级第 1 位"
        );
        assert_eq!(
            tree_change_text(Locale::EnUs, &reordered, &pending_titles),
            "Moves “决策与行动项” to position 1 among its siblings"
        );

        let range = crate::assistant_ui::AssistantRangePreview {
            document_id: None,
            start_segment_id: "seg-014".into(),
            end_segment_id: "seg-020".into(),
            evidence_segment_ids: vec!["seg-014".into()],
        };
        assert_eq!(
            range_text(Locale::ZhCn, &range, &pending_titles),
            "转写片段 seg-014 – seg-020 → 新文档"
        );
        let range = crate::assistant_ui::AssistantRangePreview {
            document_id: Some("decisions".into()),
            ..range
        };
        assert_eq!(
            range_text(Locale::EnUs, &range, &pending_titles),
            "Transcript seg-014 – seg-020 → “决策与行动项”"
        );
    }

    #[test]
    fn run_details_leave_out_what_the_run_did_not_report() {
        let mut result = AssistantRunResult {
            answer: Some("answer".into()),
            warning: None,
            warnings: Vec::new(),
            evidence_segment_ids: Vec::new(),
            usage: None,
            requested_model: None,
            resolved_model: None,
            mutation: None,
            mutations: Vec::new(),
            pending: None,
            pending_reviews: Vec::new(),
            diagnostics: None,
        };
        assert!(
            result_details(Locale::EnUs, &result).is_empty(),
            "no dashes for nothing"
        );

        result.evidence_segment_ids = vec!["seg-001".into(), "seg-014".into()];
        result.requested_model = Some("assistant-default".into());
        result.resolved_model = Some("assistant-default".into());
        result.usage = Some(crate::assistant_ui::AssistantUsage {
            input_tokens: 1_840,
            output_tokens: 260,
            total_tokens: 2_100,
            cost_microusd: None,
        });
        assert_eq!(
            result_details(Locale::EnUs, &result),
            [
                ("Evidence segments", "seg-001, seg-014".to_owned()),
                ("Model", "assistant-default".to_owned()),
                ("Input/output/total tokens", "1840 / 260 / 2100".to_owned()),
            ]
        );

        result.resolved_model = Some("gpt-x".into());
        result.usage = result
            .usage
            .map(|usage| crate::assistant_ui::AssistantUsage {
                cost_microusd: Some(25),
                ..usage
            });
        let details = result_details(Locale::EnUs, &result);
        assert_eq!(details[1].1, "assistant-default → gpt-x");
        assert_eq!(details[2].1, "1840 / 260 / 2100 · USD 0.000025");
    }

    #[test]
    fn the_context_names_the_documents_and_connection_without_ids() {
        use AssistantScopeMode::{AllVisibleDocuments, CurrentDocument};
        assert_eq!(
            context_parts(
                Locale::ZhCn,
                CurrentDocument,
                Some("产品周会"),
                3,
                Some(("Local LiteLLM", true))
            ),
            ["产品周会", "Local LiteLLM", "本机连接"]
        );
        assert_eq!(
            context_parts(
                Locale::EnUs,
                AllVisibleDocuments,
                Some("Weekly sync"),
                3,
                Some(("Cloud", false))
            ),
            ["3 documents", "Cloud", "Remote connection"]
        );
        assert_eq!(
            context_parts(Locale::EnUs, AllVisibleDocuments, None, 1, None),
            ["1 document", "No connection"]
        );
        assert_eq!(
            context_parts(Locale::ZhCn, CurrentDocument, None, 3, None),
            ["未选择文档", "未配置连接"]
        );
    }

    #[test]
    fn partial_and_failed_task_labels_read_as_sentences_without_codes() {
        let partial = task_label(
            Locale::EnUs,
            &AssistantTaskState::Partial(warning("PROVIDER_TIMEOUT")),
        );
        assert!(partial.contains("partially"), "{partial}");
        assert!(partial.contains("took too long"), "{partial}");
        assert!(!partial.contains("PROVIDER_TIMEOUT"), "{partial}");
        let failed = task_label(
            Locale::ZhCn,
            &AssistantTaskState::Failed(warning("PROVIDER_AUTHENTICATION_FAILED")),
        );
        assert!(failed.contains("API key 无效或已过期"), "{failed}");
        assert!(!failed.contains("PROVIDER_"), "{failed}");
        assert_eq!(
            task_label(
                Locale::ZhCn,
                &AssistantTaskState::Running {
                    request_id: "assistant-1-2".into()
                }
            ),
            "助手正在运行",
            "the request id stays out of the label"
        );
        assert_eq!(
            task_label(
                Locale::EnUs,
                &AssistantTaskState::Cancelling {
                    request_id: "assistant-1-2".into()
                }
            ),
            "Cancelling"
        );
    }

    #[test]
    fn the_status_shows_while_a_run_goes_on_or_has_an_outcome_and_counts_known_steps() {
        assert!(!shows_status(&AssistantTaskState::Idle));
        for task in [
            AssistantTaskState::Running {
                request_id: "run".into(),
            },
            AssistantTaskState::Completed,
            AssistantTaskState::Cancelled,
            AssistantTaskState::Failed(warning("TASK_FAILED")),
        ] {
            assert!(shows_status(&task), "{task:?}");
        }
        let progress = |completed, total| runtime::Progress { completed, total };
        assert_eq!(
            job_progress_text(progress(1, Some(3))).as_deref(),
            Some("1 / 3")
        );
        assert_eq!(job_progress_text(progress(0, None)), None);
    }

    /// `AgentErrorCode` as it crosses the service boundary:
    /// `ProviderTimeout` becomes `PROVIDER_TIMEOUT`.
    fn screaming_snake(code: contracts::AgentErrorCode) -> String {
        let mut name = String::new();
        for (index, letter) in format!("{code:?}").chars().enumerate() {
            if letter.is_uppercase() && index > 0 {
                name.push('_');
            }
            name.push(letter.to_ascii_uppercase());
        }
        name
    }

    /// Every code of the agent contract. The match stops compiling when the
    /// contract gains a code, so a new code is mapped before it ships.
    fn every_agent_code() -> Vec<String> {
        use contracts::AgentErrorCode as Code;
        let codes = [
            Code::ToolUnknown,
            Code::ToolNotAllowed,
            Code::ToolArgumentsInvalid,
            Code::ToolLoopLimitReached,
            Code::SkillNotFound,
            Code::DocumentNotFound,
            Code::DocumentAccessDenied,
            Code::DocumentRevisionConflict,
            Code::DocumentFieldReadOnly,
            Code::SegmentNotFound,
            Code::RangeInvalidOrder,
            Code::RangeOutsideParent,
            Code::RangeOverlap,
            Code::RangeCoverageGap,
            Code::TreeParentNotFound,
            Code::TreeCycle,
            Code::TreeMaxDepthExceeded,
            Code::TreeCrossAudioAsset,
            Code::WritePolicyRejected,
            Code::WriteConfirmationExpired,
            Code::IdempotencyConflict,
            Code::WriteTransactionFailed,
            Code::ConnectionNotConfigured,
            Code::ProviderCapabilityUnsupported,
            Code::ProviderAuthenticationFailed,
            Code::ProviderRateLimited,
            Code::ProviderTimeout,
            Code::ProviderResponseInvalid,
            Code::ContextLimitUnknown,
            Code::ContextLimitExceeded,
            Code::TaskCancelled,
        ];
        for code in codes {
            match code {
                Code::ToolUnknown
                | Code::ToolNotAllowed
                | Code::ToolArgumentsInvalid
                | Code::ToolLoopLimitReached
                | Code::SkillNotFound
                | Code::DocumentNotFound
                | Code::DocumentAccessDenied
                | Code::DocumentRevisionConflict
                | Code::DocumentFieldReadOnly
                | Code::SegmentNotFound
                | Code::RangeInvalidOrder
                | Code::RangeOutsideParent
                | Code::RangeOverlap
                | Code::RangeCoverageGap
                | Code::TreeParentNotFound
                | Code::TreeCycle
                | Code::TreeMaxDepthExceeded
                | Code::TreeCrossAudioAsset
                | Code::WritePolicyRejected
                | Code::WriteConfirmationExpired
                | Code::IdempotencyConflict
                | Code::WriteTransactionFailed
                | Code::ConnectionNotConfigured
                | Code::ProviderCapabilityUnsupported
                | Code::ProviderAuthenticationFailed
                | Code::ProviderRateLimited
                | Code::ProviderTimeout
                | Code::ProviderResponseInvalid
                | Code::ContextLimitUnknown
                | Code::ContextLimitExceeded
                | Code::TaskCancelled => {}
            }
        }
        codes.into_iter().map(screaming_snake).collect()
    }

    fn has_chinese(text: &str) -> bool {
        text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
    }

    fn error(code: &str, message: &str) -> AssistantUiError {
        AssistantUiError {
            code: code.into(),
            message: message.into(),
            retryable: false,
        }
    }

    #[test]
    fn every_known_error_code_reads_as_a_sentence_with_what_to_do_next() {
        assert_eq!(
            screaming_snake(contracts::AgentErrorCode::ProviderTimeout),
            "PROVIDER_TIMEOUT"
        );
        let codes = every_agent_code();
        for code in &codes {
            assert!(
                KNOWN_ERRORS.iter().any(|(known, _)| known == code),
                "{code} of the agent contract has no wording"
            );
        }
        for (code, _) in KNOWN_ERRORS {
            let raw = error(code, "raw provider message");
            let (chinese, english) = (
                describe_error(Locale::ZhCn, &raw),
                describe_error(Locale::EnUs, &raw),
            );
            for copy in [&chinese, &english] {
                assert!(!copy.title.contains(code), "{code}: {copy:?}");
                assert!(!copy.title.contains("raw provider"), "{code}: {copy:?}");
                assert!(copy.hint.is_some_and(|hint| !hint.is_empty()), "{code}");
            }
            assert!(has_chinese(&chinese.title), "{code}: {chinese:?}");
            assert!(chinese.hint.is_some_and(has_chinese), "{code}: {chinese:?}");
            assert!(!has_chinese(&english.title), "{code}: {english:?}");
            assert!(
                !english.hint.is_some_and(has_chinese),
                "{code}: {english:?}"
            );
            assert_eq!(chinese.opens_settings, english.opens_settings, "{code}");
            assert_eq!(
                error_details(&raw),
                format!("{code} · raw provider message"),
                "the code stays for troubleshooting"
            );
        }

        let authentication = describe_error(
            Locale::ZhCn,
            &error("PROVIDER_AUTHENTICATION_FAILED", "401 Unauthorized"),
        );
        assert_eq!(authentication.title, "API key 无效或已过期");
        assert!(authentication.opens_settings);
        for code in [
            "ASSISTANT_API_KEY_FILE_READ_FAILED",
            "CONNECTION_NOT_CONFIGURED",
            "ASSISTANT_DISABLED",
            "CONTEXT_LIMIT_UNKNOWN",
        ] {
            assert!(
                describe_error(Locale::EnUs, &error(code, "")).opens_settings,
                "{code}"
            );
        }
        for code in [
            "PROVIDER_RATE_LIMITED",
            "DOCUMENT_REVISION_CONFLICT",
            "TASK_CANCELLED",
        ] {
            assert!(
                !describe_error(Locale::EnUs, &error(code, "")).opens_settings,
                "{code}"
            );
        }
        assert_eq!(
            error_details(&error("PROVIDER_TIMEOUT", " ")),
            "PROVIDER_TIMEOUT"
        );
    }

    #[test]
    fn an_unknown_error_code_falls_back_to_its_message_and_keeps_the_code() {
        let notice = error("PROVIDER_NOTICE", " 上下文按窗口截断到 128000 token ");
        let copy = describe_error(Locale::ZhCn, &notice);
        assert_eq!(copy.title, "上下文按窗口截断到 128000 token");
        assert_eq!((copy.hint, copy.opens_settings), (None, false));
        assert_eq!(error_details(&notice), "PROVIDER_NOTICE");

        let silent = error("SOMETHING_NEW", "  ");
        assert_eq!(
            describe_error(Locale::EnUs, &silent).title,
            "The Assistant task did not finish"
        );
        assert_eq!(
            describe_error(Locale::ZhCn, &silent).title,
            "助手任务没有完成"
        );
    }
}
