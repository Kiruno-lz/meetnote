use crate::{controller::RootView, i18n::text as tr};
use gpui::{div, prelude::*, px, rgb, Context, IntoElement};
use gpui_component::button::{Button, ButtonVariants};

pub(crate) fn render(root: &mut RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let notice = if root.ui.notice.is_empty() {
        tr(root.ui.locale, "ask_notice").to_string()
    } else {
        root.ui.notice.clone()
    };
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_5()
        .p_8()
        .child(
            div()
                .text_3xl()
                .font_weight(gpui::FontWeight::BOLD)
                .child("meetnote"),
        )
        .child(
            div()
                .text_lg()
                .text_color(rgb(0x8992a3))
                .child(tr(root.ui.locale, "ask_library")),
        )
        .child(
            div()
                .id("assistant-input")
                .w(px(620.))
                .min_h(px(96.))
                .p_4()
                .rounded_lg()
                .border_1()
                .border_color(rgb(0x3b82f6))
                .bg(rgb(0x171b22))
                .cursor_pointer()
                .text_color(rgb(0x8992a3))
                .child(root.assistant_input.clone()),
        )
        .child(
            div()
                .flex()
                .gap_3()
                .child(
                    Button::new("assistant-submit")
                        .primary()
                        .label(tr(root.ui.locale, "send"))
                        .on_click(cx.listener(RootView::assistant_action)),
                )
                .child(action_button(tr(root.ui.locale, "organize_library")))
                .child(action_button(tr(root.ui.locale, "find_project")))
                .child(action_button(tr(root.ui.locale, "split_project"))),
        )
        .child(
            div()
                .mt_4()
                .text_xs()
                .text_color(rgb(0x667085))
                .child(notice),
        )
}

fn action_button(label: &'static str) -> impl IntoElement {
    Button::new(format!("assistant-action-{label}"))
        .secondary()
        .label(label)
}
