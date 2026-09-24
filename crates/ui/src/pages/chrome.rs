use crate::{controller::RootView, i18n::text as tr, route::Route};
use gpui::{div, prelude::*, Context, IntoElement};
use gpui_component::button::{Button, ButtonVariants};

pub(crate) fn header(root: &mut RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    div()
        .h(gpui::px(64.))
        .px_6()
        .flex()
        .items_center()
        .justify_between()
        .flex_shrink_0()
        .border_b_1()
        .border_color(gpui::rgb(0x2b303b))
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .text_xl()
                        .font_weight(gpui::FontWeight::BOLD)
                        .child("meetnote"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(gpui::rgb(0x8992a3))
                        .child(tr(root.ui.locale, "local_intelligence")),
                ),
        )
        .child(
            div()
                .debug_selector(|| "nav".into())
                .flex()
                .items_center()
                .gap_2()
                .child(nav_button(
                    tr(root.ui.locale, "assistant"),
                    Route::Assistant,
                    cx,
                ))
                .child(nav_button(
                    tr(root.ui.locale, "meetings"),
                    Route::Meetings,
                    cx,
                ))
                .child(nav_button(tr(root.ui.locale, "models"), Route::Models, cx))
                .child(nav_button(
                    tr(root.ui.locale, "settings"),
                    Route::Settings,
                    cx,
                ))
                .child(
                    Button::new("import-audio")
                        .primary()
                        .label(tr(root.ui.locale, "import_audio"))
                        .on_click(cx.listener(RootView::import_audio)),
                ),
        )
}

fn nav_button(label: &'static str, route: Route, cx: &mut Context<RootView>) -> impl IntoElement {
    Button::new(format!("nav-{route:?}"))
        .debug_selector(move || format!("nav-{route:?}"))
        .ghost()
        .compact()
        .label(label)
        .on_click(cx.listener(move |this, _, _, cx| this.navigate(route, cx)))
}
