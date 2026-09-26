// Portions adapted from Zeron (https://github.com/zeronsh/zeron), Copyright (c) 2026 Wing, MIT License.

//! The window shell: a transparent titlebar over the window chrome, the
//! sidebar with the route entries, and the opaque content plane beside it.
//!
//! The chrome (titlebar strips and sidebar) is one `tokens.window` fill over
//! the whole window: translucent over the blurred desktop under glass,
//! opaque otherwise. It is painted once, so its alpha is exactly the one
//! `theme` computed for legible chrome text. The content plane is an opaque
//! card inset from the window edges and shows only the current route's page,
//! with Detail as a panel at its right edge.

use crate::{
    controller::RootView,
    controls::{window::DragStrip, Button, NavEntry, NavList},
    i18n::text as tr,
    route::Route,
    theme,
};
use gpui::{
    div, point, prelude::*, px, size, AnyElement, Context, FocusHandle, IntoElement, Pixels, Point,
    Size, TitlebarOptions,
};

/// The window title. The transparent titlebar hides it; the Window menu,
/// Mission Control and assistive technology read it.
pub(crate) const WINDOW_TITLE: &str = "MeetNote";
/// Height of the drag strips that start the sidebar and the content column.
pub(crate) const TITLEBAR_HEIGHT: Pixels = px(38.);
/// The native traffic lights are 14px tall; top 12 centers them in the strip.
const TRAFFIC_LIGHT_POSITION: Point<Pixels> = point(px(14.), px(12.));
/// Wide enough for a three-level project tree row.
pub(crate) const SIDEBAR_WIDTH: Pixels = px(256.);
/// The sidebar beside a content plane that still fits a page and Detail.
pub(crate) const MIN_WINDOW_SIZE: Size<Pixels> = size(px(900.), px(600.));

/// The route entries in sidebar order, with their label keys.
pub(crate) const ENTRIES: [(Route, &str); 4] = [
    (Route::Assistant, "assistant"),
    (Route::Meetings, "meetings"),
    (Route::Models, "models"),
    (Route::Settings, "settings"),
];

/// The main window's titlebar: content extends under it, and the traffic
/// lights sit in the sidebar's drag strip.
pub(crate) fn titlebar_options() -> TitlebarOptions {
    TitlebarOptions {
        title: Some(WINDOW_TITLE.into()),
        appears_transparent: true,
        traffic_light_position: Some(TRAFFIC_LIGHT_POSITION),
    }
}

/// The sidebar beside the content plane, which holds `body` and, when open,
/// `detail`.
pub(crate) fn shell(
    root: &mut RootView,
    body: AnyElement,
    detail: Option<AnyElement>,
    cx: &mut Context<RootView>,
) -> gpui::Div {
    let tokens = *theme::tokens(cx);
    let plane = div()
        .debug_selector(|| "content-plane".into())
        .flex_1()
        .min_h_0()
        .mr(tokens.spacing.sm)
        .mb(tokens.spacing.sm)
        .flex()
        .overflow_hidden()
        .rounded(tokens.radius.lg)
        .border_1()
        .border_color(tokens.hairline)
        .bg(tokens.content)
        .child(div().flex_1().min_w_0().h_full().flex().child(body))
        .children(detail);
    div()
        .size_full()
        .flex()
        .bg(tokens.window)
        .text_color(tokens.text)
        .child(sidebar(root, cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .child(drag_strip("titlebar-drag-content").w_full())
                .child(plane),
        )
}

fn sidebar(root: &mut RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let tokens = *theme::tokens(cx);
    let locale = root.ui.locale;
    let current = root.ui.route;
    let focus = root.nav_focus.clone();
    let entries = NavList::new("nav", tr(locale, "navigation"))
        .debug_selector(|| "nav".into())
        .flex()
        .flex_col()
        .gap(px(2.))
        .children(ENTRIES.iter().enumerate().map(|(index, (route, key))| {
            let beside = (
                index.checked_sub(1).map(|previous| &focus[previous]),
                focus.get(index + 1),
            );
            nav_entry(
                tr(locale, key),
                *route,
                *route == current,
                &focus[index],
                beside,
                cx,
            )
        }));
    let mut column = div()
        .debug_selector(|| "sidebar".into())
        .w(SIDEBAR_WIDTH)
        .h_full()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .child(drag_strip("titlebar-drag-sidebar").w_full())
        .child(
            div()
                .flex_shrink_0()
                .px(tokens.spacing.md)
                .pb(tokens.spacing.md)
                .flex()
                .flex_col()
                .gap(tokens.spacing.md)
                .child(entries)
                .child(
                    Button::new("import-audio", tr(locale, "import_audio"))
                        .debug_selector(|| "import-audio".into())
                        .primary()
                        .w_full()
                        .on_click(cx.listener(RootView::import_audio)),
                ),
        );
    if current == Route::Meetings {
        column = column.child(super::meetings::project_tree(root, cx));
    }
    column
}

/// A route entry; the current route's entry is filled with the selection
/// wash, set in a heavier weight and marks its label `nav-<Route>-active`.
/// Up and Down move focus to the entries `beside` it.
fn nav_entry(
    label: &'static str,
    route: Route,
    active: bool,
    focus: &FocusHandle,
    beside: (Option<&FocusHandle>, Option<&FocusHandle>),
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let mut text = div().child(label);
    if active {
        text = text.debug_selector(move || format!("nav-{route:?}-active"));
    }
    NavEntry::new(format!("nav-{route:?}"), label, focus)
        .debug_selector(move || format!("nav-{route:?}"))
        .selected(active)
        .beside(beside.0, beside.1)
        .on_click(cx.listener(move |this, _, _, cx| this.navigate(route, cx)))
        .child(text)
}

/// A titlebar strip that drags the window (`DragStrip`).
fn drag_strip(id: &'static str) -> DragStrip {
    DragStrip::titlebar(id)
        .debug_selector(move || id.into())
        .h(TITLEBAR_HEIGHT)
        .flex_shrink_0()
}
