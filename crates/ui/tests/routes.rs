//! L2: the structure of every route in the desktop window. Each test opens
//! the window as `run` does on the shared fixture (`support`), reaches the
//! route through its navigation button, and checks by `debug_selector` that
//! the navigation, the route's page with its primary content, and one
//! primary control are laid out inside the 1180×760 window, and that no
//! other route's page is rendered. GPUI's test platform keeps the tests
//! deterministic.
//!
//! A `debug_selector` on a scrollable element marks its scroll content,
//! which may extend below the window; such content is only required to start
//! inside its page.

mod support;

use gpui::{Bounds, Modifiers, Pixels, TestAppContext, VisualTestContext};
use support::{DOCUMENT_IDS, WINDOW_HEIGHT, WINDOW_WIDTH};
use ui::test_support::{MainWindow, Route};

/// The page root of every route; each route renders only its own.
const ROUTE_PAGES: [(Route, &str); 4] = [
    (Route::Assistant, "assistant-page"),
    (Route::Meetings, "meetings-page"),
    (Route::Models, "models-page"),
    (Route::Settings, "settings-page"),
];

fn nav_selector(route: Route) -> &'static str {
    match route {
        Route::Assistant => "nav-Assistant",
        Route::Meetings => "nav-Meetings",
        Route::Models => "nav-Models",
        Route::Settings => "nav-Settings",
    }
}

/// Opens the fixture window and clicks through to `route`, by way of a
/// different route so every test exercises a real switch. Returns the
/// bounds of the route's page.
fn open_route(
    cx: &mut TestAppContext,
    route: Route,
) -> (MainWindow, &mut VisualTestContext, Bounds<Pixels>) {
    // The startup installation probe completes on a worker thread.
    cx.executor().allow_parking();
    let window = cx.update(|cx| {
        support::init(cx);
        support::open_main_window(cx)
    });
    let cx = VisualTestContext::from_window(window.window(), cx).into_mut();
    for _ in 0..500 {
        cx.run_until_parked();
        if cx.update(|_, cx| window.installation_loaded(cx)) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(
        cx.update(|_, cx| window.installation_loaded(cx)),
        "the installation probe did not finish"
    );
    assert_eq!(cx.update(|_, cx| window.route(cx)), Route::Assistant);

    let detour = if route == Route::Models {
        Route::Settings
    } else {
        Route::Models
    };
    for step in [detour, route] {
        let button = visible(cx, nav_selector(step));
        cx.simulate_click(button.center(), Modifiers::none());
        cx.run_until_parked();
        assert_eq!(cx.update(|_, cx| window.route(cx)), step);
    }
    let page = page_below_navigation(cx, route);
    (window, cx, page)
}

fn window_bounds() -> Bounds<Pixels> {
    Bounds::from_corners(
        gpui::point(gpui::px(0.), gpui::px(0.)),
        gpui::point(gpui::px(WINDOW_WIDTH), gpui::px(WINDOW_HEIGHT)),
    )
}

/// The bounds of `selector`, which must be non-empty.
#[track_caller]
fn rendered(cx: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("`{selector}` is not rendered"));
    assert!(
        bounds.size.width > gpui::px(0.) && bounds.size.height > gpui::px(0.),
        "`{selector}` is empty: {bounds:?}"
    );
    bounds
}

/// The bounds of `selector`, which must be non-empty and inside the window.
#[track_caller]
fn visible(cx: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
    let bounds = rendered(cx, selector);
    assert!(
        within(bounds, window_bounds()),
        "`{selector}` leaves the window: {bounds:?}"
    );
    bounds
}

/// Scroll content `selector`, which must start inside `page` and keep to
/// its width.
#[track_caller]
fn scroll_content(
    cx: &mut VisualTestContext,
    selector: &'static str,
    page: Bounds<Pixels>,
) -> Bounds<Pixels> {
    let bounds = rendered(cx, selector);
    assert!(
        bounds.left() >= page.left()
            && bounds.right() <= page.right()
            && bounds.top() >= page.top()
            && bounds.top() < page.bottom(),
        "`{selector}` does not start inside its page: {bounds:?} in {page:?}"
    );
    bounds
}

/// Whether `inner` lies in `outer`, edges included.
fn within(inner: Bounds<Pixels>, outer: Bounds<Pixels>) -> bool {
    inner.left() >= outer.left()
        && inner.top() >= outer.top()
        && inner.right() <= outer.right()
        && inner.bottom() <= outer.bottom()
}

/// The navigation holds every route button, and `route`'s page, the only
/// page rendered, fills the window below it.
#[track_caller]
fn page_below_navigation(cx: &mut VisualTestContext, route: Route) -> Bounds<Pixels> {
    let navigation = visible(cx, "nav");
    for (other, _) in ROUTE_PAGES {
        let button = visible(cx, nav_selector(other));
        assert!(
            within(button, navigation),
            "`{}` is outside the navigation",
            nav_selector(other)
        );
    }
    let mut page = None;
    for (other, selector) in ROUTE_PAGES {
        if other == route {
            let bounds = visible(cx, selector);
            assert!(
                navigation.bottom() <= bounds.top(),
                "the navigation overlaps the {route:?} page"
            );
            assert_eq!(
                bounds.bottom(),
                gpui::px(WINDOW_HEIGHT),
                "the {route:?} page ends at the window bottom"
            );
            page = Some(bounds);
        } else {
            assert!(
                cx.debug_bounds(selector).is_none(),
                "{route:?} still renders the {other:?} page"
            );
        }
    }
    page.expect("every route has a page")
}

#[gpui::test]
fn the_assistant_route_shows_the_prompt_and_send_button(cx: &mut TestAppContext) {
    let (_window, cx, page) = open_route(cx, Route::Assistant);
    let prompt = visible(cx, "assistant-input");
    let send = visible(cx, "assistant-submit");
    assert!(within(prompt, page) && within(send, page));
    assert!(prompt.bottom() <= send.top(), "send sits below the prompt");
}

#[gpui::test]
fn the_meetings_route_shows_the_tree_content_and_process_button(cx: &mut TestAppContext) {
    let (_window, cx, page) = open_route(cx, Route::Meetings);
    let tree = scroll_content(cx, "project-tree", page);
    let content = scroll_content(cx, "meeting-content", page);
    let process = visible(cx, "process-project");
    assert!(within(process, page));
    assert!(
        tree.right() <= content.left(),
        "the tree sits left of the content"
    );

    let [root, child, second_root] =
        DOCUMENT_IDS.map(|id| visible(cx, Box::leak(format!("project-row-{id}").into_boxed_str())));
    assert!(
        root.bottom() <= child.top() && child.bottom() <= second_root.top(),
        "the child row sits between its parent and the next root"
    );
    assert!(
        child.left() > root.left() && second_root.left() == root.left(),
        "only the child row is indented"
    );
}

#[gpui::test]
fn the_models_route_shows_the_catalog_and_a_family_toggle(cx: &mut TestAppContext) {
    let (_window, cx, page) = open_route(cx, Route::Models);
    let first_family = support::catalog()
        .models
        .into_iter()
        .map(|model| model.model_family)
        .min()
        .expect("the catalog lists models");
    let list = scroll_content(cx, "models-scroll", page);
    let toggle = visible(
        cx,
        Box::leak(format!("model-family-{first_family}").into_boxed_str()),
    );
    assert!(
        within(toggle, list) && within(toggle, page),
        "the first family toggle is inside the model list"
    );
}

#[gpui::test]
fn the_settings_route_shows_the_form_and_language_select(cx: &mut TestAppContext) {
    let (_window, cx, page) = open_route(cx, Route::Settings);
    let form = scroll_content(cx, "settings-scroll", page);
    let language = visible(cx, "select-language");
    assert!(
        within(language, form) && within(language, page),
        "the language select is inside the settings form"
    );
}
