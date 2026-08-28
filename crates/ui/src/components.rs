use crate::{controller::RootView, i18n, state::ContentMode};
use domain::{MeetingProject, ProjectId, ProjectStatus};
use gpui::{div, prelude::*, rgb, Context, Render, SharedString, Window};

#[derive(Clone)]
pub(crate) struct ProjectDrag {
    pub(crate) project_id: ProjectId,
}

pub(crate) struct DragGhost {
    pub(crate) title: String,
}

impl Render for DragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .p_3()
            .rounded_md()
            .bg(rgb(0x263142))
            .text_sm()
            .child(self.title.clone())
    }
}

pub(crate) fn empty_card(message: impl Into<SharedString>) -> impl IntoElement {
    div()
        .flex_1()
        .rounded_lg()
        .bg(rgb(0x171b22))
        .flex()
        .items_center()
        .justify_center()
        .p_6()
        .text_color(rgb(0x8992a3))
        .child(message.into())
}

pub(crate) fn settings_section(
    title: impl Into<SharedString>,
    rows: Vec<gpui::AnyElement>,
) -> impl IntoElement {
    div()
        .pb_4()
        .border_b_1()
        .border_color(rgb(0x2b303b))
        .child(div().text_lg().child(title.into()))
        .children(rows)
}

pub(crate) fn setting_control_row(
    label: impl Into<SharedString>,
    control: gpui::AnyElement,
) -> impl IntoElement {
    div()
        .mt_3()
        .py_2()
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .text_sm()
        .child(div().text_color(rgb(0x8992a3)).child(label.into()))
        .child(control)
}

pub(crate) fn mode_button(
    label: impl Into<SharedString>,
    mode: ContentMode,
    active: ContentMode,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let label = label.into();
    div()
        .id(format!("content-mode-{}", label))
        .cursor_pointer()
        .px_3()
        .py_1()
        .rounded_md()
        .bg(if mode == active {
            rgb(0x2563eb)
        } else {
            rgb(0x263142)
        })
        .child(label)
        .on_click(cx.listener(move |this, _, _, cx| this.set_content_mode(mode, cx)))
}

pub(crate) fn ordered_projects(
    projects: &[MeetingProject],
    expanded: &std::collections::BTreeSet<ProjectId>,
) -> Vec<(MeetingProject, u8)> {
    fn append(
        projects: &[MeetingProject],
        parent: Option<&ProjectId>,
        depth: u8,
        expanded: &std::collections::BTreeSet<ProjectId>,
        output: &mut Vec<(MeetingProject, u8)>,
    ) {
        let mut children: Vec<_> = projects
            .iter()
            .filter(|project| project.parent_id.as_ref() == parent)
            .cloned()
            .collect();
        children.sort_by_key(|project| project.sibling_order);
        for child in children {
            let id = child.id.clone();
            output.push((child, depth));
            if expanded.contains(&id) {
                append(
                    projects,
                    Some(&id),
                    depth.saturating_add(1),
                    expanded,
                    output,
                );
            }
        }
    }
    let mut output = Vec::new();
    append(projects, None, 0, expanded, &mut output);
    output
}

pub(crate) fn status_label(locale: i18n::Locale, status: ProjectStatus) -> &'static str {
    i18n::status(locale, status)
}

pub(crate) fn status_color(status: ProjectStatus) -> gpui::Rgba {
    match status {
        ProjectStatus::Imported => rgb(0x374151),
        ProjectStatus::Processing => rgb(0x92400e),
        ProjectStatus::Ready => rgb(0x166534),
        ProjectStatus::Failed => rgb(0x991b1b),
    }
}

pub(crate) fn timestamp(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    let total_minutes = seconds / 60;
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    let seconds = seconds % 60;
    format!(
        "{hours:02}:{minutes:02}:{seconds:02}.{:03}",
        milliseconds % 1_000
    )
}

pub(crate) fn backend_ids(descriptors: &[application::BackendDescriptor]) -> String {
    descriptors
        .iter()
        .map(|descriptor| descriptor.id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}
