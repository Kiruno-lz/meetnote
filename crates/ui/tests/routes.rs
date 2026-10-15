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
    wait_for(cx, |cx| cx.update(|_, cx| window.startup_loaded(cx)));
    assert_eq!(cx.update(|_, cx| window.route(cx)), Route::Assistant);
    (window, cx)
}

/// Worker completion is bounded by elapsed time, not the number of frames
/// the GPUI executor happened to render on this machine.
#[track_caller]
fn wait_for(cx: &mut VisualTestContext, done: impl Fn(&mut VisualTestContext) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the route state did not settle"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
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
    let sidebar = visible(cx, "navigation-rail");
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
fn the_assistant_route_keeps_results_clear_of_the_floating_input_bar(cx: &mut TestAppContext) {
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
    // The scroll viewport ends above the bar, so pointer and keyboard
    // navigation both keep the final result visible.
    assert!(
        results.top() < bar.top() && results.bottom() < bar.top(),
        "the results stay above the bar: {results:?} and {bar:?}"
    );
    assert!(
        bar.bottom() < body.bottom() && body.bottom() == page.bottom(),
        "the bar floats above the bottom of the page: {bar:?} in {body:?}"
    );
}

#[gpui::test]
fn the_meetings_route_shows_the_tree_content_and_process_button(cx: &mut TestAppContext) {
    let (_window, cx, page) = open_route(cx, Route::Meetings);
    let sidebar = visible(cx, "project-sidebar");
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

    // Each card's one action sits after its chip in a column of its own:
    // the installed model's re-probe and the failed one's retry line up.
    let [reprobe, retry] = [
        "model-reprobe-paraformer-zh",
        "model-retry-speakrs-community-1-cpu",
    ]
    .map(|selector| rendered(cx, selector));
    assert_eq!(
        (reprobe.left(), reprobe.size.width),
        (retry.left(), retry.size.width),
        "actions share one column"
    );
    assert!(reprobe.left() >= chips[0].right());
    for (model, action) in [
        ("paraformer-zh", reprobe),
        ("speakrs-community-1-cpu", retry),
    ] {
        let chip = rendered(cx, leak(format!("model-status-{model}")));
        // Centered on one row, up to pixel snapping.
        assert!(
            (chip.center().y - action.center().y).abs() <= gpui::px(0.5),
            "the {model} action shares its chip's row: {chip:?} {action:?}"
        );
    }
}

/// The fixture's records load through the installation service: the
/// installed model shows its passing probe and offers a re-probe, the
/// failed one its failure and a retry, and neither record is unreadable.
#[gpui::test]
fn the_models_route_shows_the_fixture_probe_record_and_failure(cx: &mut TestAppContext) {
    let (_window, cx, page) = open_route(cx, Route::Models);
    let [passing, failing] = ["paraformer-zh", "speakrs-community-1-cpu"];
    for (selector, shown) in [
        (format!("model-probe-{passing}"), true),
        (format!("model-reprobe-{passing}"), true),
        (format!("model-failure-{passing}"), false),
        (format!("model-record-error-{passing}"), false),
        // The failure stands for the failed download probe.
        (format!("model-probe-{failing}"), false),
        (format!("model-failure-{failing}"), true),
        (format!("model-retry-{failing}"), true),
        (format!("model-download-{failing}"), false),
        (format!("model-record-error-{failing}"), false),
    ] {
        let selector = leak(selector);
        if shown {
            assert!(
                within(visible(cx, selector), page),
                "{selector} is on the page"
            );
        } else {
            assert!(
                cx.debug_bounds(selector).is_none(),
                "{selector} is not shown"
            );
        }
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
            let sidebar = visible(cx, "project-sidebar");
            let tree = scroll_content(cx, "project-tree", sidebar);
            assert!(
                tree.top() >= sidebar.top(),
                "the tree starts inside the project sidebar"
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
fn the_toolbar_sits_above_the_fixed_rail_and_rounded_frame(cx: &mut TestAppContext) {
    let (_window, cx) = open_window(cx);
    assert_eq!(cx.window_title().as_deref(), Some("MeetNote"));
    let toolbar = visible(cx, "window-toolbar");
    let rail = visible(cx, "navigation-rail");
    let frame = visible(cx, "inner-frame");
    assert_eq!(toolbar.top(), gpui::px(0.));
    assert!(toolbar.bottom() <= frame.top() && toolbar.bottom() <= rail.top());
    for id in [
        "navigate-back",
        "navigate-forward",
        "toggle-sidebar",
        "titlebar-drag-content",
        "titlebar-drag-sidebar",
    ] {
        assert!(within(visible(cx, id), toolbar));
    }
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

// Keyboard and focus. Every focusable control draws its focus ring, marked
// `focus-ring`, only while it holds keyboard focus; a test tells which
// control holds it by the smallest listed selector whose bounds contain the
// ring. A press on a sidebar entry opens its route and leaves focus on the
// entry, so each traversal starts there.

/// The sidebar's tab stops, before the page's.
const SIDEBAR_STOPS: [&str; 5] = [
    "nav-Assistant",
    "nav-Meetings",
    "nav-Models",
    "nav-Settings",
    "import-audio",
];

/// The bounds of the focus ring, if a control shows one.
fn ring(cx: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
    cx.debug_bounds("focus-ring")
}

/// The control of `candidates` that shows the focus ring: the smallest
/// whose bounds contain the ring's.
fn focused_control(
    cx: &mut VisualTestContext,
    candidates: &[&'static str],
) -> Option<&'static str> {
    let ring = ring(cx)?;
    candidates
        .iter()
        .filter_map(|selector| Some((*selector, cx.debug_bounds(selector)?)))
        .filter(|(_, bounds)| within(ring, *bounds))
        .min_by(|(_, a), (_, b)| {
            let area = |bounds: &Bounds<Pixels>| {
                f32::from(bounds.size.width) * f32::from(bounds.size.height)
            };
            area(a).total_cmp(&area(b))
        })
        .map(|(selector, _)| selector)
}

/// Presses `key` until focus comes back to the stop the first press reached,
/// and returns the stops in visiting order: the listed control each one is,
/// or `None`. Every stop shows its ring inside the window: a stop in a
/// scrolled list is scrolled into view.
#[track_caller]
fn cycle(
    cx: &mut VisualTestContext,
    key: &str,
    candidates: &[&'static str],
) -> Vec<Option<&'static str>> {
    cx.simulate_keystrokes(key);
    let first = focused_control(cx, candidates).expect("the first stop is a listed control");
    let mut stops = vec![Some(first)];
    for _ in 0..200 {
        let bounds = ring(cx).unwrap_or_else(|| panic!("no ring after {stops:?}"));
        assert!(
            within(bounds, window_bounds()),
            "the ring of the stop after {stops:?} leaves the window: {bounds:?}"
        );
        cx.simulate_keystrokes(key);
        let stop = focused_control(cx, candidates);
        if stop == Some(first) {
            return stops;
        }
        stops.push(stop);
    }
    panic!("`{key}` never came back to `{first}`: {stops:?}")
}

/// `stops` rotated to start at the first sidebar entry.
fn from_the_first_entry(mut stops: Vec<Option<&'static str>>) -> Vec<Option<&'static str>> {
    let start = stops
        .iter()
        .position(|stop| *stop == Some(SIDEBAR_STOPS[0]))
        .expect("the traversal reaches the first entry");
    stops.rotate_left(start);
    stops
}

/// Tab visits `expected` in order and nothing else, and Shift-Tab visits it
/// in reverse.
#[track_caller]
fn assert_tab_order(cx: &mut VisualTestContext, expected: &[&'static str]) {
    let mut expected = expected.to_vec();
    for id in ["navigate-back", "navigate-forward", "toggle-sidebar"] {
        // All open_route fixtures have backward history and no forward history.
        if id == "navigate-forward" {
            continue;
        }
        let sidebar = cx.debug_bounds("project-sidebar").is_some()
            || cx.debug_bounds("settings-sidebar").is_some();
        if id != "toggle-sidebar" || sidebar {
            expected.push(id);
        }
    }
    let expected = expected.as_slice();
    let forward = from_the_first_entry(cycle(cx, "tab", expected));
    assert_eq!(
        forward,
        expected.iter().copied().map(Some).collect::<Vec<_>>(),
        "Tab order"
    );
    let mut backward = cycle(cx, "shift-tab", expected);
    backward.reverse();
    assert_eq!(from_the_first_entry(backward), forward, "Shift-Tab order");
}

/// Presses and releases `key`: a keyboard click fires on the release.
fn press(cx: &mut VisualTestContext, key: &str) {
    let keystroke = gpui::Keystroke::parse(key).unwrap();
    cx.simulate_event(gpui::KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(gpui::KeyUpEvent { keystroke });
    cx.run_until_parked();
}

/// Presses Tab until `selector` of `candidates` holds keyboard focus.
#[track_caller]
fn tab_to(cx: &mut VisualTestContext, selector: &'static str, candidates: &[&'static str]) {
    for _ in 0..60 {
        cx.simulate_keystrokes("tab");
        if focused_control(cx, candidates) == Some(selector) {
            return;
        }
    }
    panic!("Tab never reached `{selector}`");
}

#[gpui::test]
fn tab_traverses_the_assistant_route_in_order(cx: &mut TestAppContext) {
    let (_window, cx, _) = open_route(cx, Route::Assistant);
    let mut expected = SIDEBAR_STOPS.to_vec();
    expected.extend([
        "assistant-scene-ask",
        "assistant-scene-organize",
        "assistant-scene-topic-partition",
        "assistant-scene-topic-extract",
        "assistant-scene-move",
        "assistant-scope-current",
        "assistant-scope-all",
        "assistant-input",
        // Cancel is disabled while nothing runs, so it is no stop.
        "assistant-submit",
    ]);
    assert_tab_order(cx, &expected);
}

#[gpui::test]
fn tab_traverses_the_meetings_route_in_order(cx: &mut TestAppContext) {
    let (_window, cx, _) = open_route(cx, Route::Meetings);
    let mut expected = SIDEBAR_STOPS.to_vec();
    expected.extend([
        // The tree is one stop, on its selected row.
        leak(format!("project-row-{}", DOCUMENT_IDS[0])),
        "process-project",
        "open-project-detail",
        "content-mode-summary",
        "content-mode-original",
        "content-mode-edit",
    ]);
    assert_tab_order(cx, &expected);
}

#[gpui::test]
fn tab_traverses_the_models_route_in_order(cx: &mut TestAppContext) {
    let (_window, cx, _) = open_route(cx, Route::Models);
    // Families in name order, each header before its cards; each card's
    // header action before its details toggle.
    let mut families = std::collections::BTreeMap::<String, Vec<String>>::new();
    for model in support::catalog().models {
        families
            .entry(model.model_family.clone())
            .or_default()
            .push(model.id.clone());
    }
    let mut expected = SIDEBAR_STOPS.to_vec();
    for (family, models) in families {
        expected.push(leak(format!("model-family-{family}")));
        for model in models {
            for action in ["download", "retry", "reprobe", "details-toggle"] {
                let selector = leak(format!("model-{action}-{model}"));
                if cx.debug_bounds(selector).is_some() {
                    expected.push(selector);
                }
            }
        }
    }
    assert!(expected.len() > SIDEBAR_STOPS.len() + 6);
    assert_tab_order(cx, &expected);
}

#[gpui::test]
fn tab_traverses_the_settings_route_in_order(cx: &mut TestAppContext) {
    let (_window, cx, _) = open_route(cx, Route::Settings);
    // The form's selects, choose buttons and Assistant actions in reading
    // order; its text fields and Speaker Library rows lie between them.
    let mut listed = SIDEBAR_STOPS.to_vec();
    listed.extend((0..8).map(|index| leak(format!("settings-section-{index}"))));
    listed.extend([
        "select-theme",
        "select-material",
        "choose-documents-root",
        "select-language",
        "select-asr-backend",
        "select-asr-model",
        "select-transcription-language",
        "select-diarization-backend",
        "select-diarization-model",
        "select-expected-speakers",
        "choose-speaker-library",
        "select-compute-device",
        "assistant-api-key-field",
        "assistant-api-key-file-field",
        "settings-write-mode-automatic",
        "settings-write-mode-confirm-each",
        "assistant-save-settings",
        "assistant-readiness",
        "assistant-model-probe",
    ]);
    listed.extend(["navigate-back", "toggle-sidebar"]);
    let forward = from_the_first_entry(cycle(cx, "tab", &listed));
    assert_eq!(
        forward.iter().flatten().copied().collect::<Vec<_>>(),
        listed,
        "Tab reaches every listed control once, in order"
    );
    assert!(
        forward.len() > listed.len() + 10,
        "the text fields and profile rows are stops too: {forward:?}"
    );
    let mut backward = cycle(cx, "shift-tab", &listed);
    backward.reverse();
    assert_eq!(from_the_first_entry(backward), forward, "Shift-Tab order");
}

#[gpui::test]
fn tab_starts_at_the_first_entry_when_no_control_has_focus(cx: &mut TestAppContext) {
    let (window, cx) = open_window(cx);
    assert!(
        cx.update(|gpui_window, cx| window.root_holds_focus(gpui_window, cx)),
        "no control has focus at launch: the window root holds it"
    );
    cx.simulate_keystrokes("tab");
    assert_eq!(
        focused_control(cx, &SIDEBAR_STOPS),
        Some("nav-Assistant"),
        "Tab with nothing focused"
    );

    // The focused field leaves with its route.
    let prompt = visible(cx, "assistant-input");
    cx.simulate_click(prompt.center(), Modifiers::none());
    window_navigate(cx, &window, Route::Models);
    assert!(
        cx.update(|gpui_window, cx| window.root_holds_focus(gpui_window, cx)),
        "focus returns to the root with the field gone"
    );
    cx.simulate_keystrokes("tab");
    // With history to go back to, the toolbar's Back comes first.
    let stops = [&["navigate-back"], SIDEBAR_STOPS.as_slice()].concat();
    assert_eq!(
        focused_control(cx, &stops),
        Some("navigate-back"),
        "Tab from a control the route no longer shows"
    );
}

/// Opens `route` the way a shortcut or history would, leaving focus where
/// it was.
fn window_navigate(cx: &mut VisualTestContext, window: &MainWindow, route: Route) {
    cx.update(|_, cx| window.navigate(route, cx));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear());
}

#[gpui::test]
fn the_ring_shows_for_keyboard_focus_only(cx: &mut TestAppContext) {
    let (_window, cx, _) = open_route(cx, Route::Assistant);
    assert!(ring(cx).is_none(), "a press on an entry shows no ring");
    cx.simulate_keystrokes("tab");
    let focused = cx.update(|window, cx| window.focused(cx));
    assert!(ring(cx).is_some(), "Tab shows the ring");
    // Pointer input hides the ring; focus stays where it was.
    cx.simulate_mouse_move(
        gpui::point(gpui::px(600.), gpui::px(400.)),
        None,
        Modifiers::none(),
    );
    assert!(ring(cx).is_none(), "the pointer hides the ring");
    assert_eq!(cx.update(|window, cx| window.focused(cx)), focused);

    // A field focused by a press shows no ring, even while typing in it; one
    // reached with Tab does.
    let prompt = visible(cx, "assistant-input");
    cx.simulate_click(prompt.center(), Modifiers::none());
    cx.simulate_input("a");
    assert!(
        ring(cx).is_none(),
        "typing in a pressed field shows no ring"
    );
    cx.simulate_keystrokes("tab shift-tab");
    assert_eq!(
        focused_control(cx, &["assistant-input"]),
        Some("assistant-input")
    );
}

#[gpui::test]
fn enter_and_space_activate_the_focused_control(cx: &mut TestAppContext) {
    let (window, cx, _) = open_route(cx, Route::Assistant);
    let stops = [
        SIDEBAR_STOPS.as_slice(),
        &["open-project-detail", "close-detail"],
    ]
    .concat();
    tab_to(cx, "nav-Meetings", &stops);
    press(cx, "space");
    assert_eq!(cx.update(|_, cx| window.route(cx)), Route::Meetings);
    assert_eq!(
        focused_control(cx, &stops),
        Some("nav-Meetings"),
        "focus stays on the entry"
    );

    tab_to(cx, "open-project-detail", &stops);
    press(cx, "enter");
    assert!(
        cx.debug_bounds("detail-panel").is_some(),
        "Enter opens Detail"
    );
    tab_to(cx, "close-detail", &stops);
    press(cx, "enter");
    assert!(
        cx.debug_bounds("detail-panel").is_none(),
        "Enter closes Detail"
    );
    assert_eq!(
        focused_control(cx, &stops),
        Some("open-project-detail"),
        "focus returns to the button that opened Detail"
    );
}

#[gpui::test]
fn a_route_change_leaves_focus_on_the_route_entry(cx: &mut TestAppContext) {
    let (window, cx, _) = open_route(cx, Route::Assistant);
    let prompt = visible(cx, "assistant-input");
    cx.simulate_click(prompt.center(), Modifiers::none());
    click_entry(cx, &window, Route::Models);
    // The prompt left with its page; Tab goes on from the Models entry.
    cx.simulate_keystrokes("tab");
    assert_eq!(focused_control(cx, &SIDEBAR_STOPS), Some("nav-Settings"));
    cx.simulate_keystrokes("shift-tab shift-tab");
    assert_eq!(focused_control(cx, &SIDEBAR_STOPS), Some("nav-Meetings"));
}

#[gpui::test]
fn up_and_down_move_between_the_sidebar_entries(cx: &mut TestAppContext) {
    let (window, cx, _) = open_route(cx, Route::Assistant);
    tab_to(cx, "nav-Meetings", &SIDEBAR_STOPS);
    cx.simulate_keystrokes("down");
    assert_eq!(focused_control(cx, &SIDEBAR_STOPS), Some("nav-Models"));
    cx.simulate_keystrokes("up up");
    assert_eq!(focused_control(cx, &SIDEBAR_STOPS), Some("nav-Assistant"));
    cx.simulate_keystrokes("up");
    assert_eq!(
        focused_control(cx, &SIDEBAR_STOPS),
        Some("nav-Assistant"),
        "Up stops at the first entry"
    );
    assert_eq!(
        cx.update(|_, cx| window.route(cx)),
        Route::Assistant,
        "moving focus opens no route"
    );
}

#[gpui::test]
fn arrow_keys_move_through_the_project_tree(cx: &mut TestAppContext) {
    let (window, cx, _) = open_route(cx, Route::Meetings);
    let [weekly, decisions, interview] = DOCUMENT_IDS;
    let rows = DOCUMENT_IDS.map(|id| leak(format!("project-row-{id}")));
    let selected = |cx: &mut VisualTestContext| cx.update(|_, cx| window.selected_document(cx));
    tab_to(cx, rows[0], &rows);
    let step = |cx: &mut VisualTestContext, key: &str, document: &str, row: &'static str| {
        cx.simulate_keystrokes(key);
        assert_eq!(selected(cx).as_deref(), Some(document), "after `{key}`");
        assert_eq!(
            focused_control(cx, &rows),
            Some(row),
            "the ring after `{key}`"
        );
    };
    step(cx, "down", decisions, rows[1]);
    step(cx, "down", interview, rows[2]);
    step(cx, "down", interview, rows[2]);
    step(cx, "up", decisions, rows[1]);
    // Left on a child moves to its parent, then collapses the parent.
    step(cx, "left", weekly, rows[0]);
    cx.simulate_keystrokes("left");
    assert!(
        cx.debug_bounds(rows[1]).is_none(),
        "Left collapses the parent"
    );
    step(cx, "down", interview, rows[2]);
    step(cx, "up", weekly, rows[0]);
    // Right expands the parent, then moves to its first child.
    cx.simulate_keystrokes("right");
    assert!(
        cx.debug_bounds(rows[1]).is_some(),
        "Right expands the parent"
    );
    assert_eq!(selected(cx).as_deref(), Some(weekly));
    step(cx, "right", decisions, rows[1]);
}

#[gpui::test]
fn escape_closes_the_select_and_focus_returns_to_its_trigger(cx: &mut TestAppContext) {
    let (_window, cx, _) = open_route(cx, Route::Settings);
    let stops = [SIDEBAR_STOPS.as_slice(), &["select-theme"]].concat();
    tab_to(cx, "select-theme", &stops);
    for open in ["enter", "space"] {
        cx.simulate_keystrokes(open);
        assert!(
            cx.debug_bounds("select-menu").is_some(),
            "`{open}` opens the menu"
        );
        cx.simulate_keystrokes("escape");
        assert!(
            cx.debug_bounds("select-menu").is_none(),
            "Esc closes the menu"
        );
        assert_eq!(focused_control(cx, &stops), Some("select-theme"));
    }
}

#[gpui::test]
fn a_dialog_a_select_opened_takes_focus_and_escape_returns_it(cx: &mut TestAppContext) {
    let (window, cx, _) = open_route(cx, Route::Meetings);
    cx.update(|_, cx| window.open_overlay(Overlay::OriginalSpeakers, cx));
    cx.run_until_parked();
    let merge = "merge-speaker-speaker-0";
    let stops = [SIDEBAR_STOPS.as_slice(), &[merge]].concat();
    tab_to(cx, merge, &stops);
    // Choosing the other speaker from the keyboard asks before merging.
    cx.simulate_keystrokes("enter enter");
    assert!(cx.debug_bounds("select-menu").is_none());
    assert!(
        cx.debug_bounds("confirm-dialog").is_some(),
        "the merge asks first"
    );
    cx.simulate_keystrokes("escape");
    assert!(
        cx.debug_bounds("confirm-dialog").is_none(),
        "Esc cancels the merge"
    );
    assert_eq!(
        focused_control(cx, &stops),
        Some(merge),
        "focus returns to the select that opened the dialog"
    );
}

#[gpui::test]
fn a_dialog_a_button_opened_returns_focus_to_it(cx: &mut TestAppContext) {
    let (_window, cx, _) = open_route(cx, Route::Settings);
    let delete = "delete-speaker-profile-profile-li";
    let stops = [SIDEBAR_STOPS.as_slice(), &[delete]].concat();
    tab_to(cx, delete, &stops);
    press(cx, "enter");
    assert!(
        cx.debug_bounds("confirm-dialog").is_some(),
        "the delete asks first"
    );
    cx.simulate_keystrokes("escape");
    assert!(cx.debug_bounds("confirm-dialog").is_none());
    assert_eq!(focused_control(cx, &stops), Some(delete));
}

#[gpui::test]
fn ctrl_tab_leaves_a_text_area_that_takes_tab(cx: &mut TestAppContext) {
    let (_window, cx, _) = open_route(cx, Route::Meetings);
    let edit = visible(cx, "content-mode-edit");
    cx.simulate_click(edit.center(), Modifiers::none());
    wait_for(cx, |cx| cx.debug_bounds("document-editor").is_some());
    let stops = [SIDEBAR_STOPS.as_slice(), &["select-document-status"]].concat();
    tab_to(cx, "select-document-status", &stops);
    // The summary is four rows tall and the body twelve; scrolling to
    // reveal one moves the other, so they are told apart by height.
    let height = |cx: &mut VisualTestContext| ring(cx).map(|ring| ring.size.height);
    cx.simulate_keystrokes("tab");
    let summary = height(cx).expect("Tab reaches the summary");
    cx.simulate_keystrokes("tab");
    assert_eq!(height(cx), Some(summary), "Tab indents inside the summary");
    cx.simulate_keystrokes("ctrl-tab");
    let body = height(cx).expect("Ctrl-Tab moves on");
    assert!(
        body > summary,
        "Ctrl-Tab reaches the taller body: {summary:?} {body:?}"
    );
    cx.simulate_keystrokes("ctrl-shift-tab");
    assert_eq!(height(cx), Some(summary), "Ctrl-Shift-Tab moves back");
}

/// Runs the sidebar fold for `milliseconds` of the test clock, then draws.
fn fold_for(cx: &mut VisualTestContext, milliseconds: u64) {
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(milliseconds));
    cx.update(|window, cx| window.draw(cx).clear());
}

#[gpui::test]
fn project_sidebar_collapses_without_hiding_the_icon_rail(cx: &mut TestAppContext) {
    let (window, cx) = open_window(cx);
    click_entry(cx, &window, Route::Meetings);
    let rail = visible(cx, "navigation-rail");
    let sidebar = visible(cx, "project-sidebar");
    let frame = visible(cx, "inner-frame");
    let plane = visible(cx, "content-plane");
    assert!(rail.right() <= frame.left());
    assert!(within(sidebar, frame) && within(plane, frame));
    assert_eq!(sidebar.right(), plane.left(), "the inner seam has no gap");
    let toggle = visible(cx, "toggle-sidebar");
    cx.simulate_click(toggle.center(), Modifiers::none());
    cx.run_until_parked();
    fold_for(cx, 350);
    assert!(cx.debug_bounds("project-sidebar").is_none());
    assert_eq!(visible(cx, "navigation-rail"), rail);
    assert!(visible(cx, "content-plane").size.width > plane.size.width);
    cx.simulate_click(toggle.center(), Modifiers::none());
    cx.run_until_parked();
    fold_for(cx, 350);
    assert_eq!(visible(cx, "project-sidebar"), sidebar);
}

#[gpui::test]
fn toolbar_back_forward_and_new_navigation_follow_history(cx: &mut TestAppContext) {
    let (window, cx) = open_window(cx);
    click_entry(cx, &window, Route::Meetings);
    click_entry(cx, &window, Route::Settings);
    let click = |cx: &mut VisualTestContext, id| {
        let bounds = visible(cx, id);
        cx.simulate_click(bounds.center(), Modifiers::none());
        cx.run_until_parked();
    };
    click(cx, "navigate-back");
    assert_eq!(cx.update(|_, cx| window.route(cx)), Route::Meetings);
    click(cx, "navigate-back");
    assert_eq!(cx.update(|_, cx| window.route(cx)), Route::Assistant);
    click(cx, "navigate-forward");
    assert_eq!(cx.update(|_, cx| window.route(cx)), Route::Meetings);
    click_entry(cx, &window, Route::Models);
    click(cx, "navigate-forward");
    assert_eq!(cx.update(|_, cx| window.route(cx)), Route::Models);
}

/// Rests the pointer on `selector` for `rest` of the test clock, then
/// draws.
fn rest_on(cx: &mut VisualTestContext, selector: &'static str, rest: std::time::Duration) {
    let bounds = visible(cx, selector);
    cx.simulate_mouse_move(bounds.center(), None, Modifiers::none());
    cx.executor().advance_clock(rest);
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear());
}

#[gpui::test]
fn every_icon_only_control_names_itself_in_a_tooltip(cx: &mut TestAppContext) {
    let (window, cx) = open_window(cx);
    click_entry(cx, &window, Route::Meetings);
    let delay = std::time::Duration::from_millis(500);
    for selector in [
        "nav-Assistant",
        "nav-Meetings",
        "nav-Models",
        "nav-Settings",
        "import-audio",
        "navigate-back",
        "navigate-forward",
        "toggle-sidebar",
    ] {
        // Off every control, so each rest starts its own delay.
        let plane = visible(cx, "content-plane");
        cx.simulate_mouse_move(plane.center(), None, Modifiers::none());
        cx.run_until_parked();
        rest_on(cx, selector, delay - std::time::Duration::from_millis(1));
        assert!(
            cx.debug_bounds("tooltip").is_none(),
            "{selector}: no tooltip before the pointer rests"
        );
        rest_on(cx, selector, delay);
        let pointer = visible(cx, selector).center();
        let tooltip = visible(cx, "tooltip");
        // Below the pointer, or above it where the window ends.
        let beside = tooltip.left() > pointer.x
            && tooltip.left() < pointer.x + gpui::px(24.)
            && (tooltip.top() > pointer.y && tooltip.top() < pointer.y + gpui::px(32.)
                || tooltip.bottom() < pointer.y && tooltip.bottom() > pointer.y - gpui::px(32.));
        assert!(
            beside,
            "{selector}: the tooltip sits beside the pointer: {tooltip:?} {pointer:?}"
        );
    }
    let plane = visible(cx, "content-plane");
    cx.simulate_mouse_move(plane.center(), None, Modifiers::none());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear());
    assert!(
        cx.debug_bounds("tooltip").is_none(),
        "leaving the control hides it"
    );
}

#[gpui::test]
fn shortcuts_open_routes_travel_fold_the_sidebar_and_import(cx: &mut TestAppContext) {
    let (window, cx) = open_window(cx);
    let route = |cx: &mut VisualTestContext| cx.update(|_, cx| window.route(cx));
    assert!(
        cx.update(|gpui_window, cx| window.root_holds_focus(gpui_window, cx)),
        "the shortcuts work before any control has focus"
    );
    for (keys, expected) in [
        ("cmd-2", Route::Meetings),
        ("cmd-3", Route::Models),
        ("cmd-4", Route::Settings),
        ("cmd-1", Route::Assistant),
        ("cmd-,", Route::Settings),
    ] {
        cx.simulate_keystrokes(keys);
        assert_eq!(route(cx), expected, "{keys}");
    }
    cx.simulate_keystrokes("cmd-[");
    assert_eq!(route(cx), Route::Assistant, "⌘[ goes back");
    cx.simulate_keystrokes("cmd-]");
    assert_eq!(route(cx), Route::Settings, "⌘] goes forward");

    cx.simulate_keystrokes("cmd-2");
    cx.update(|window, cx| window.draw(cx).clear());
    let sidebar = visible(cx, "project-sidebar");
    cx.simulate_keystrokes("ctrl-cmd-s");
    fold_for(cx, 350);
    assert!(
        cx.debug_bounds("project-sidebar").is_none(),
        "⌃⌘S folds the sidebar"
    );
    cx.simulate_keystrokes("ctrl-cmd-s");
    fold_for(cx, 350);
    assert_eq!(visible(cx, "project-sidebar"), sidebar, "and unfolds it");

    assert!(!cx.cx.did_prompt_for_paths());
    cx.simulate_keystrokes("cmd-o");
    assert!(
        cx.cx.did_prompt_for_paths(),
        "⌘O asks for the audio to import"
    );
}

#[gpui::test]
fn shortcuts_pass_by_a_focused_field(cx: &mut TestAppContext) {
    let (window, cx, _) = open_route(cx, Route::Meetings);
    click_entry(cx, &window, Route::Assistant);
    let prompt = visible(cx, "assistant-input");
    cx.simulate_click(prompt.center(), Modifiers::none());
    cx.simulate_input("周会");
    let field = cx.update(|window, cx| window.focused(cx));
    assert!(field.is_some(), "the press focuses the field");
    cx.simulate_keystrokes("cmd-[");
    assert_eq!(cx.update(|_, cx| window.route(cx)), Route::Meetings);
    cx.simulate_keystrokes("cmd-1");
    assert_eq!(cx.update(|_, cx| window.route(cx)), Route::Assistant);
}

#[gpui::test]
fn dragging_the_sidebar_divider_resizes_and_clamps_then_returns(cx: &mut TestAppContext) {
    let (window, cx) = open_window(cx);
    click_entry(cx, &window, Route::Meetings);
    let original = visible(cx, "project-sidebar");
    let handle = visible(cx, "sidebar-resize-handle");
    let start = handle.center();
    cx.simulate_mouse_move(start, None, Modifiers::none());
    assert!(cx.debug_bounds("sidebar-resize-glow").is_some());
    cx.simulate_event(gpui::MouseDownEvent {
        position: start,
        button: gpui::MouseButton::Left,
        modifiers: Modifiers::none(),
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_mouse_move(
        start + gpui::point(gpui::px(60.), gpui::px(0.)),
        Some(gpui::MouseButton::Left),
        Modifiers::none(),
    );
    assert_eq!(
        visible(cx, "project-sidebar").size.width,
        original.size.width + gpui::px(60.)
    );
    let far = start + gpui::point(gpui::px(900.), gpui::px(0.));
    cx.simulate_mouse_move(far, Some(gpui::MouseButton::Left), Modifiers::none());
    assert!(visible(cx, "project-sidebar").size.width <= gpui::px(380.));
    cx.simulate_event(gpui::MouseUpEvent {
        position: far,
        button: gpui::MouseButton::Left,
        modifiers: Modifiers::none(),
        click_count: 1,
    });
    cx.run_until_parked();
    assert!(
        visible(cx, "project-sidebar").size.width > gpui::px(360.),
        "the release starts from the resisted width"
    );
    // The rebound is a GPUI animation on the wall clock; the wait only has
    // to outlast it.
    std::thread::sleep(std::time::Duration::from_millis(350));
    cx.update(|window, cx| window.draw(cx).clear());
    assert_eq!(visible(cx, "project-sidebar").size.width, gpui::px(360.));
}

#[gpui::test]
fn settings_categories_reveal_their_sections_and_collapse(cx: &mut TestAppContext) {
    let (window, cx) = open_window(cx);
    click_entry(cx, &window, Route::Settings);
    let select = visible(cx, "select-theme");
    let category = visible(cx, "settings-section-5");
    cx.simulate_click(category.center(), Modifiers::none());
    cx.run_until_parked();
    let library = visible(cx, "speaker-library-title");
    assert!(library.top() < visible(cx, "settings-page").top() + gpui::px(130.));
    let appearance = visible(cx, "settings-section-0");
    cx.simulate_click(appearance.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(visible(cx, "select-theme"), select);
    let toggle = visible(cx, "toggle-sidebar");
    cx.simulate_click(toggle.center(), Modifiers::none());
    cx.run_until_parked();
    fold_for(cx, 350);
    assert!(cx.debug_bounds("settings-sidebar").is_none());
    assert!(cx.debug_bounds("navigation-rail").is_some());
}

#[gpui::test]
fn project_history_restores_the_selected_document(cx: &mut TestAppContext) {
    let (window, cx) = open_window(cx);
    click_entry(cx, &window, Route::Meetings);
    let second = visible(cx, leak(format!("project-row-{}", DOCUMENT_IDS[2])));
    cx.simulate_click(second.center(), Modifiers::none());
    cx.run_until_parked();
    let back = visible(cx, "navigate-back");
    cx.simulate_click(back.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| window.selected_document(cx)).as_deref(),
        Some(DOCUMENT_IDS[0])
    );
    let forward = visible(cx, "navigate-forward");
    cx.simulate_click(forward.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| window.selected_document(cx)).as_deref(),
        Some(DOCUMENT_IDS[2])
    );
}

#[gpui::test]
fn settings_still_fit_the_minimum_window_width(cx: &mut TestAppContext) {
    let (window, cx) = open_window(cx);
    click_entry(cx, &window, Route::Settings);
    cx.simulate_resize(gpui::size(gpui::px(900.), gpui::px(600.)));
    cx.run_until_parked();
    let page = rendered(cx, "settings-page");
    for id in [
        "select-theme",
        "select-material",
        "choose-documents-root",
        "select-language",
    ] {
        let control = rendered(cx, id);
        assert!(
            control.right() <= page.right(),
            "{id} overflows at the minimum width"
        );
    }
}

#[gpui::test]
fn sidebar_fold_keeps_content_stable_and_can_reverse_mid_animation(cx: &mut TestAppContext) {
    let (window, cx) = open_window(cx);
    click_entry(cx, &window, Route::Meetings);
    let original = visible(cx, "project-sidebar");
    let row = visible(cx, leak(format!("project-row-{}", DOCUMENT_IDS[0])));
    let toggle = visible(cx, "toggle-sidebar");
    cx.simulate_click(toggle.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("project-sidebar").is_some(),
        "the sidebar must remain while folding"
    );
    fold_for(cx, 100);
    let partial = rendered(cx, "project-sidebar").size.width;
    assert!(
        partial > gpui::px(0.) && partial < original.size.width,
        "an intermediate fold width must be visible"
    );
    assert_eq!(
        rendered(cx, leak(format!("project-row-{}", DOCUMENT_IDS[0])))
            .size
            .width,
        row.size.width,
        "the tree must crop smoothly instead of reflowing"
    );
    cx.simulate_click(toggle.center(), Modifiers::none());
    cx.run_until_parked();
    let reversed = rendered(cx, "project-sidebar").size.width;
    assert!(
        (reversed - partial).abs() < gpui::px(12.),
        "reversal must begin at the current width: {partial:?} -> {reversed:?}"
    );
    fold_for(cx, 350);
    assert_eq!(
        visible(cx, "project-sidebar").size.width,
        original.size.width
    );
}

#[gpui::test]
fn the_focused_select_visual_fixture_reaches_the_theme_control(cx: &mut TestAppContext) {
    let (window, cx, _) = open_route(cx, Route::Settings);
    cx.cx
        .update(|cx| window.open_overlay(Overlay::FocusedSelect, cx));
    cx.run_until_parked();
    assert!(within(
        ring(cx).expect("keyboard focus ring"),
        visible(cx, "select-theme")
    ));
}

#[gpui::test]
fn an_empty_workspace_offers_import_where_the_work_begins(cx: &mut TestAppContext) {
    let (window, cx, _) = open_route(cx, Route::Meetings);
    cx.update(|_, cx| window.open_overlay(Overlay::EmptyWorkspace, cx));
    cx.run_until_parked();
    let action = visible(cx, "empty-import-audio");
    assert!(within(action, visible(cx, "content-plane")));
    assert!(!cx.cx.did_prompt_for_paths());
    cx.simulate_click(action.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(
        cx.cx.did_prompt_for_paths(),
        "the main action opens the audio picker"
    );
}

#[gpui::test]
fn an_empty_workspace_import_is_reachable_by_keyboard_at_minimum_size(cx: &mut TestAppContext) {
    let (window, cx, _) = open_route(cx, Route::Meetings);
    cx.update(|_, cx| window.open_overlay(Overlay::EmptyWorkspace, cx));
    cx.simulate_resize(gpui::size(gpui::px(900.), gpui::px(600.)));
    cx.run_until_parked();
    let viewport = Bounds::from_corners(
        gpui::point(gpui::px(0.), gpui::px(0.)),
        gpui::point(gpui::px(900.), gpui::px(600.)),
    );
    assert!(within(rendered(cx, "empty-import-audio"), viewport));
    tab_to(cx, "empty-import-audio", &["empty-import-audio"]);
    press(cx, "enter");
    assert!(
        cx.cx.did_prompt_for_paths(),
        "Enter imports from the empty workspace"
    );
}

#[gpui::test]
fn first_use_fixtures_reach_settings_and_import_at_minimum_size(cx: &mut TestAppContext) {
    let (window, cx, _) = open_route(cx, Route::Assistant);
    cx.simulate_resize(gpui::size(gpui::px(900.), gpui::px(600.)));
    cx.update(|_, cx| window.open_overlay(Overlay::AssistantSetup, cx));
    cx.run_until_parked();
    let setup = rendered(cx, "assistant-first-settings");
    assert!(within(setup, rendered(cx, "assistant-results")));
    cx.simulate_click(setup.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| window.route(cx)), Route::Settings);
    cx.update(|_, cx| {
        window.close_overlay(Overlay::AssistantSetup, cx);
        window.navigate(Route::Assistant, cx);
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("assistant-first-settings").is_none());
    cx.update(|_, cx| window.open_overlay(Overlay::AssistantImport, cx));
    cx.run_until_parked();
    assert!(within(
        rendered(cx, "assistant-first-import"),
        rendered(cx, "assistant-results")
    ));
    tab_to(cx, "assistant-first-import", &["assistant-first-import"]);
    press(cx, "enter");
    assert!(cx.cx.did_prompt_for_paths());
    cx.update(|_, cx| window.close_overlay(Overlay::AssistantImport, cx));
    wait_for(cx, |cx| cx.debug_bounds("assistant-first-import").is_none());
}
