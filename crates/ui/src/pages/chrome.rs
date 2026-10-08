// Portions adapted from Zeron (https://github.com/zeronsh/zeron), Copyright (c) 2026 Wing, MIT License.

//! Fixed icon rail outside one rounded frame. The secondary sidebar and
//! content meet at a straight seam inside that frame.

use crate::{
    controller::RootView,
    controls::{sidebar::SidebarPanel, window::DragStrip, Button, NavEntry, NavList, Toast},
    i18n::{text as tr, Locale},
    route::Route,
    state, theme,
};
use gpui::{
    div, point, prelude::*, px, size, AnyElement, Context, FocusHandle, IntoElement, Pixels, Point,
    Size, TitlebarOptions, Window,
};

pub(crate) const WINDOW_TITLE: &str = "MeetNote";
pub(crate) const TITLEBAR_HEIGHT: Pixels = px(46.);
const TRAFFIC_LIGHT_POSITION: Point<Pixels> = point(px(14.), px(16.));
const RAIL_WIDTH: Pixels = px(64.);
pub(crate) const MIN_WINDOW_SIZE: Size<Pixels> = size(px(900.), px(600.));

pub(crate) const ENTRIES: [(Route, &str); 4] = [
    (Route::Assistant, "assistant"),
    (Route::Meetings, "meetings"),
    (Route::Models, "models"),
    (Route::Settings, "settings"),
];

pub(crate) fn titlebar_options() -> TitlebarOptions {
    TitlebarOptions {
        title: Some(WINDOW_TITLE.into()),
        appears_transparent: true,
        traffic_light_position: Some(TRAFFIC_LIGHT_POSITION),
    }
}

pub(crate) fn shell(
    root: &mut RootView,
    body: AnyElement,
    detail: Option<AnyElement>,
    window: &mut Window,
    cx: &mut Context<RootView>,
) -> gpui::Div {
    let tokens = *theme::tokens(cx);
    let secondary = matches!(root.ui.route, Route::Meetings | Route::Settings);
    let now = cx.background_executor().now();
    let folding = root.sidebar_sizing.read(cx).folding(now);
    let show_sidebar = secondary && (root.ui.sidebar_open || folding);
    if folding && secondary {
        window.request_animation_frame();
    }
    let toast = root
        .ui
        .toast()
        .cloned()
        .map(|toast| notice_toast(toast, root.ui.locale, cx));
    let plane = div()
        .debug_selector(|| "content-plane".into())
        .relative()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .h_full()
        .flex()
        .overflow_hidden()
        .bg(tokens.content)
        .map(|plane| {
            if show_sidebar {
                plane.rounded_r(tokens.radius.xl)
            } else {
                plane.rounded(tokens.radius.xl)
            }
        })
        .child(div().flex_1().min_w_0().h_full().flex().child(body))
        .children(detail)
        .children(toast);
    let mut frame = div()
        .debug_selector(|| "inner-frame".into())
        .flex_1()
        .min_w_0()
        .min_h_0()
        .mr(tokens.spacing.sm)
        .mb(tokens.spacing.sm)
        .flex()
        .rounded(tokens.radius.xl)
        .overflow_hidden()
        .border_1()
        .border_color(tokens.hairline);
    if show_sidebar {
        let id = if root.ui.route == Route::Meetings {
            "project-sidebar"
        } else {
            "settings-sidebar"
        };
        let content = secondary_sidebar(root, cx);
        frame = frame.child(SidebarPanel::new(
            id,
            tr(root.ui.locale, "resize_sidebar"),
            &root.sidebar_sizing,
            window.viewport_size().width - RAIL_WIDTH - tokens.spacing.sm,
            root.ui.sidebar_open,
            content,
        ));
    }
    frame = frame.child(plane);
    div()
        .size_full()
        .flex()
        .flex_col()
        .bg(tokens.window)
        .text_color(tokens.text)
        .child(toolbar(root, secondary, cx))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .child(rail(root, cx))
                .child(frame),
        )
}

fn toolbar(root: &mut RootView, secondary: bool, cx: &mut Context<RootView>) -> impl IntoElement {
    let locale = root.ui.locale;
    div()
        .debug_selector(|| "window-toolbar".into())
        .h(TITLEBAR_HEIGHT)
        .flex_shrink_0()
        .flex()
        .items_center()
        .child(drag_strip("titlebar-drag-sidebar").w(px(98.)))
        .child(
            Button::new("navigate-back", tr(locale, "navigate_back"))
                .debug_selector(|| "navigate-back".into())
                .ghost()
                .compact()
                .icon("icons/arrow-left.svg")
                .disabled(!root.ui.can_go_back())
                .on_click(cx.listener(|this, _, _, cx| this.travel(false, cx))),
        )
        .child(
            Button::new("navigate-forward", tr(locale, "navigate_forward"))
                .debug_selector(|| "navigate-forward".into())
                .ghost()
                .compact()
                .icon("icons/arrow-right.svg")
                .disabled(!root.ui.can_go_forward())
                .on_click(cx.listener(|this, _, _, cx| this.travel(true, cx))),
        )
        .child(
            Button::new("toggle-sidebar", tr(locale, "toggle_sidebar"))
                .debug_selector(|| "toggle-sidebar".into())
                .ghost()
                .compact()
                .icon("icons/panel-left.svg")
                .disabled(!secondary)
                .on_click(cx.listener(|this, _, window, cx| this.toggle_sidebar(window, cx))),
        )
        .child(drag_strip("titlebar-drag-content").flex_1())
}

fn rail(root: &mut RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let tokens = *theme::tokens(cx);
    let locale = root.ui.locale;
    let current = root.ui.route;
    let focus = root.nav_focus.clone();
    let entries = NavList::new("nav", tr(locale, "navigation"))
        .debug_selector(|| "nav".into())
        .flex()
        .flex_col()
        .items_center()
        .gap(px(8.))
        .children(ENTRIES.iter().enumerate().map(|(index, (route, key))| {
            nav_entry(
                tr(locale, key),
                *route,
                *route == current,
                &focus[index],
                (
                    index.checked_sub(1).map(|previous| &focus[previous]),
                    focus.get(index + 1),
                ),
                cx,
            )
        }));
    div()
        .debug_selector(|| "navigation-rail".into())
        .w(RAIL_WIDTH)
        .h_full()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .items_center()
        .gap(tokens.spacing.md)
        .child(entries)
        .child(div().flex_1())
        .child(
            Button::new("import-audio", tr(locale, "import_audio"))
                .debug_selector(|| "import-audio".into())
                .ghost()
                .icon("icons/plus.svg")
                .w(px(44.))
                .h(px(44.))
                .on_click(cx.listener(RootView::import_audio)),
        )
        .child(div().h(tokens.spacing.sm))
}

/// The latest notice's toast, centered at the top of the content plane,
/// clear of the Assistant's input bar at its bottom.
fn notice_toast(toast: state::Toast, locale: Locale, cx: &mut Context<RootView>) -> gpui::Div {
    let tokens = *theme::tokens(cx);
    let generation = toast.generation;
    let mut card = Toast::new(
        ("notice-toast", generation),
        state::toast_text(&toast.notice).to_string(),
        toast.tone,
    )
    .leaving(toast.leaving);
    if !toast.tone.fades() {
        card = card.on_close(
            tr(locale, "dismiss_notice"),
            cx.listener(move |this, _, _, cx| this.dismiss_toast(generation, cx)),
        );
    }
    div()
        .absolute()
        .top(tokens.spacing.md)
        .left_0()
        .right_0()
        .px(tokens.spacing.lg)
        .flex()
        .justify_center()
        .child(card)
}

fn secondary_sidebar(root: &mut RootView, cx: &mut Context<RootView>) -> AnyElement {
    let tokens = *theme::tokens(cx);
    let locale = root.ui.locale;
    let mut column = div().size_full().min_h_0().flex().flex_col().child(
        div()
            .p(tokens.spacing.lg)
            .text_lg()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .child(tr(
                locale,
                if root.ui.route == Route::Meetings {
                    "meetings"
                } else {
                    "settings"
                },
            )),
    );
    if root.ui.route == Route::Meetings {
        column = column.child(super::meetings::project_tree(root, cx));
    } else {
        column = column.child(
            div()
                .px(tokens.spacing.sm)
                .flex()
                .flex_col()
                .gap(px(4.))
                .children(
                    super::settings::SECTIONS
                        .iter()
                        .enumerate()
                        .map(|(index, key)| {
                            Button::new(format!("settings-section-{index}"), tr(locale, key))
                                .debug_selector(move || format!("settings-section-{index}"))
                                .ghost()
                                .leading_icon(super::settings::SECTION_ICONS[index])
                                .h(px(38.))
                                .w_full()
                                .when(index == root.ui.settings_section, |button| {
                                    button.bg(tokens.selection)
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.ui.settings_section = index;
                                    this.scroll.settings.scroll_to_top_of_item(index);
                                    cx.notify();
                                }))
                        }),
                ),
        );
    }
    column.into_any_element()
}

fn nav_entry(
    label: &'static str,
    route: Route,
    active: bool,
    focus: &FocusHandle,
    beside: (Option<&FocusHandle>, Option<&FocusHandle>),
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let path = match route {
        Route::Assistant => "icons/bot.svg",
        Route::Meetings => "icons/folder.svg",
        Route::Models => "icons/cpu.svg",
        Route::Settings => "icons/settings.svg",
    };
    let tokens = *theme::tokens(cx);
    let mut icon = div().child(gpui::svg().path(path).size(px(22.)).text_color(if active {
        tokens.text
    } else {
        tokens.text_muted
    }));
    if active {
        icon = icon.debug_selector(move || format!("nav-{route:?}-active"));
    }
    NavEntry::new(format!("nav-{route:?}"), label, focus)
        .debug_selector(move || format!("nav-{route:?}"))
        .icon_only()
        .selected(active)
        .beside(beside.0, beside.1)
        .on_click(cx.listener(move |this, _, _, cx| this.navigate(route, cx)))
        .child(icon)
}

fn drag_strip(id: &'static str) -> DragStrip {
    DragStrip::titlebar(id)
        .debug_selector(move || id.into())
        .h(TITLEBAR_HEIGHT)
        .flex_shrink_0()
}

#[cfg(test)]
mod tests {
    use gpui::AssetSource;

    #[test]
    fn every_shell_and_settings_icon_is_bundled() {
        for source in [include_str!("chrome.rs"), include_str!("settings.rs")] {
            for path in source
                .split('"')
                .filter(|path| path.starts_with("icons/") && path.ends_with(".svg"))
            {
                let asset = gpui_component_assets::Assets
                    .load(path)
                    .unwrap_or_else(|error| panic!("{path}: {error}"));
                assert!(
                    asset.is_some_and(|asset| !asset.is_empty()),
                    "missing icon: {path}"
                );
            }
        }
    }
}
