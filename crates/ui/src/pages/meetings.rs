use crate::{
    components::{
        document_meta, empty_state, ordered_projects, row_indicator, status_label, timestamp,
        tree_row_label, DragGhost, ProjectDrag, RowIndicator,
    },
    controller::RootView,
    controls::{tree, Button, Collapsible, ProgressBar, RadioGroup, Scrollbar, Tree, TreeItem},
    i18n::{self, text as tr},
    route::Route,
    speakers::{document_speakers, transcript_turns, DocumentSpeaker},
    state::{self, ContentMode, MissingModels, ProcessingFeedback, ProcessingStage, StageFeedback},
    theme::{self, Tokens},
};
use domain::{DocumentState, ProjectStatus};
use gpui::{
    div, percentage, prelude::*, px, relative, svg, Animation, AnimationExt as _, AnyElement,
    Context, ElementId, IntoElement, Pixels, Transformation,
};
use std::time::Duration;

/// The tallest the issue list grows before it scrolls.
const ISSUE_LIST_HEIGHT: Pixels = px(168.);
/// The group of a tree row, whose hover shows its drag handle.
const ROW_GROUP: &str = "project-row";
/// The drag handle's column in a tree row.
const GRIP_WIDTH: Pixels = px(14.);
/// The status dot at the end of a tree row.
const STATUS_DOT: Pixels = px(6.);
/// The widest a line of the summary or transcript runs.
pub(crate) const READING_WIDTH: Pixels = px(680.);
/// The line height of the reading column, relative to its text.
const READING_LINE_HEIGHT: f32 = 1.7;
/// The widest the processing progress grows.
const RECOVERY_MESSAGE_HEIGHT: Pixels = px(96.);
/// One turn of the processing spinner.
const SPIN: Duration = Duration::from_millis(1_000);
/// The width of the document status select.
const STATUS_SELECT_WIDTH: Pixels = px(240.);
/// The name column of a speaker row.
const SPEAKER_NAME_WIDTH: Pixels = px(160.);
/// The width of a speaker's name field and merge select.
const SPEAKER_FIELD_WIDTH: Pixels = px(200.);

/// The current document in the content plane; its project tree lives in
/// the sidebar (`project_tree`).
pub(crate) fn render(
    root: &mut RootView,
    selected: Option<&DocumentState>,
    viewport_height: Pixels,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    main_panel(root, selected, viewport_height, cx).debug_selector(|| "meetings-page".into())
}

/// The three-level project tree the sidebar shows on the Meetings route:
/// click selects, the handle drags, dropping onto a row reparents and
/// between rows reorders. The tree is one tab stop; the arrow keys move the
/// selection and expand or collapse the selected row, and Option with the
/// arrows moves the selected row as dragging does.
pub(crate) fn project_tree(root: &mut RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let tokens = *theme::tokens(cx);
    let scroll = root.scroll.project_tree.clone();
    let rows = ordered_projects(&root.projects, &root.ui.expanded_projects);
    let has_selection = rows
        .iter()
        .any(|(project, _)| root.ui.selected_project.as_ref() == Some(project.id()));
    let mut tree = Tree::new(
        "project-tree",
        tr(root.ui.locale, "audio_projects"),
        &root.tree_focus,
    )
    .debug_selector(|| "project-tree".into())
    .description(tr(root.ui.locale, "project_tree_shortcuts"))
    .has_items(!rows.is_empty())
    .has_selection(has_selection)
    .reveal_selection(std::mem::take(&mut root.reveal_tree_selection))
    .on_step(cx.listener(|this, step, window, cx| this.step_project_tree(*step, window, cx)))
    .when(!root.documents_root_switch.switching, |tree| {
        tree.on_shift(cx.listener(|this, shift, _, cx| this.shift_project(*shift, cx)))
    })
    .size_full()
    .overflow_y_scroll()
    .track_scroll(&scroll)
    .flex()
    .flex_col();
    if root.projects.is_empty() {
        tree = tree.child(
            div()
                .p_3()
                .text_sm()
                .text_color(tokens.text_muted)
                .when(root.projects_error.is_some(), |empty| {
                    empty.debug_selector(|| "project-tree-unavailable".into())
                })
                .child(tr(
                    root.ui.locale,
                    if root.projects_error.is_some() {
                        "documents_unavailable"
                    } else {
                        "no_projects"
                    },
                )),
        );
    } else {
        for (project, depth) in rows {
            let selected = root.ui.selected_project.as_ref() == Some(project.id());
            let processing = root.ui.is_processing_document(project.id());
            let display_status = root.ui.processing_status(&project);
            let project_id = project.id().clone();
            let drag_id = project.id().clone();
            let drag_title = project.title().to_string();
            let target_id = project.id().clone();
            let has_children = root
                .projects
                .iter()
                .any(|child| child.hierarchy.parent_id.as_ref() == Some(project.id()));
            let expanded = root.ui.expanded_projects.contains(project.id());
            let id = project.id().as_str().to_string();
            let audio = root.audio.problem(project.id());
            // The handle shows on the selected row and the one under the
            // pointer; it is there to drag from either way.
            let drag_handle = div()
                .id(format!("drag-{id}"))
                .debug_selector(|| format!("drag-{}", project.id().as_str()))
                .flex_shrink_0()
                .w(GRIP_WIDTH)
                .flex()
                .justify_center()
                .cursor_grab()
                .child(
                    svg()
                        .path("icons/ellipsis-vertical.svg")
                        .size(px(14.))
                        .text_color(tokens.text_muted),
                )
                .when(!selected, |grip| {
                    grip.opacity(0.)
                        .group_hover(ROW_GROUP, |style| style.opacity(1.))
                })
                .on_drag(
                    ProjectDrag {
                        project_id: drag_id,
                        revision: project.revision(),
                        root_generation: root.documents_root_generation,
                    },
                    move |_, _, _, cx| {
                        cx.new(|_| DragGhost {
                            title: drag_title.clone(),
                        })
                    },
                );
            let expand_handle = if has_children {
                let expand_id = project.id().clone();
                tree::disclosure(
                    format!("expand-{id}"),
                    expanded,
                    tokens.text_muted,
                    cx.listener(move |this, _, _, cx| this.toggle_project(expand_id.clone(), cx)),
                )
                .into_any_element()
            } else {
                tree::disclosure_space().into_any_element()
            };
            let before_target = project.id().clone();
            let after_target = project.id().clone();
            let before_drop = div()
                .id(format!("before-{id}"))
                .debug_selector(|| format!("before-{}", project.id().as_str()))
                .h(px(5.))
                .on_drop(cx.listener(move |this, dragged: &ProjectDrag, _, cx| {
                    this.reorder_document(dragged.clone(), before_target.clone(), true, cx)
                }));
            let after_drop = div()
                .id(format!("after-{id}"))
                .debug_selector(|| format!("after-{}", project.id().as_str()))
                .h(px(5.))
                .on_drop(cx.listener(move |this, dragged: &ProjectDrag, _, cx| {
                    this.reorder_document(dragged.clone(), after_target.clone(), false, cx)
                }));
            let row = TreeItem::new(
                id.clone(),
                tree_row_label(root.ui.locale, project.title(), display_status, audio),
                &root.tree_focus,
            )
            .debug_selector(|| format!("project-row-{}", project.id().as_str()))
            .group(ROW_GROUP)
            .depth(depth.into())
            .selected(selected)
            .expanded(has_children.then_some(expanded))
            .cursor_pointer()
            .px(tokens.spacing.xs)
            .py(tokens.spacing.xs)
            .ml(px((depth as f32) * 16.))
            .rounded(tokens.radius.sm)
            .map(|row| {
                if selected {
                    row.bg(tokens.selection)
                } else {
                    row.hover(|style| style.bg(tokens.wash))
                }
            })
            .child(
                div()
                    .debug_selector({
                        let id = id.clone();
                        move || format!("project-status-{id}-{display_status:?}")
                    })
                    .flex()
                    .items_center()
                    .gap(tokens.spacing.xs)
                    .child(drag_handle)
                    .child(expand_handle)
                    .child(
                        div()
                            .debug_selector({
                                let id = id.clone();
                                move || format!("project-title-{id}")
                            })
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .child(theme::tail_faded_label(
                                format!("project-title-{id}"),
                                project.title().to_string(),
                                if selected {
                                    tokens.selection
                                } else {
                                    tokens.sidebar
                                },
                            )),
                    )
                    .child(row_end(
                        &id,
                        row_indicator(project.processing_status(), processing, audio),
                        root.ui.locale,
                        &tokens,
                    )),
            )
            .on_click(
                cx.listener(move |this, _, _, cx| this.select_project(project_id.clone(), cx)),
            )
            .on_drop(cx.listener(move |this, dragged: &ProjectDrag, _, cx| {
                this.reparent_document(dragged.clone(), target_id.clone(), cx)
            }));
            tree = tree.child(div().child(before_drop).child(row).child(after_drop));
        }
    }
    // Inset like the entries above, so the hairline stops short of the
    // window edge.
    div()
        .flex_1()
        .min_h_0()
        .mx(tokens.spacing.md)
        .mb(tokens.spacing.md)
        .pt(tokens.spacing.md)
        .gap(tokens.spacing.sm)
        .flex()
        .flex_col()
        .border_t_1()
        .border_color(tokens.hairline)
        .child(
            div()
                .px(tokens.spacing.sm)
                .text_xs()
                .text_color(tokens.text_muted)
                .child(tr(root.ui.locale, "audio_projects")),
        )
        // The scrollbar stays outside the fade.
        .child(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .child(theme::scroll_faded(&scroll, tokens.sidebar, tree))
                .child(
                    Scrollbar::vertical("project-tree-scrollbar", &scroll)
                        .outset_right(tokens.spacing.md),
                ),
        )
}

/// The end of tree row `id`: a spinner, a warning, the word for a failure
/// or a status dot, as `indicator` says.
fn row_end(id: &str, indicator: RowIndicator, locale: i18n::Locale, tokens: &Tokens) -> AnyElement {
    let dot = |color| {
        let id = id.to_string();
        div()
            .debug_selector(move || format!("status-dot-{id}"))
            .flex_shrink_0()
            .size(STATUS_DOT)
            .rounded_full()
            .bg(color)
    };
    let end = div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap(tokens.spacing.xs)
        .pr(px(2.));
    match indicator {
        RowIndicator::Spinner => {
            let selector = format!("processing-indicator-{id}");
            end.debug_selector(move || selector.clone())
                .child(spinner(format!("processing-spinner-{id}"), tokens))
        }
        RowIndicator::Audio => {
            let selector = format!("audio-indicator-{id}");
            end.debug_selector(move || selector.clone()).child(
                svg()
                    .path("icons/triangle-alert.svg")
                    .size(px(14.))
                    .text_color(tokens.warning_text),
            )
        }
        RowIndicator::Failed => end
            .child(
                div()
                    .text_xs()
                    .text_color(tokens.danger)
                    .child(status_label(locale, ProjectStatus::Failed)),
            )
            .child(dot(tokens.danger)),
        RowIndicator::Dot(status) => end.child(dot(match status {
            ProjectStatus::Ready => tokens.success,
            ProjectStatus::Processing => tokens.warning,
            ProjectStatus::Failed => tokens.danger,
            ProjectStatus::Imported => tokens.text_subtle,
        })),
    }
    .into_any_element()
}

fn main_panel(
    root: &mut RootView,
    project: Option<&DocumentState>,
    viewport_height: Pixels,
    cx: &mut Context<RootView>,
) -> gpui::Div {
    let locale = root.ui.locale;
    let tokens = *theme::tokens(cx);
    // A different workspace, document or view starts at its beginning;
    // refreshing the same content keeps the user's reading position.
    let context = project.map(|project| {
        (
            root.documents_root_generation,
            project.id().clone(),
            root.ui.content_mode,
        )
    });
    if root.scroll.meeting_context != context {
        root.scroll.meeting_context = context;
        root.scroll.meeting_content.set_offset(Default::default());
    }
    // Compact the metadata beside Detail in a short window so recovery
    // feedback still leaves room for reading and editing.
    let compact = root.ui.detail_open
        && viewport_height < super::chrome::MIN_WINDOW_SIZE.height + RECOVERY_MESSAGE_HEIGHT;
    let inset = if compact {
        tokens.spacing.md
    } else {
        tokens.spacing.xl
    };
    let gap = if compact {
        tokens.spacing.xs
    } else {
        tokens.spacing.lg
    };
    let processing_here =
        project.is_some_and(|project| root.ui.is_processing_document(project.id()));
    let display_status = project.map(|project| {
        if processing_here {
            ProjectStatus::Processing
        } else {
            project.processing_status()
        }
    });
    let mut panel = div()
        .flex_1()
        .min_w_0()
        .h_full()
        .p(inset)
        .flex()
        .flex_col()
        .gap(gap)
        .children(workspace_read_error(root, &tokens, cx))
        .children(conflict_notice(root, &tokens, cx))
        .children(issues_panel(root, &tokens, cx));
    if let Some(project) = project {
        let ready = project.processing.latest.is_some();
        let original_available = ready
            || !project.results.transcript.is_empty()
            || !project.results.speaker_segments.is_empty();
        let mode = root.ui.content_mode;
        let scroll = root.scroll.meeting_content.clone();
        let empty = (match mode {
            ContentMode::Edit | ContentMode::Markdown => false,
            ContentMode::Original => !original_available,
            ContentMode::Summary => !ready,
        })
        .then(|| unprocessed_state(root, project, &tokens, cx));
        let mut content = div()
            .id("meeting-content")
            .debug_selector(|| "meeting-content".into())
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .p(if compact {
                tokens.spacing.md
            } else {
                tokens.spacing.lg
            })
            .rounded(tokens.radius.md)
            .bg(tokens.surface);
        content = match mode {
            ContentMode::Edit => content.child(editor_form(root, project, &tokens, cx)),
            ContentMode::Markdown => content.child(saved_markdown(root, project, &tokens, cx)),
            ContentMode::Summary if ready => content.child(match &project.content.summary {
                Some(summary) => {
                    reading(summary.clone()).debug_selector(|| "document-summary".into())
                }
                None => reading(tr(locale, "no_summary")).text_color(tokens.text_muted),
            }),
            ContentMode::Original if original_available => content
                .child(speakers_panel(root, project, &tokens, cx))
                .child(transcript(project, locale, &tokens)),
            _ => content,
        };
        panel = panel.child(document_header(
            root,
            project,
            display_status.unwrap_or(ProjectStatus::Imported),
            &tokens,
            cx,
        ));
        // Summary and Edit always; the speaker-labelled original once
        // processed.
        let mut controls = RadioGroup::new("content-mode", tr(locale, "content_mode"))
            .debug_selector(|| "content-mode-group".into())
            .segmented()
            .flex_shrink_0()
            .self_start()
            .option(
                mode == ContentMode::Summary,
                crate::components::mode_button(tr(locale, "summary"), ContentMode::Summary, cx),
            );
        if original_available {
            controls = controls.option(
                mode == ContentMode::Original,
                crate::components::mode_button(tr(locale, "original"), ContentMode::Original, cx),
            );
        }
        controls = controls.option(
            mode == ContentMode::Markdown,
            crate::components::mode_button(tr(locale, "saved_markdown"), ContentMode::Markdown, cx),
        );
        controls = controls.option(
            mode == ContentMode::Edit,
            crate::components::mode_button(tr(locale, "edit"), ContentMode::Edit, cx),
        );
        panel = panel.child(controls);
        panel = panel.child(match empty {
            Some(empty) => empty_region(empty),
            // The scrollbar stays outside the fade.
            None => div()
                .relative()
                .flex_1()
                .min_h_0()
                .child(
                    theme::scroll_faded(&scroll, tokens.surface, content).rounded(tokens.radius.md),
                )
                .child(
                    Scrollbar::vertical("meeting-content-scrollbar", &scroll).outset_right(inset),
                ),
        });
    } else if root.projects_error.is_some() {
        // The retained recovery panel explains why no selection is available.
    } else if root.projects.is_empty() {
        panel = panel.child(empty_region(empty_state(
            "icons/inbox.svg",
            tr(locale, "no_projects_title"),
            tr(locale, "no_projects_hint"),
            Some(
                Button::new("empty-import-audio", tr(locale, "import_audio"))
                    .debug_selector(|| "empty-import-audio".into())
                    .primary()
                    .on_click(cx.listener(RootView::import_audio)),
            ),
            &tokens,
        )));
    } else {
        panel = panel.child(empty_region(empty_state(
            "icons/file.svg",
            tr(locale, "no_selection_title"),
            tr(locale, "no_selection_hint"),
            None,
            &tokens,
        )));
    }
    panel
}

/// The models processing would run with that are not installed.
fn missing_models(root: &RootView) -> MissingModels {
    state::missing_models(
        &root.installation,
        &root.settings_draft,
        (&root.services.asr_model, &root.services.diarization_model),
    )
}

/// The content region holding a centered empty state.
fn empty_region(empty: gpui::Div) -> gpui::Div {
    div()
        .debug_selector(|| "meeting-content".into())
        .flex_1()
        .min_h_0()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .max_w(px(360.))
                .debug_selector(|| "document-empty".into())
                .child(empty),
        )
}

/// What an unprocessed document shows in place of its summary: that it is
/// being processed, or the one step that processes it.
fn unprocessed_state(
    root: &RootView,
    project: &DocumentState,
    tokens: &Tokens,
    cx: &mut Context<RootView>,
) -> gpui::Div {
    let locale = root.ui.locale;
    if root.ui.is_processing_document(project.id()) {
        return empty_state(
            "icons/loader-circle.svg",
            tr(locale, "processing_empty_title"),
            tr(locale, "processing_empty_hint"),
            None,
            tokens,
        );
    }
    if root.audio.missing(project.id()) {
        return empty_state(
            "icons/triangle-alert.svg",
            tr(locale, "audio_missing_title"),
            tr(locale, "audio_missing_hint"),
            Some(
                Button::new("empty-relink-audio", tr(locale, "relink_audio"))
                    .debug_selector(|| "empty-relink-audio".into())
                    .primary()
                    .disabled(
                        root.audio.relinking.is_some() || root.documents_root_switch.switching,
                    )
                    .on_click(cx.listener(RootView::relink_audio)),
            ),
            tokens,
        );
    }
    if let Some(hint) = missing_models(root)
        .hint_key()
        .filter(|_| root.projects_error.is_none())
    {
        let section = root.model_setup_section(missing_models(root));
        let (action, label) = if section.is_some() {
            ("go-to-model-settings", "choose_model")
        } else {
            ("go-to-models", "go_to_models")
        };
        return empty_state(
            "icons/cpu.svg",
            tr(locale, "transcript_empty_title"),
            tr(locale, hint),
            Some(
                Button::new(action, tr(locale, label))
                    .debug_selector(move || action.into())
                    .primary()
                    .on_click(cx.listener(|this, _, _, cx| {
                        let section = this.model_setup_section(missing_models(this));
                        if let Some(section) = section {
                            this.open_settings_section(section, cx);
                        } else {
                            this.navigate(Route::Models, cx);
                        }
                    })),
            ),
            tokens,
        )
        .debug_selector(|| "models-required".into());
    }
    empty_state(
        "icons/file.svg",
        tr(locale, "transcript_empty_title"),
        tr(locale, "transcript_empty_hint"),
        Some(
            Button::new("empty-process", tr(locale, "process"))
                .debug_selector(|| "empty-process".into())
                .primary()
                // One foreground job at a time.
                .disabled(root.ui.processing || root.documents_root_switch.switching)
                .on_click(cx.listener(RootView::process_selected)),
        ),
        tokens,
    )
}

/// The document's header: its title with the Detail toggle at the right,
/// the file line, then its status and actions on one line, and the
/// progress of its processing while that runs.
fn document_header(
    root: &RootView,
    project: &DocumentState,
    status: ProjectStatus,
    tokens: &Tokens,
    cx: &mut Context<RootView>,
) -> AnyElement {
    let locale = root.ui.locale;
    let processing_here = root.ui.is_processing_document(project.id());
    // One foreground job at a time; another document's is under way.
    let busy_elsewhere = root.ui.processing && !processing_here;
    let audio_problem = root.audio.problem(project.id());
    let process_label = if processing_here {
        "cancel"
    } else if project.processing_status() == ProjectStatus::Ready {
        "reprocess"
    } else {
        "process"
    };
    let detail_open = root.ui.detail_open;
    let missing = missing_models(root);
    // The empty state in the content region explains an unprocessed
    // document's missing models itself.
    let empty_shown =
        root.ui.content_mode != ContentMode::Edit && project.processing.latest.is_none();
    let models_hint = missing.hint_key().filter(|_| {
        !processing_here && audio_problem.is_none() && !empty_shown && root.projects_error.is_none()
    });
    let chip = |selector: &'static str, fill, text, label: &'static str| {
        div()
            .debug_selector(move || selector.into())
            .flex_shrink_0()
            .px(tokens.spacing.sm)
            .py(px(2.))
            .rounded(tokens.radius.sm)
            .bg(fill)
            .text_color(text)
            .text_xs()
            .child(label)
    };
    let actions = div()
        .debug_selector(|| "document-actions".into())
        .min_w_0()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(tokens.spacing.sm)
        .child(
            div()
                .debug_selector(move || format!("document-status-{status:?}"))
                .child(chip(
                    "document-status",
                    tokens.status(status),
                    tokens.text,
                    status_label(locale, status),
                )),
        )
        .children(audio_problem.map(|problem| {
            chip(
                "audio-problem",
                tokens.warning_surface,
                tokens.warning_text,
                tr(locale, problem.label_key()),
            )
        }))
        .child(
            Button::new("process-project", tr(locale, process_label))
                .debug_selector(|| "process-project".into())
                .secondary()
                .compact()
                // Missing audio or models cannot process; relink or
                // install them.
                .disabled(
                    root.documents_root_switch.switching
                        || busy_elsewhere
                        || (!processing_here
                            && (root.audio.missing(project.id()) || missing.any())),
                )
                .on_click(cx.listener(RootView::process_or_cancel)),
        )
        .children(audio_problem.map(|_| {
            Button::new("relink-audio", tr(locale, "relink_audio"))
                .debug_selector(|| "relink-audio".into())
                .secondary()
                .compact()
                .disabled(root.audio.relinking.is_some() || root.documents_root_switch.switching)
                .on_click(cx.listener(RootView::relink_audio))
        }));
    div()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap(tokens.spacing.sm)
        .child(
            div()
                .flex()
                .items_start()
                .gap(tokens.spacing.md)
                .child(
                    div()
                        .debug_selector(|| "document-title".into())
                        .flex_1()
                        .min_w_0()
                        .text_2xl()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .flex()
                        .child(theme::tail_faded_label(
                            format!("document-title-{}", project.id()),
                            project.title().to_string(),
                            tokens.content,
                        )),
                )
                .child(
                    Button::new(
                        "open-project-detail",
                        tr(
                            locale,
                            if detail_open {
                                "hide_detail"
                            } else {
                                "show_detail"
                            },
                        ),
                    )
                    .debug_selector(|| "open-project-detail".into())
                    .focus_handle(&root.detail_trigger)
                    .ghost()
                    .compact()
                    .icon("icons/panel-right.svg")
                    .when(detail_open, |toggle| toggle.bg(tokens.selection))
                    .on_click(cx.listener(RootView::toggle_detail)),
                ),
        )
        .child(
            div()
                .debug_selector(|| "document-audio-path".into())
                .min_w_0()
                .flex()
                .text_sm()
                .text_color(tokens.text_muted)
                .child(
                    div()
                        .debug_selector(|| "document-meta".into())
                        .min_w_0()
                        .flex()
                        .child(theme::tail_faded_label(
                            format!("document-meta-{}", project.id()),
                            document_meta(project, locale),
                            tokens.content,
                        )),
                ),
        )
        .child(
            // Actions and the models hint wrap inside the reading plane
            // beside an open Detail, without clipping their labels.
            div()
                .pt(tokens.spacing.xs)
                .flex()
                .flex_wrap()
                .items_center()
                .gap(tokens.spacing.sm)
                .child(actions)
                .children(models_hint.map(|hint| {
                    model_guidance(
                        hint,
                        root.model_setup_section(missing),
                        locale,
                        false,
                        tokens,
                        cx,
                    )
                })),
        )
        .children(
            processing_here
                .then(|| processing_feedback(&root.ui.processing_feedback, locale, tokens)),
        )
        .into_any_element()
}

/// Prose of the reading column: a measured width and an open line height.
fn reading(text: impl IntoElement) -> gpui::Div {
    div()
        .max_w(READING_WIDTH)
        .text_sm()
        .line_height(relative(READING_LINE_HEIGHT))
        .child(text)
}

fn saved_markdown(
    root: &RootView,
    project: &DocumentState,
    tokens: &Tokens,
    cx: &mut Context<RootView>,
) -> AnyElement {
    let locale = root.ui.locale;
    let saved = root
        .saved_document
        .as_ref()
        .filter(|saved| &saved.document_id == project.id());
    match saved.and_then(|saved| saved.outcome.as_ref()) {
        Some(Ok(record)) => div()
            .flex()
            .flex_col()
            .gap(tokens.spacing.md)
            .child(
                div()
                    .text_xs()
                    .text_color(tokens.text_muted)
                    .child(tr(locale, "saved_markdown_hint")),
            )
            .child(
                reading(record.markdown.clone()).debug_selector(|| "saved-markdown-content".into()),
            )
            .into_any_element(),
        Some(Err(error)) => div()
            .flex()
            .flex_col()
            .gap(tokens.spacing.md)
            .child(
                div()
                    .debug_selector(|| "saved-markdown-error".into())
                    .child(error.clone()),
            )
            .child(
                Button::new("retry-saved-markdown", tr(locale, "retry_document_read"))
                    .debug_selector(|| "retry-saved-markdown".into())
                    .disabled(root.documents_root_switch.switching)
                    .on_click(cx.listener(|root, _, _, cx| {
                        if let Some(id) = root.ui.selected_project.clone() {
                            root.load_saved_document(id, cx);
                        }
                    })),
            )
            .into_any_element(),
        None => div()
            .debug_selector(|| "saved-markdown-loading".into())
            .child(tr(locale, "document_loading"))
            .into_any_element(),
    }
}

/// The speaker-labelled transcript as turns: each speaker's name in its
/// color over the paragraphs it says, with a quiet start time.
fn transcript(project: &DocumentState, locale: i18n::Locale, tokens: &Tokens) -> gpui::Div {
    div().flex().flex_col().gap(tokens.spacing.lg).children(
        transcript_turns(project, locale)
            .into_iter()
            .enumerate()
            .map(|(index, turn)| {
                div()
                    .debug_selector(move || format!("transcript-turn-{index}"))
                    .max_w(READING_WIDTH)
                    .flex()
                    .flex_col()
                    .gap(tokens.spacing.xs)
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(tokens.spacing.sm)
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(tokens.speaker(turn.order))
                                    .child(turn.name),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(tokens.text_muted)
                                    .child(timestamp(turn.start_ms)),
                            ),
                    )
                    .children(turn.paragraphs.into_iter().map(reading))
            }),
    )
}

/// Where the foreground processing of the shown document is: its phase
/// over a thin bar, which sweeps until a stage knows its extent.
fn processing_feedback(
    feedback: &ProcessingFeedback,
    locale: i18n::Locale,
    tokens: &Tokens,
) -> impl IntoElement {
    div()
        .debug_selector(|| "processing-progress".into())
        .w_full()
        .max_w(px(360.))
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap(tokens.spacing.sm)
        .children(
            [ProcessingStage::Transcription, ProcessingStage::Diarization].map(|stage| {
                let state = feedback.stage(stage);
                let value = state.value();
                let stage_id = stage.name();
                div()
                    .debug_selector(move || format!("processing-{stage_id}"))
                    .flex()
                    .flex_col()
                    .gap(tokens.spacing.xs)
                    .child(
                        div()
                            .debug_selector(move || {
                                format!("processing-{stage_id}-{}", state.name())
                            })
                            .flex()
                            .justify_between()
                            .text_xs()
                            .text_color(tokens.text_muted)
                            .child(tr(locale, stage.label_key()))
                            .child(tr(locale, state.status_key())),
                    )
                    .children(
                        matches!(state, StageFeedback::Waiting | StageFeedback::Running(_)).then(
                            || {
                                div()
                                    .debug_selector(move || {
                                        format!(
                                            "processing-{stage_id}-{}",
                                            if value.is_some() {
                                                "determinate"
                                            } else {
                                                "indeterminate"
                                            }
                                        )
                                    })
                                    .child(
                                        ProgressBar::new(
                                            format!("processing-{stage_id}-bar"),
                                            tr(locale, stage.label_key()),
                                        )
                                        .value(value.unwrap_or(0.))
                                        .indeterminate(value.is_none()),
                                    )
                            },
                        ),
                    )
            }),
        )
}

/// A turning loader icon; still under reduced motion.
fn spinner(id: impl Into<ElementId>, tokens: &Tokens) -> AnyElement {
    let icon = svg()
        .path("icons/loader-circle.svg")
        .size(px(14.))
        .text_color(tokens.text_muted);
    if tokens.motion.layout.is_zero() {
        return icon.into_any_element();
    }
    icon.with_animation(id, Animation::new(SPIN).repeat(), |icon, turn| {
        icon.with_transformation(Transformation::rotate(percentage(turn)))
    })
    .into_any_element()
}

/// The speakers of the document above its transcript, each with its
/// segment count, a name field with Rename, and a select that merges it
/// into another speaker after a confirmation.
fn speakers_panel(
    root: &RootView,
    project: &DocumentState,
    tokens: &Tokens,
    cx: &mut Context<RootView>,
) -> AnyElement {
    let locale = root.ui.locale;
    let busy = root.speaker_command.is_some() || root.documents_root_switch.switching;
    // Built for this document during the same render.
    let rows = root
        .speaker_rows
        .as_ref()
        .filter(|rows| &rows.key().0 == project.id());
    let speakers = document_speakers(project);
    let expanded = root.ui.speakers_expanded;
    let summary = i18n::message(locale, "speakers_summary", &speakers.len().to_string());
    let names = speakers
        .iter()
        .map(|speaker| speaker.name(locale))
        .collect::<Vec<_>>()
        .join(match locale {
            i18n::Locale::ZhCn => "、",
            i18n::Locale::EnUs => ", ",
        });
    let header = Collapsible::new("document-speakers-toggle", summary.clone())
        .debug_selector(|| "document-speakers-toggle".into())
        .open(expanded)
        .on_toggle(cx.listener(|this, _, _, cx| this.toggle_speakers(cx)))
        .flex()
        .flex_row()
        .items_center()
        .gap(tokens.spacing.sm)
        .py(tokens.spacing.xs)
        .rounded(tokens.radius.sm)
        .child(
            svg()
                .path(if expanded {
                    "icons/chevron-down.svg"
                } else {
                    "icons/chevron-right.svg"
                })
                .size(px(12.))
                .text_color(tokens.text_muted),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(summary),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(tokens.text_muted)
                .child(theme::tail_faded_label(
                    "document-speaker-names",
                    names,
                    tokens.surface,
                )),
        );
    let mut panel = div()
        .flex()
        .flex_col()
        .gap(tokens.spacing.sm)
        .pt(tokens.spacing.xs)
        .child(
            div()
                .text_xs()
                .text_color(tokens.text_muted)
                .child(tr(locale, "speakers_hint")),
        );
    if speakers.is_empty() {
        panel = panel.child(
            div()
                .text_sm()
                .text_color(tokens.text_muted)
                .child(tr(locale, "no_speakers")),
        );
    }
    for speaker in &speakers {
        let Some(row) = rows.and_then(|rows| rows.row(&speaker.id)) else {
            continue;
        };
        let id = speaker.id.clone();
        let rename_id = id.clone();
        panel = panel.child(
            div()
                .debug_selector(move || format!("speaker-row-{id}"))
                .flex()
                .flex_wrap()
                .items_center()
                .gap(tokens.spacing.sm)
                .child(
                    div()
                        .w(SPEAKER_NAME_WIDTH)
                        .flex_shrink_0()
                        .flex()
                        .flex_col()
                        .child(div().text_sm().child(theme::tail_faded_label(
                            format!("speaker-label-{}", speaker.id),
                            speaker.name(locale),
                            tokens.surface,
                        )))
                        .child(
                            div()
                                .text_xs()
                                .text_color(tokens.text_muted)
                                .child(speaker_facts(speaker, locale)),
                        ),
                )
                .child(marked_control(
                    format!("speaker-name-{}", speaker.id),
                    SPEAKER_FIELD_WIDTH,
                    row.name.clone().into_any_element(),
                ))
                .child(
                    Button::new(
                        format!("rename-speaker-{}", speaker.id),
                        tr(locale, "rename"),
                    )
                    .debug_selector({
                        let id = speaker.id.clone();
                        move || format!("rename-speaker-{id}")
                    })
                    .secondary()
                    .compact()
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.rename_document_speaker(rename_id.clone(), cx)
                    })),
                )
                .children(row.merge_into.as_ref().map(|select| {
                    marked_control(
                        format!("merge-speaker-{}", speaker.id),
                        SPEAKER_FIELD_WIDTH,
                        select.clone().into_any_element(),
                    )
                })),
        );
    }
    div()
        .debug_selector(|| "document-speakers".into())
        .mb(tokens.spacing.lg)
        .pb(tokens.spacing.sm)
        .border_b_1()
        .border_color(tokens.hairline)
        .child(header.content(panel))
        .into_any_element()
}

/// The quiet line under a speaker's name: its segment count.
fn speaker_facts(speaker: &DocumentSpeaker, locale: i18n::Locale) -> String {
    i18n::message(locale, "speaker_segments", &speaker.segments.to_string())
}

/// A fixed-width wrapper that `selector` marks for tests; the control
/// inside carries its own label.
pub(crate) fn marked_control(selector: String, width: Pixels, control: AnyElement) -> gpui::Div {
    div()
        .debug_selector(move || selector.clone())
        .w(width)
        .flex_shrink_0()
        .child(control)
}

/// The notice of a document write another writer's change refused, with
/// the reload that reads the document again.
fn conflict_notice(
    root: &RootView,
    tokens: &Tokens,
    cx: &mut Context<RootView>,
) -> Option<AnyElement> {
    let conflict = root.document_conflict.as_ref()?;
    let locale = root.ui.locale;
    let scroll = root.scroll.document_conflict.clone();
    Some(
        div()
            .debug_selector(|| "document-conflict".into())
            .flex_shrink_0()
            .p_3()
            .rounded(tokens.radius.sm)
            .bg(tokens.warning_surface)
            .text_color(tokens.warning_text)
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .id("document-conflict-message")
                    .debug_selector(|| "document-conflict-message".into())
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .max_h(RECOVERY_MESSAGE_HEIGHT)
                    .overflow_y_scroll()
                    .track_scroll(&scroll)
                    .child(
                        div()
                            .debug_selector(|| "document-conflict-text".into())
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().text_sm().child(i18n::message(
                                locale,
                                "document_conflict",
                                &conflict.title,
                            )))
                            .child(div().text_xs().child(tr(locale, "document_conflict_hint")))
                            .child(div().text_xs().child(format!(
                                "{} {} → {}",
                                tr(locale, "detail_revision"),
                                conflict.expected,
                                conflict.actual
                            ))),
                    )
                    .child(Scrollbar::vertical("document-conflict-scrollbar", &scroll)),
            )
            .child(
                Button::new("reload-conflicted-document", tr(locale, "reload"))
                    .disabled(root.documents_root_switch.switching)
                    .debug_selector(|| "reload-conflicted-document".into())
                    .secondary()
                    .compact()
                    .on_click(cx.listener(RootView::reload_conflicted_document)),
            )
            .into_any_element(),
    )
}

/// The sentence the issues panel shows for `issue`: its kind's in
/// `locale`, or the English sentence of an issue built without a kind.
pub(crate) fn issue_sentence(issue: &application::DocumentIssue, locale: i18n::Locale) -> String {
    issue.kind.as_ref().map_or_else(
        || issue.message.clone(),
        |kind| i18n::document_issue(locale, kind),
    )
}

/// The scanner's issues of the documents directory as the last reload read
/// them: one line with their count, which expands to each path and reason.
/// Nothing when there are none.
fn issues_panel(
    root: &RootView,
    tokens: &Tokens,
    cx: &mut Context<RootView>,
) -> Option<AnyElement> {
    if root.projects_error.is_some() || root.document_issues.is_empty() {
        return None;
    }
    let locale = root.ui.locale;
    let expanded = root.ui.issues_expanded;
    let summary = i18n::message(
        locale,
        "document_issues",
        &root.document_issues.len().to_string(),
    );
    let header = Collapsible::new("document-issues-toggle", summary.clone())
        .debug_selector(|| "document-issues-toggle".into())
        .open(expanded)
        .on_toggle(cx.listener(|this, _, _, cx| this.toggle_issues(cx)))
        .flex()
        .flex_row()
        .items_center()
        .gap(tokens.spacing.sm)
        .child(
            svg()
                .path("icons/triangle-alert.svg")
                .size(px(14.))
                .text_color(tokens.warning_text),
        )
        .child(div().flex_1().min_w_0().text_sm().child(summary))
        .child(
            div()
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(tokens.spacing.xs)
                .text_xs()
                .child(tr(
                    locale,
                    if expanded {
                        "hide_issues"
                    } else {
                        "show_issues"
                    },
                ))
                .child(
                    svg()
                        .path(if expanded {
                            "icons/chevron-up.svg"
                        } else {
                            "icons/chevron-down.svg"
                        })
                        .size(px(12.))
                        .text_color(tokens.warning_text),
                ),
        );
    let scroll = root.scroll.document_issues.clone();
    let list = div()
        .id("document-issue-list")
        .max_h(ISSUE_LIST_HEIGHT)
        .overflow_y_scroll()
        .track_scroll(&scroll)
        .flex()
        .flex_col()
        .gap(tokens.spacing.sm)
        .children(
            root.document_issues
                .iter()
                .enumerate()
                .map(|(index, issue)| {
                    div()
                        .debug_selector(move || format!("document-issue-{index}"))
                        .flex()
                        .flex_col()
                        .child(div().text_xs().child(issue.path.clone()))
                        .child(
                            div()
                                .text_sm()
                                .text_color(tokens.text)
                                .child(issue_sentence(issue, locale)),
                        )
                }),
        );
    Some(
        div()
            .debug_selector(|| "document-issues".into())
            .flex_shrink_0()
            .px(tokens.spacing.md)
            .py(tokens.spacing.sm)
            .rounded(tokens.radius.sm)
            .bg(tokens.warning_surface)
            .text_color(tokens.warning_text)
            .child(
                header.content(
                    // The scrollbar stays outside the fade.
                    div()
                        .relative()
                        .mt(tokens.spacing.sm)
                        .child(theme::scroll_faded(&scroll, tokens.warning_surface, list))
                        .child(
                            Scrollbar::vertical("document-issues-scrollbar", &scroll)
                                .outset_right(tokens.spacing.md),
                        ),
                ),
            )
            .into_any_element(),
    )
}

/// The editor of the document's semantic fields: its draft until Save.
/// Processing and results are not shown here and cannot be edited.
fn editor_form(
    root: &RootView,
    project: &DocumentState,
    tokens: &Tokens,
    cx: &mut Context<RootView>,
) -> AnyElement {
    let locale = root.ui.locale;
    let editor = &root.editor;
    if let Some(error) = editor.read_errors.get(project.id()) {
        return div()
            .debug_selector(|| "document-editor-read-failed".into())
            .w_full()
            .flex()
            .flex_col()
            .items_start()
            .gap(tokens.spacing.md)
            .child(
                div()
                    .w_full()
                    .text_sm()
                    .text_color(tokens.warning_text)
                    .child(error.clone()),
            )
            .child(
                Button::new("retry-document-read", tr(locale, "retry_document_read"))
                    .disabled(root.documents_root_switch.switching)
                    .debug_selector(|| "retry-document-read".into())
                    .secondary()
                    .compact()
                    .on_click(cx.listener(RootView::retry_document_read)),
            )
            .into_any_element();
    }
    if editor.shown() != Some(project.id()) {
        return div()
            .debug_selector(|| "document-editor-loading".into())
            .text_sm()
            .text_color(tokens.text_muted)
            .child(tr(locale, "document_loading"))
            .into_any_element();
    }
    let field = |label: &'static str, control: AnyElement| {
        div()
            .flex()
            .flex_col()
            .gap(tokens.spacing.xs)
            .child(div().text_xs().text_color(tokens.text_muted).child(label))
            .child(control)
    };
    let saving = editor.saving.is_some();
    div()
        .debug_selector(|| "document-editor".into())
        .flex()
        .flex_col()
        .gap(tokens.spacing.md)
        .child(field(
            tr(locale, "document_title"),
            editor.title.clone().into_any_element(),
        ))
        .child(field(
            tr(locale, "document_tags"),
            editor.tags.clone().into_any_element(),
        ))
        .child(field(
            tr(locale, "document_status"),
            div()
                .debug_selector(|| "select-document-status".into())
                .w(STATUS_SELECT_WIDTH)
                .max_w_full()
                .child(editor.status.clone())
                .into_any_element(),
        ))
        .child(field(
            tr(locale, "document_summary"),
            editor.summary.clone().into_any_element(),
        ))
        .child(field(
            tr(locale, "document_body"),
            editor.body.clone().into_any_element(),
        ))
        .child(
            div().flex().items_center().gap_3().child(
                Button::new("save-document", tr(locale, "save_document"))
                    .debug_selector(|| "save-document".into())
                    .primary()
                    .compact()
                    .disabled(saving || root.documents_root_switch.switching)
                    .on_click(cx.listener(RootView::save_document_draft)),
            ),
        )
        .into_any_element()
}

fn workspace_read_error(
    root: &RootView,
    tokens: &Tokens,
    cx: &mut Context<RootView>,
) -> Option<AnyElement> {
    let error = root.projects_error.as_ref()?;
    let locale = root.ui.locale;
    let scroll = root.scroll.workspace_read_error.clone();
    Some(
        div()
            .debug_selector(|| "workspace-read-error".into())
            .flex_shrink_0()
            .min_w_0()
            .p_2()
            .rounded(tokens.radius.sm)
            .bg(tokens.warning_surface)
            .text_color(tokens.warning_text)
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .id("workspace-read-error-message")
                    .debug_selector(|| "workspace-read-error-message".into())
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .max_h(RECOVERY_MESSAGE_HEIGHT)
                    .overflow_y_scroll()
                    .track_scroll(&scroll)
                    .child(
                        div()
                            .debug_selector(|| "workspace-read-error-text".into())
                            .text_sm()
                            .child(i18n::message(locale, "workspace_read_failed", error)),
                    )
                    .child(Scrollbar::vertical(
                        "workspace-read-error-scrollbar",
                        &scroll,
                    )),
            )
            .child(
                Button::new("retry-workspace-read", tr(locale, "retry_workspace_read"))
                    .disabled(root.documents_root_switch.switching)
                    .debug_selector(|| "retry-workspace-read".into())
                    .secondary()
                    .compact()
                    .on_click(cx.listener(|this, _, _, cx| this.reload_projects(cx))),
            )
            .into_any_element(),
    )
}

fn model_guidance(
    hint: &'static str,
    settings_section: Option<&'static str>,
    locale: i18n::Locale,
    in_body: bool,
    tokens: &Tokens,
    cx: &mut Context<RootView>,
) -> gpui::Div {
    let (action, label) = if settings_section.is_some() {
        ("go-to-model-settings", "choose_model")
    } else {
        ("go-to-models", "go_to_models")
    };
    div()
        .debug_selector(|| "models-required".into())
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .items_start()
        .gap(tokens.spacing.sm)
        .text_sm()
        .when(!in_body, |row| {
            row.flex_row().flex_wrap().items_center().text_xs()
        })
        .child(
            div()
                .min_w_0()
                .max_w_full()
                .text_color(tokens.text_muted)
                .child(tr(locale, hint)),
        )
        .child(
            Button::new(action, tr(locale, label))
                .debug_selector(move || action.into())
                .compact()
                .when(in_body, |button| button.primary())
                .when(!in_body, |button| button.ghost())
                .on_click(cx.listener(move |this, _, _, cx| match settings_section {
                    Some(section) => this.open_settings_section(section, cx),
                    None => this.navigate(Route::Models, cx),
                })),
        )
}
