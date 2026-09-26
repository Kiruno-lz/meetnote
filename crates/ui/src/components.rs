use crate::{
    controller::RootView,
    i18n,
    state::ContentMode,
    theme::{self, Tokens},
};
use domain::{DocumentState, ProjectId, ProjectStatus};
use gpui::{div, prelude::*, px, Context, Pixels, Render, SharedString, Window};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::separator::Separator;

/// A tree row being dragged, with the revision the row showed when the drag
/// started; the move is checked against it.
#[derive(Clone)]
pub(crate) struct ProjectDrag {
    pub(crate) project_id: ProjectId,
    pub(crate) revision: u64,
}

pub(crate) struct DragGhost {
    pub(crate) title: String,
}

impl Render for DragGhost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme::tokens(cx);
        div()
            .p_3()
            .rounded(tokens.radius.sm)
            .bg(tokens.drag_ghost)
            .text_sm()
            .child(self.title.clone())
    }
}

pub(crate) fn empty_card(message: impl Into<SharedString>, tokens: &Tokens) -> impl IntoElement {
    div()
        .flex_1()
        .rounded(tokens.radius.md)
        .bg(tokens.surface)
        .flex()
        .items_center()
        .justify_center()
        .p_6()
        .text_color(tokens.text_muted)
        .child(message.into())
}

pub(crate) fn settings_section(
    title: impl Into<SharedString>,
    rows: Vec<gpui::AnyElement>,
) -> impl IntoElement {
    div()
        .pb_4()
        .flex_shrink_0()
        .child(div().mb_1().text_lg().child(title.into()))
        .children(rows)
        .child(div().mt_4().child(Separator::horizontal()))
}

/// The settings label column; wide enough for the longest label in either
/// locale, so every control starts at the same x.
pub(crate) const SETTING_LABEL_WIDTH: Pixels = px(200.);
/// The width of selects and text fields in a settings row.
pub(crate) const SETTING_CONTROL_WIDTH: Pixels = px(360.);

/// A settings row: the label in the fixed label column, the control after
/// it. Rows are at least one large control tall, so rows of selects, fields
/// and buttons share one height.
pub(crate) fn setting_control_row(
    label: impl Into<SharedString>,
    control: gpui::AnyElement,
    tokens: &Tokens,
) -> impl IntoElement {
    div()
        .mt(tokens.spacing.sm)
        .min_h(tokens.control_height.lg)
        .flex()
        .items_center()
        .gap(tokens.spacing.lg)
        .text_sm()
        .child(
            div()
                .w(SETTING_LABEL_WIDTH)
                .flex_shrink_0()
                .text_color(tokens.text_muted)
                .child(label.into()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .child(control),
        )
}

pub(crate) fn mode_button(
    label: impl Into<SharedString>,
    mode: ContentMode,
    active: ContentMode,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let label = label.into();
    let name = match mode {
        ContentMode::Summary => "summary",
        ContentMode::Original => "original",
        ContentMode::Edit => "edit",
    };
    let button = Button::new(format!("content-mode-{name}"))
        .debug_selector(move || format!("content-mode-{name}"))
        .compact()
        .label(label);
    let button = if mode == active {
        button.primary()
    } else {
        button.secondary()
    };
    button.on_click(cx.listener(move |this, _, _, cx| this.set_content_mode(mode, cx)))
}

pub(crate) fn ordered_projects(
    projects: &[DocumentState],
    expanded: &std::collections::BTreeSet<ProjectId>,
) -> Vec<(DocumentState, u8)> {
    fn append(
        projects: &[DocumentState],
        parent: Option<&ProjectId>,
        depth: u8,
        expanded: &std::collections::BTreeSet<ProjectId>,
        output: &mut Vec<(DocumentState, u8)>,
    ) {
        let mut children: Vec<_> = projects
            .iter()
            .filter(|project| project.hierarchy.parent_id.as_ref() == parent)
            .cloned()
            .collect();
        children.sort_by_key(|project| project.hierarchy.sibling_order);
        for child in children {
            let id = child.id().clone();
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
