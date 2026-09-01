use crate::{
    components::{empty_card, timestamp},
    controller::RootView,
    i18n::{text as tr, Locale},
};
use domain::DocumentState;
use gpui::{div, prelude::*, px, rgb, Context, IntoElement};
use gpui_component::{
    button::{Button, ButtonVariants},
    scroll::ScrollableElement as _,
};

pub(crate) fn render(
    project: Option<&DocumentState>,
    locale: Locale,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let content = project
        .map(|project| {
            let transcript = div()
                .id("detail-transcript")
                .flex_1()
                .overflow_y_scrollbar()
                .children(project.results.speaker_segments.iter().map(|segment| {
                    div()
                        .pb_3()
                        .child(format!(
                            "{}  {}",
                            timestamp(segment.start_ms),
                            segment.speaker_id
                        ))
                        .child(div().text_sm().child(segment.text.clone()))
                }));
            div()
                .child(div().text_xl().child(project.title().to_string()))
                .child(
                    div()
                        .mt_2()
                        .text_sm()
                        .text_color(rgb(0x8992a3))
                        .child(format!("UUID: {}", project.id().as_str())),
                )
                .child(
                    div()
                        .mt_2()
                        .text_sm()
                        .text_color(rgb(0x8992a3))
                        .child(project.audio_path().to_string()),
                )
                .child(transcript)
                .into_any_element()
        })
        .unwrap_or_else(|| empty_card(tr(locale, "select_project_first")).into_any_element());
    div()
        .absolute()
        .top(px(76.))
        .right(px(24.))
        .bottom(px(24.))
        .w(px(440.))
        .p_5()
        .rounded_lg()
        .border_1()
        .border_color(rgb(0x3b82f6))
        .bg(rgb(0x202733))
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(0x60a5fa))
                        .child(tr(locale, "project_detail")),
                )
                .child(
                    Button::new("close-detail")
                        .ghost()
                        .compact()
                        .label("×")
                        .on_click(cx.listener(RootView::close_detail)),
                ),
        )
        .child(content)
}
