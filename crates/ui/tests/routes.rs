//! L2: the structure of every route in the desktop window. Each test opens
//! the window as `run` does on the shared fixture (`support`), reaches the
//! route through its sidebar entry, and checks by `debug_selector` that the
//! glass sidebar holds the entries, the opaque content plane beside it holds
//! only the route's page with its primary content, and that no other route's
//! page is rendered. GPUI's test platform keeps the tests deterministic.
//!
//! A `debug_selector` on a scrollable element marks its scroll content,
//! which may extend below the window; such content is only required to start
//! inside its container.

mod support;

use gpui::{Bounds, Modifiers, Pixels, TestAppContext, VisualTestContext};
use support::{DOCUMENT_IDS, WINDOW_HEIGHT, WINDOW_WIDTH};
use ui::test_support::{MainWindow, Overlay, Route};

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

/// Rendered inside the sidebar entry of the current route only.
fn active_selector(route: Route) -> &'static str {
    match route {
        Route::Assistant => "nav-Assistant-active",
        Route::Meetings => "nav-Meetings-active",
        Route::Models => "nav-Models-active",
        Route::Settings => "nav-Settings-active",
    }
}

/// Opens the fixture window on its start route, Assistant, once the startup
/// installation probe and project list have reached the view.
fn open_window(cx: &mut TestAppContext) -> (MainWindow, &mut VisualTestContext) {
    // The startup loads complete on worker threads.
    cx.executor().allow_parking();
    let window = cx.update(|cx| {
        support::init(cx);
        support::open_main_window(cx)
    });
    let cx = VisualTestContext::from_window(window.window(), cx).into_mut();
    for _ in 0..500 {
        cx.run_until_parked();
        if cx.update(|_, cx| window.startup_loaded(cx)) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(
        cx.update(|_, cx| window.startup_loaded(cx)),
        "the startup loads did not finish"
    );
    assert_eq!(cx.update(|_, cx| window.route(cx)), Route::Assistant);
    (window, cx)
}

/// Clicks the sidebar entry of `route`.
fn click_entry(cx: &mut VisualTestContext, window: &MainWindow, route: Route) {
    let entry = visible(cx, nav_selector(route));
    cx.simulate_click(entry.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| window.route(cx)), route);
}

/// Opens the fixture window and clicks through to `route`, by way of a
/// different route so every test exercises a real switch. Returns the
/// bounds of the route's page.
fn open_route(
    cx: &mut TestAppContext,
    route: Route,
) -> (MainWindow, &mut VisualTestContext, Bounds<Pixels>) {
    let (window, cx) = open_window(cx);
    let detour = if route == Route::Models {
        Route::Settings
    } else {
        Route::Models
    };
    for step in [detour, route] {
        click_entry(cx, &window, step);
    }
    let page = page_in_content_plane(cx, route);
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

/// Scroll content `selector`, which must start inside `container` and keep
/// to its width.
#[track_caller]
fn scroll_content(
    cx: &mut VisualTestContext,
    selector: &'static str,
    container: Bounds<Pixels>,
) -> Bounds<Pixels> {
    let bounds = rendered(cx, selector);
    assert!(
        bounds.left() >= container.left()
            && bounds.right() <= container.right()
            && bounds.top() >= container.top()
            && bounds.top() < container.bottom(),
        "`{selector}` does not start inside its container: {bounds:?} in {container:?}"
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

fn leak(selector: String) -> &'static str {
    Box::leak(selector.into_boxed_str())
}

/// The sidebar holds every route entry, the content plane lies to its
/// right, and `route`'s page, the only page rendered, lies in the plane.
#[track_caller]
fn page_in_content_plane(cx: &mut VisualTestContext, route: Route) -> Bounds<Pixels> {
    let sidebar = visible(cx, "sidebar");
    let navigation = visible(cx, "nav");
    assert!(
        within(navigation, sidebar),
        "the entries are in the sidebar"
    );
    for (other, _) in ROUTE_PAGES {
        let entry = visible(cx, nav_selector(other));
        assert!(
            within(entry, navigation),
            "`{}` is outside the navigation",
            nav_selector(other)
        );
    }
    let import = visible(cx, "import-audio");
    assert!(
        within(import, sidebar) && navigation.bottom() <= import.top(),
        "import audio sits in the sidebar below the entries"
    );
    let plane = visible(cx, "content-plane");
    assert!(
        sidebar.right() <= plane.left(),
        "the content plane lies right of the sidebar"
    );
    let mut page = None;
    for (other, selector) in ROUTE_PAGES {
        if other == route {
            let bounds = visible(cx, selector);
            assert!(
                within(bounds, plane),
                "the {route:?} page lies in the content plane: {bounds:?} in {plane:?}"
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
fn the_assistant_route_floats_the_input_bar_over_the_results(cx: &mut TestAppContext) {
    let (_window, cx, page) = open_route(cx, Route::Assistant);
    let body = visible(cx, "assistant-body");
    let results = scroll_content(cx, "assistant-results", body);
    let bar = visible(cx, "assistant-input-bar");
    let prompt = visible(cx, "assistant-input");
    let send = visible(cx, "assistant-submit");
    assert!(within(body, page) && within(bar, body));
    assert!(
        within(prompt, bar) && within(send, bar),
        "the prompt and the send button are in the bar"
    );
    assert!(
        prompt.right() <= send.left(),
        "send sits right of the prompt"
    );
    // The results fill the body, bar included, so they scroll under it; the
    // bar floats above the body's bottom edge.
    assert!(
        results.top() < bar.top() && results.bottom() >= bar.bottom(),
        "the results run under the bar: {results:?} and {bar:?}"
    );
    assert!(
        bar.bottom() < body.bottom() && body.bottom() == page.bottom(),
        "the bar floats above the bottom of the page: {bar:?} in {body:?}"
    );
}

#[gpui::test]
fn the_meetings_route_shows_the_tree_content_and_process_button(cx: &mut TestAppContext) {
    let (_window, cx, page) = open_route(cx, Route::Meetings);
    let sidebar = visible(cx, "sidebar");
    let tree = scroll_content(cx, "project-tree", sidebar);
    let content = scroll_content(cx, "meeting-content", page);
    let process = visible(cx, "process-project");
    assert!(within(process, page));
    assert!(
        tree.right() <= content.left(),
        "the tree sits left of the content"
    );

    let [root, child, second_root] =
        DOCUMENT_IDS.map(|id| visible(cx, leak(format!("project-row-{id}"))));
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
fn the_models_route_shows_the_catalog_and_aligned_status_chips(cx: &mut TestAppContext) {
    let (_window, cx, page) = open_route(cx, Route::Models);
    let catalog = support::catalog().models;
    let first_family = catalog
        .iter()
        .map(|model| model.model_family.clone())
        .min()
        .expect("the catalog lists models");
    let list = scroll_content(cx, "models-scroll", page);
    let toggle = visible(cx, leak(format!("model-family-{first_family}")));
    assert!(
        within(toggle, list) && within(toggle, page),
        "the first family toggle is inside the model list"
    );

    // Every family starts expanded, so every model row has a status chip;
    // the chips form one column whatever the model id's length.
    let chips = catalog
        .iter()
        .map(|model| rendered(cx, leak(format!("model-status-{}", model.id))))
        .collect::<Vec<_>>();
    assert!(chips.len() >= 3, "the fixture catalog lists several models");
    for chip in &chips {
        assert_eq!(
            (chip.left(), chip.size.width),
            (chips[0].left(), chips[0].size.width),
            "status chips share one column: {chips:?}"
        );
    }
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

#[gpui::test]
fn settings_controls_start_at_one_left_edge(cx: &mut TestAppContext) {
    let (window, cx) = open_window(cx);
    click_entry(cx, &window, Route::Settings);
    // Labels of different widths in three sections; the controls after them
    // must still start at the same x.
    let controls = [
        "select-theme",
        "select-material",
        "select-language",
        "select-asr-backend",
        "select-asr-model",
        "select-transcription-language",
    ]
    .map(|selector| (selector, rendered(cx, selector)));
    let (first, left) = (controls[0].0, controls[0].1.left());
    for (selector, bounds) in controls {
        assert_eq!(
            bounds.left(),
            left,
            "`{selector}` starts at {:?}, `{first}` at {left:?}",
            bounds.left()
        );
    }
}

#[gpui::test]
fn every_sidebar_entry_opens_its_route_and_only_it_is_active(cx: &mut TestAppContext) {
    let (window, cx) = open_window(cx);
    for route in [
        Route::Assistant,
        Route::Models,
        Route::Settings,
        Route::Meetings,
        Route::Assistant,
    ] {
        click_entry(cx, &window, route);
        for (other, _) in ROUTE_PAGES {
            let active = cx.debug_bounds(active_selector(other));
            if other == route {
                let entry = visible(cx, nav_selector(other));
                let marker = active.unwrap_or_else(|| panic!("{route:?} is not marked active"));
                assert!(within(marker, entry), "the active marker is in its entry");
            } else {
                assert!(
                    active.is_none(),
                    "{other:?} is marked active on the {route:?} route"
                );
            }
        }
    }
}

#[gpui::test]
fn the_project_tree_is_in_the_sidebar_on_meetings_only(cx: &mut TestAppContext) {
    let (window, cx) = open_window(cx);
    for route in [
        Route::Meetings,
        Route::Models,
        Route::Settings,
        Route::Assistant,
    ] {
        click_entry(cx, &window, route);
        let tree = cx.debug_bounds("project-tree");
        if route == Route::Meetings {
            let sidebar = visible(cx, "sidebar");
            let tree = scroll_content(cx, "project-tree", sidebar);
            assert!(
                visible(cx, "import-audio").bottom() <= tree.top(),
                "the tree sits below the entries and import button"
            );
            for id in DOCUMENT_IDS {
                let row = visible(cx, leak(format!("project-row-{id}")));
                assert!(within(row, sidebar), "`{id}` is in the sidebar");
            }
        } else {
            assert!(tree.is_none(), "the {route:?} route renders the tree");
        }
    }
}

#[gpui::test]
fn the_window_is_titled_and_both_columns_start_with_a_drag_strip(cx: &mut TestAppContext) {
    let (_window, cx) = open_window(cx);
    assert_eq!(cx.window_title().as_deref(), Some("MeetNote"));

    let sidebar = visible(cx, "sidebar");
    let plane = visible(cx, "content-plane");
    let sidebar_strip = visible(cx, "titlebar-drag-sidebar");
    let content_strip = visible(cx, "titlebar-drag-content");
    for strip in [sidebar_strip, content_strip] {
        assert_eq!(strip.top(), gpui::px(0.), "the strip starts at the top");
        assert_eq!(strip.size.height, sidebar_strip.size.height);
    }
    assert!(
        within(sidebar_strip, sidebar) && sidebar_strip.size.width == sidebar.size.width,
        "the sidebar strip spans the sidebar"
    );
    assert!(
        content_strip.left() >= sidebar.right() && content_strip.bottom() <= plane.top(),
        "the content strip lies above the content plane"
    );
    assert!(
        sidebar_strip.bottom() <= visible(cx, "nav").top(),
        "the entries start below the strip and the traffic lights"
    );
}

#[gpui::test]
fn the_settings_overlays_open_inside_the_window_over_the_page(cx: &mut TestAppContext) {
    let (window, cx) = open_window(cx);
    click_entry(cx, &window, Route::Settings);
    let page = visible(cx, "settings-page");

    cx.update(|_, cx| window.open_overlay(Overlay::ThemeSelect, cx));
    cx.run_until_parked();
    let trigger = visible(cx, "select-theme");
    let menu = visible(cx, "select-menu");
    assert!(
        menu.top() >= trigger.bottom() && within(menu, page),
        "the theme menu opens below its trigger on the page: {menu:?}"
    );
    for option in 0..3 {
        let row = visible(cx, leak(format!("select-option-{option}")));
        assert!(within(row, menu), "option {option} is in the menu");
    }

    cx.update(|_, cx| window.open_overlay(Overlay::SpeakerLibraryDialog, cx));
    cx.run_until_parked();
    let dialog = visible(cx, "confirm-dialog");
    let plane = visible(cx, "content-plane");
    assert!(
        dialog.center().x > plane.left() && dialog.center().x < plane.right(),
        "the dialog is centered over the window: {dialog:?}"
    );
    for button in ["dialog-confirm", "dialog-cancel"] {
        assert!(
            within(visible(cx, button), dialog),
            "`{button}` is in the dialog"
        );
    }
}
