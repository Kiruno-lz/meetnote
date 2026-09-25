use crate::{
    components::{empty_card, timestamp},
    controller::RootView,
    i18n::{text as tr, Locale},
    theme,
};
use domain::DocumentState;
use gpui::{div, prelude::*, px, Context, IntoElement, Pixels};
use gpui_component::{
    button::{Button, ButtonVariants},
    scroll::ScrollableElement as _,
};

const DETAIL_WIDTH: Pixels = px(400.);

/// Detail: the selected document's facts in an opaque panel at the right of
/// the content plane, beside the document rather than over it. Opening or
/// closing it changes nothing but its own visibility.
pub(crate) fn render(
    project: Option<&DocumentState>,
    locale: Locale,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let tokens = *theme::tokens(cx);
    let content = project
        .map(|project| {
            let transcript = div()
                .id("detail-transcript")
                .mt_4()
                .flex_1()
                .min_h_0()
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
                .mt_3()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(div().text_xl().child(project.title().to_string()))
                .child(
                    div()
                        .mt_2()
                        .text_sm()
                        .text_color(tokens.text_muted)
                        .child(format!("UUID: {}", project.id().as_str())),
                )
                .child(
                    div()
                        .mt_2()
                        .text_sm()
                        .text_color(tokens.text_muted)
                        .child(project.audio_path().to_string()),
                )
                .child(transcript)
                .into_any_element()
        })
        .unwrap_or_else(|| {
            empty_card(tr(locale, "select_project_first"), &tokens).into_any_element()
        });
    // The panel's right corners follow the content plane's.
    div()
        .debug_selector(|| "detail-panel".into())
        .w(DETAIL_WIDTH)
        .h_full()
        .flex_shrink_0()
        .p_5()
        .rounded_tr(tokens.radius.lg)
        .rounded_br(tokens.radius.lg)
        .border_l_1()
        .border_color(tokens.hairline)
        .bg(tokens.raised)
        .flex()
        .flex_col()
        .child(
            div()
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .text_color(tokens.accent_text)
                        .child(tr(locale, "project_detail")),
                )
                .child(
                    Button::new("close-detail")
                        .debug_selector(|| "close-detail".into())
                        .ghost()
                        .compact()
                        .label("×")
                        .on_click(cx.listener(RootView::close_detail)),
                ),
        )
        .child(content)
}
