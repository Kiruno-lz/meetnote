use crate::{
    components::{
        backend_ids, ordered_projects, status_color, status_label, timestamp, DragGhost,
        ProjectDrag,
    },
    controller::RootView,
    i18n::text as tr,
    state::ContentMode,
};
use domain::{DocumentState, ProjectStatus};
use gpui::{div, prelude::*, px, rgb, Context, IntoElement};
use gpui_component::{
    button::{Button, ButtonVariants},
    scroll::ScrollableElement as _,
};

pub(crate) fn render(
    root: &mut RootView,
    selected: Option<&DocumentState>,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    div()
        .flex_1()
        .flex()
        .child(sidebar(root, cx))
        .child(main_panel(root, selected, cx))
}

fn sidebar(root: &mut RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let mut tree = div()
        .id("project-tree")
        .flex_1()
        .overflow_y_scrollbar()
        .flex()
        .flex_col()
        .gap_1();
    if root.projects.is_empty() {
        tree = tree.child(
            div()
                .p_3()
                .rounded_md()
                .bg(rgb(0x1b2029))
                .text_sm()
                .text_color(rgb(0x8992a3))
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
                .mr_2()
                .cursor_pointer()
                .text_color(rgb(0x667085))
                .child("⋮⋮")
                .on_drag(
                    ProjectDrag {
                        project_id: drag_id,
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
                .h(px(5.))
                .on_drop(cx.listener(move |this, dragged: &ProjectDrag, _, cx| {
                    this.reorder_project(
                        dragged.project_id.clone(),
                        before_target.clone(),
                        true,
                        cx,
                    )
                }));
            let after_drop = div()
                .id(format!("after-{}", project.id().as_str()))
                .h(px(5.))
                .on_drop(cx.listener(move |this, dragged: &ProjectDrag, _, cx| {
                    this.reorder_project(
                        dragged.project_id.clone(),
                        after_target.clone(),
                        false,
                        cx,
                    )
                }));
            let row = div()
                .id(project.id().as_str().to_string())
                .cursor_pointer()
                .p_3()
                .ml(px((depth as f32) * 16.))
                .rounded_md()
                .bg(if selected {
                    rgb(0x27344a)
                } else {
                    rgb(0x171b22)
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(drag_handle)
                        .child(expand_handle)
                        .child(div().text_sm().child(format!(
                            "{} {}",
                            if depth > 0 { "↳" } else { "◈" },
                            project.title()
                        ))),
                )
                .child(
                    div()
                        .mt_1()
                        .text_xs()
                        .text_color(rgb(0x8992a3))
                        .child(status_label(root.ui.locale, display_status)),
                )
                .on_click(
                    cx.listener(move |this, _, _, cx| this.select_project(project_id.clone(), cx)),
                )
                .on_drop(cx.listener(move |this, dragged: &ProjectDrag, _, cx| {
                    this.move_project(dragged.project_id.clone(), target_id.clone(), cx)
                }));
            tree = tree.child(div().child(before_drop).child(row).child(after_drop));
        }
    }
    div()
        .w(px(280.))
        .h_full()
        .p_4()
        .gap_3()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .border_r_1()
        .border_color(rgb(0x2b303b))
        .child(
            div()
                .text_sm()
                .text_color(rgb(0x8992a3))
                .child(tr(root.ui.locale, "audio_projects")),
        )
        .child(tree)
}

fn main_panel(
    root: &mut RootView,
    project: Option<&DocumentState>,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let locale = root.ui.locale;
    let display_status = project.map(|project| {
        if root.ui.processing {
            ProjectStatus::Processing
        } else {
            project.processing_status()
        }
    });
    let mut panel = div().flex_1().h_full().p_6().flex().flex_col().gap_4();
    if let Some(project) = project {
        let mut content = div()
            .id("meeting-content")
            .flex_1()
            .overflow_y_scrollbar()
            .p_5()
            .rounded_lg()
            .bg(rgb(0x171b22));
        if project.processing_status() == ProjectStatus::Ready {
            if root.ui.content_mode == ContentMode::Summary {
                content = content.child(
                    div().text_sm().child(
                        project
                            .content
                            .summary
                            .clone()
                            .unwrap_or_else(|| tr(locale, "no_summary").into()),
                    ),
                );
            } else {
                content =
                    content.children(project.results.speaker_segments.iter().map(|segment| {
                        div()
                            .pb_3()
                            .child(div().text_xs().text_color(rgb(0x60a5fa)).child(format!(
                            "{}  {}",
                            timestamp(segment.start_ms),
                            segment
                                .speaker_label
                                .as_deref()
                                .unwrap_or(&segment.speaker_id)
                        )))
                            .child(div().text_sm().child(segment.text.clone()))
                    }));
            }
        } else {
            content = content.child(
                div()
                    .text_color(rgb(0x8992a3))
                    .child(tr(locale, "need_process")),
            );
        }
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
                    .text_color(rgb(0x8992a3))
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
                            .rounded_md()
                            .bg(status_color(
                                display_status.unwrap_or(ProjectStatus::Imported),
                            ))
                            .text_sm()
                            .child(status_label(
                                locale,
                                display_status.unwrap_or(ProjectStatus::Imported),
                            )),
                    )
                    .child(
                        Button::new("process-project")
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
                            .secondary()
                            .compact()
                            .label(tr(locale, "open_detail"))
                            .on_click(cx.listener(RootView::open_detail)),
                    ),
            );
        if project.processing_status() == ProjectStatus::Ready {
            let has_second_speaker = project
                .results
                .speaker_segments
                .iter()
                .any(|segment| segment.speaker_id == "speaker-1");
            let mut controls = div().flex_shrink_0().flex().items_center().gap_2();
            controls = controls
                .child(crate::components::mode_button(
                    tr(locale, "summary"),
                    ContentMode::Summary,
                    root.ui.content_mode,
                    cx,
                ))
                .child(crate::components::mode_button(
                    tr(locale, "original"),
                    ContentMode::Original,
                    root.ui.content_mode,
                    cx,
                ));
            if has_second_speaker {
                controls = controls.child(
                    Button::new("merge-speaker")
                        .secondary()
                        .compact()
                        .label(tr(locale, "merge_speaker"))
                        .on_click(cx.listener(RootView::merge_speaker)),
                );
            }
            panel = panel.child(controls);
        }
        panel = panel.child(content);
    } else {
        panel = panel
            .child(
                div()
                    .flex_shrink_0()
                    .text_2xl()
                    .child(tr(locale, "meetings")),
            )
            .child(crate::components::empty_card(tr(locale, "empty_meetings")));
    }
    panel.child(
        div()
            .flex_shrink_0()
            .border_t_1()
            .border_color(rgb(0x2b303b))
            .pt_3()
            .text_xs()
            .text_color(rgb(0x8992a3))
            .child(format!(
                "{}   •   ASR: {}   •   DIA: {}",
                root.ui.notice,
                backend_ids(&root.inventory.asr),
                backend_ids(&root.inventory.diarization)
            ))
            .child(format!(
                "{}: {}",
                tr(locale, "models"),
                root.services.model_status(locale)
            )),
    )
}
