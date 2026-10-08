use crate::{
    components::{backend_ids, ordered_projects, status_label, timestamp, DragGhost, ProjectDrag},
    controller::RootView,
    controls::{tree, Button, RadioGroup, Scrollbar, Tree, TreeItem},
    i18n::{self, text as tr},
    speakers::{document_speakers, DocumentSpeaker},
    state::{self, ContentMode},
    theme::{self, Tokens},
};
use domain::{DocumentState, ProjectStatus};
use gpui::{div, prelude::*, px, AnyElement, Context, IntoElement, Pixels};

/// The tallest the issue list grows before it scrolls.
const ISSUE_LIST_HEIGHT: Pixels = px(168.);
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
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    main_panel(root, selected, cx).debug_selector(|| "meetings-page".into())
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
    .on_shift(cx.listener(|this, shift, _, cx| this.shift_project(*shift, cx)))
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
                .child(tr(root.ui.locale, "no_projects")),
        );
    } else {
        for (project, depth) in rows {
            let selected = root.ui.selected_project.as_ref() == Some(project.id());
            let display_status = if selected && root.ui.processing {
                ProjectStatus::Processing
            } else {
                project.processing_status()
            };
            let project_id = project.id().clone();
            let drag_id = project.id().clone();
            let drag_title = project.title().to_string();
            let target_id = project.id().clone();
            let has_children = root
                .projects
                .iter()
                .any(|child| child.hierarchy.parent_id.as_ref() == Some(project.id()));
            let expanded = root.ui.expanded_projects.contains(project.id());
            let drag_handle = div()
                .id(format!("drag-{}", project.id().as_str()))
                .debug_selector(|| format!("drag-{}", project.id().as_str()))
                .mr_2()
                .cursor_pointer()
                .text_color(tokens.text_muted)
                .child("⋮⋮")
                .on_drag(
                    ProjectDrag {
                        project_id: drag_id,
                        revision: project.revision(),
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
                    format!("expand-{}", project.id().as_str()),
                    expanded,
                    cx.listener(move |this, _, _, cx| this.toggle_project(expand_id.clone(), cx)),
                )
                .into_any_element()
            } else {
                tree::disclosure_space().into_any_element()
            };
            let before_target = project.id().clone();
            let after_target = project.id().clone();
            let before_drop = div()
                .id(format!("before-{}", project.id().as_str()))
                .debug_selector(|| format!("before-{}", project.id().as_str()))
                .h(px(5.))
                .on_drop(cx.listener(move |this, dragged: &ProjectDrag, _, cx| {
                    this.reorder_document(dragged.clone(), before_target.clone(), true, cx)
                }));
            let after_drop = div()
                .id(format!("after-{}", project.id().as_str()))
                .debug_selector(|| format!("after-{}", project.id().as_str()))
                .h(px(5.))
                .on_drop(cx.listener(move |this, dragged: &ProjectDrag, _, cx| {
                    this.reorder_document(dragged.clone(), after_target.clone(), false, cx)
                }));
            let row = TreeItem::new(
                project.id().as_str().to_string(),
                project.title().to_string(),
                &root.tree_focus,
            )
            .debug_selector(|| format!("project-row-{}", project.id().as_str()))
            .depth(depth.into())
            .selected(selected)
            .expanded(has_children.then_some(expanded))
            .cursor_pointer()
            .px(tokens.spacing.sm)
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
                    .flex()
                    .items_center()
                    .child(drag_handle)
                    .child(expand_handle)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .child(theme::tail_faded_label(
                                format!("project-title-{}", project.id().as_str()),
                                format!(
                                    "{} {}",
                                    if depth > 0 { "↳" } else { "◈" },
                                    project.title()
                                ),
                                if selected {
                                    tokens.selection
                                } else {
                                    tokens.sidebar
                                },
                            )),
                    ),
            )
            .child(status_line(
                status_label(root.ui.locale, display_status),
                root.audio.problem(project.id()).map(|problem| {
                    (
                        format!("audio-indicator-{}", project.id().as_str()),
                        tr(root.ui.locale, problem.label_key()),
                    )
                }),
                &tokens,
            ))
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
                .child(Scrollbar::vertical("project-tree-scrollbar", &scroll)),
        )
}

/// A tree row's processing status, and beside it the audio problem the
/// last check found, if any, as `(selector, label)`.
fn status_line(
    status: &'static str,
    audio_problem: Option<(String, &'static str)>,
    tokens: &Tokens,
) -> gpui::Div {
    let line = div().mt_1().text_xs();
    match audio_problem {
        None => line.text_color(tokens.text_muted).child(status),
        Some((selector, label)) => line
            .flex()
            .items_center()
            .gap(tokens.spacing.sm)
            .child(div().text_color(tokens.text_muted).child(status))
            .child(
                div()
                    .debug_selector(move || selector.clone())
                    .text_color(tokens.warning_text)
                    .child(label),
            ),
    }
}

fn main_panel(
    root: &mut RootView,
    project: Option<&DocumentState>,
    cx: &mut Context<RootView>,
) -> gpui::Div {
    let locale = root.ui.locale;
    let tokens = *theme::tokens(cx);
    let display_status = project.map(|project| {
        if root.ui.processing {
            ProjectStatus::Processing
        } else {
            project.processing_status()
        }
    });
    let mut panel = div()
        .flex_1()
        .min_w_0()
        .h_full()
        .p_6()
        .flex()
        .flex_col()
        .gap_4()
        .children(conflict_notice(root, &tokens, cx))
        .children(issues_panel(root, &tokens));
    if let Some(project) = project {
        let ready = project.processing_status() == ProjectStatus::Ready;
        let audio_problem = root.audio.problem(project.id());
        let process_label = tr(
            locale,
            if root.ui.processing {
                "cancel"
            } else {
                "process"
            },
        );
        let mode = root.ui.content_mode;
        let scroll = root.scroll.meeting_content.clone();
        let mut content = div()
            .id("meeting-content")
            .debug_selector(|| "meeting-content".into())
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .p_5()
            .rounded(tokens.radius.md)
            .bg(tokens.surface);
        content = match mode {
            ContentMode::Edit => content.child(editor_form(root, project, &tokens, cx)),
            ContentMode::Summary if ready => content.child(
                div().text_sm().child(
                    project
                        .content
                        .summary
                        .clone()
                        .unwrap_or_else(|| tr(locale, "no_summary").into()),
                ),
            ),
            ContentMode::Original if ready => content
                .child(speakers_panel(root, project, &tokens, cx))
                .children(project.results.speaker_segments.iter().map(|segment| {
                    div()
                        .pb_3()
                        .child(
                            div()
                                .text_xs()
                                .text_color(tokens.accent_text)
                                .child(format!(
                                    "{}  {}",
                                    timestamp(segment.start_ms),
                                    segment
                                        .speaker_label
                                        .as_deref()
                                        .unwrap_or(&segment.speaker_id)
                                )),
                        )
                        .child(div().text_sm().child(segment.text.clone()))
                })),
            _ => content.child(
                div()
                    .text_color(tokens.text_muted)
                    .child(tr(locale, "need_process")),
            ),
        };
        panel = panel
            .child(
                div()
                    .flex_shrink_0()
                    .text_2xl()
                    .child(project.title().to_string()),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_sm()
                    .text_color(tokens.text_muted)
                    .child(project.audio_path().to_string()),
            )
            // The status and actions wrap beside an open Detail rather than
            // run past the content column.
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .px_3()
                            .py_1()
                            .rounded(tokens.radius.sm)
                            .bg(tokens.status(display_status.unwrap_or(ProjectStatus::Imported)))
                            .text_sm()
                            .child(status_label(
                                locale,
                                display_status.unwrap_or(ProjectStatus::Imported),
                            )),
                    )
                    .children(audio_problem.map(|problem| {
                        div()
                            .debug_selector(|| "audio-problem".into())
                            .px_3()
                            .py_1()
                            .rounded(tokens.radius.sm)
                            .bg(tokens.warning_surface)
                            .text_color(tokens.warning_text)
                            .text_sm()
                            .child(tr(locale, problem.label_key()))
                    }))
                    .child(
                        Button::new("process-project", process_label)
                            .debug_selector(|| "process-project".into())
                            .primary()
                            .compact()
                            // Missing audio cannot be processed; relink it.
                            .disabled(!root.ui.processing && root.audio.missing(project.id()))
                            .on_click(cx.listener(RootView::process_or_cancel)),
                    )
                    .children(audio_problem.map(|_| {
                        Button::new("relink-audio", tr(locale, "relink_audio"))
                            .debug_selector(|| "relink-audio".into())
                            .secondary()
                            .compact()
                            .disabled(root.audio.relinking.is_some())
                            .on_click(cx.listener(RootView::relink_audio))
                    }))
                    .child(
                        Button::new("open-project-detail", tr(locale, "open_detail"))
                            .debug_selector(|| "open-project-detail".into())
                            .focus_handle(&root.detail_trigger)
                            .secondary()
                            .compact()
                            .on_click(cx.listener(RootView::open_detail)),
                    ),
            );
        // Summary and Edit always; the speaker-labelled original once
        // processed.
        let mut controls = RadioGroup::new("content-mode", tr(locale, "content_mode"))
            .debug_selector(|| "content-mode-group".into())
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_2()
            .option(
                mode == ContentMode::Summary,
                crate::components::mode_button(tr(locale, "summary"), ContentMode::Summary, cx),
            );
        if ready {
            controls = controls.option(
                mode == ContentMode::Original,
                crate::components::mode_button(tr(locale, "original"), ContentMode::Original, cx),
            );
        }
        controls = controls.option(
            mode == ContentMode::Edit,
            crate::components::mode_button(tr(locale, "edit"), ContentMode::Edit, cx),
        );
        panel = panel.child(controls);
        // The scrollbar stays outside the fade.
        panel = panel.child(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .child(
                    theme::scroll_faded(&scroll, tokens.surface, content).rounded(tokens.radius.md),
                )
                .child(Scrollbar::vertical("meeting-content-scrollbar", &scroll)),
        );
    } else {
        panel = panel
            .child(
                div()
                    .flex_shrink_0()
                    .text_2xl()
                    .child(tr(locale, "meetings")),
            )
            .child(crate::components::empty_card(
                tr(locale, "empty_meetings"),
                &tokens,
            ));
    }
    panel.child(
        div()
            .flex_shrink_0()
            .border_t_1()
            .border_color(tokens.hairline)
            .pt_3()
            .text_xs()
            .text_color(tokens.text_muted)
            .child(format!(
                "{}   •   ASR: {}   •   DIA: {}",
                root.ui.notice,
                backend_ids(&root.inventory.asr),
                backend_ids(&root.inventory.diarization)
            ))
            .child(format!(
                "{}: {}",
                tr(locale, "models"),
                state::model_status(&root.installation, locale)
            )),
    )
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
    let busy = root.speaker_command.is_some();
    // Built for this document during the same render.
    let rows = root
        .speaker_rows
        .as_ref()
        .filter(|rows| &rows.key().0 == project.id());
    let speakers = document_speakers(project);
    let mut panel = div()
        .debug_selector(|| "document-speakers".into())
        .mb(tokens.spacing.lg)
        .pb(tokens.spacing.md)
        .border_b_1()
        .border_color(tokens.hairline)
        .flex()
        .flex_col()
        .gap(tokens.spacing.sm)
        .child(div().text_sm().child(tr(locale, "speakers")))
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
                            speaker.name().to_string(),
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
    panel.into_any_element()
}

/// The quiet line under a speaker's name: its segment count, after its id
/// when a label is shown above.
fn speaker_facts(speaker: &DocumentSpeaker, locale: i18n::Locale) -> String {
    let segments = i18n::message(locale, "speaker_segments", &speaker.segments.to_string());
    match speaker.label {
        Some(_) => format!("{} · {segments}", speaker.id),
        None => segments,
    }
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
                    .flex_1()
                    .min_w_0()
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
            .child(
                Button::new("reload-conflicted-document", tr(locale, "reload"))
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

/// The scanner's issues of the documents directory, each with its path and
/// reason, as the last reload read them. Nothing when there are none.
fn issues_panel(root: &RootView, tokens: &Tokens) -> Option<AnyElement> {
    if root.document_issues.is_empty() {
        return None;
    }
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
                                .child(issue_sentence(issue, root.ui.locale)),
                        )
                }),
        );
    Some(
        div()
            .debug_selector(|| "document-issues".into())
            .flex_shrink_0()
            .p_3()
            .rounded(tokens.radius.sm)
            .bg(tokens.warning_surface)
            .text_color(tokens.warning_text)
            .flex()
            .flex_col()
            .gap(tokens.spacing.sm)
            .child(div().text_sm().child(i18n::message(
                root.ui.locale,
                "document_issues",
                &root.document_issues.len().to_string(),
            )))
            // The scrollbar stays outside the fade.
            .child(
                div()
                    .relative()
                    .child(theme::scroll_faded(&scroll, tokens.warning_surface, list))
                    .child(Scrollbar::vertical("document-issues-scrollbar", &scroll)),
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
                    .disabled(saving)
                    .on_click(cx.listener(RootView::save_document_draft)),
            ),
        )
        .into_any_element()
}
