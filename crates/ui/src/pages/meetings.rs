use crate::{
    components::{backend_ids, ordered_projects, status_label, timestamp, DragGhost, ProjectDrag},
    controller::RootView,
    i18n::{self, text as tr},
    state::{self, ContentMode},
    theme::{self, Tokens},
};
use domain::{DocumentState, ProjectStatus};
use gpui::{div, prelude::*, px, AnyElement, Context, IntoElement, Pixels};
use gpui_component::{
    button::{Button, ButtonVariants},
    scroll::ScrollableElement as _,
    Disableable,
};

/// The tallest the issue list grows before it scrolls.
const ISSUE_LIST_HEIGHT: Pixels = px(168.);
/// The width of the document status select.
const STATUS_SELECT_WIDTH: Pixels = px(240.);

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
/// between rows reorders.
pub(crate) fn project_tree(root: &mut RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let tokens = *theme::tokens(cx);
    let scroll = root.scroll.project_tree.clone();
    let mut tree = div()
        .id("project-tree")
        .debug_selector(|| "project-tree".into())
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
        for (project, depth) in ordered_projects(&root.projects, &root.ui.expanded_projects) {
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
            let expand_handle =
                if has_children {
                    let expand_id = project.id().clone();
                    div()
                        .id(format!("expand-{}", project.id().as_str()))
                        .cursor_pointer()
                        .mr_1()
                        .child(if root.ui.expanded_projects.contains(project.id()) {
                            "▾"
                        } else {
                            "▸"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_project(expand_id.clone(), cx)
                        }))
                        .into_any_element()
                } else {
                    div().w(px(10.)).into_any_element()
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
            let row =
                div()
                    .id(project.id().as_str().to_string())
                    .debug_selector(|| format!("project-row-{}", project.id().as_str()))
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
                            .child(div().flex_1().min_w_0().text_sm().child(
                                theme::tail_faded_label(
                                    format!("project-title-{}", project.id().as_str()),
                                    format!(
                                        "{} {}",
                                        if depth > 0 { "↳" } else { "◈" },
                                        project.title()
                                    ),
                                    if selected {
                                        tokens.selection
                                    } else {
                                        tokens.window
                                    },
                                ),
                            )),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_xs()
                            .text_color(tokens.text_muted)
                            .child(status_label(root.ui.locale, display_status)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_project(project_id.clone(), cx)
                    }))
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
                .child(theme::scroll_faded(&scroll, tokens.window, tree))
                .vertical_scrollbar(&scroll),
        )
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
            ContentMode::Original if ready => {
                content.children(project.results.speaker_segments.iter().map(|segment| {
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
                }))
            }
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
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
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
                    .child(
                        Button::new("process-project")
                            .debug_selector(|| "process-project".into())
                            .primary()
                            .compact()
                            .label(if root.ui.processing {
                                tr(locale, "cancel")
                            } else {
                                tr(locale, "process")
                            })
                            .on_click(cx.listener(RootView::process_or_cancel)),
                    )
                    .child(
                        Button::new("open-project-detail")
                            .debug_selector(|| "open-project-detail".into())
                            .secondary()
                            .compact()
                            .label(tr(locale, "open_detail"))
                            .on_click(cx.listener(RootView::open_detail)),
                    ),
            );
        // Summary and Edit always; the speaker-labelled original once
        // processed.
        let mut controls = div().flex_shrink_0().flex().items_center().gap_2().child(
            crate::components::mode_button(tr(locale, "summary"), ContentMode::Summary, mode, cx),
        );
        if ready {
            controls = controls.child(crate::components::mode_button(
                tr(locale, "original"),
                ContentMode::Original,
                mode,
                cx,
            ));
        }
        controls = controls.child(crate::components::mode_button(
            tr(locale, "edit"),
            ContentMode::Edit,
            mode,
            cx,
        ));
        let has_second_speaker = project
            .results
            .speaker_segments
            .iter()
            .any(|segment| segment.speaker_id == "speaker-1");
        if ready && has_second_speaker {
            controls = controls.child(
                Button::new("merge-speaker")
                    .secondary()
                    .compact()
                    .label(tr(locale, "merge_speaker"))
                    .on_click(cx.listener(RootView::merge_speaker)),
            );
        }
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
                .vertical_scrollbar(&scroll),
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
                Button::new("reload-conflicted-document")
                    .debug_selector(|| "reload-conflicted-document".into())
                    .secondary()
                    .compact()
                    .label(tr(locale, "reload"))
                    .on_click(cx.listener(RootView::reload_conflicted_document)),
            )
            .into_any_element(),
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
                                .child(issue.message.clone()),
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
                    .vertical_scrollbar(&scroll),
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
                Button::new("save-document")
                    .debug_selector(|| "save-document".into())
                    .primary()
                    .compact()
                    .label(tr(locale, "save_document"))
                    .disabled(saving)
                    .on_click(cx.listener(RootView::save_document_draft)),
            ),
        )
        .into_any_element()
}
