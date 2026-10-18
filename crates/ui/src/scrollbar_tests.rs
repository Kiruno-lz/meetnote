//! L2: scrollbar tracks align with their container while content keeps its inset.
use crate::document_tests::{open_on, processed, rendered, select_row, DocumentsRoot};
use crate::route::Route;
use crate::tests::{click_selector, open_desktop_with, test_settings, FakeAssistantUiService};
use gpui::{point, px, size, Modifiers, MouseButton, TestAppContext, VisualTestContext};
use std::sync::Arc;

#[track_caller]
fn at_right_edge(cx: &mut VisualTestContext, scrollbar: &'static str, container: &'static str) {
    let track = rendered(cx, scrollbar);
    let area = rendered(cx, container);
    assert!(
        (track.right() - area.right()).abs() <= px(1.),
        "{scrollbar} must reach {container}'s right edge: {track:?} in {area:?}"
    );
}

#[gpui::test]
fn page_scrollbar_tracks_reach_the_container_edge_at_both_window_sizes(cx: &mut TestAppContext) {
    let (view, cx) = open_desktop_with(
        cx,
        Arc::new(FakeAssistantUiService::default()),
        test_settings(),
    );
    for dimensions in [size(px(900.), px(600.)), size(px(1180.), px(760.))] {
        cx.simulate_resize(dimensions);
        for (route, track, page) in [
            (
                Route::Assistant,
                "assistant-results-scrollbar-viewport",
                "assistant-page",
            ),
            (Route::Models, "models-scrollbar-viewport", "models-page"),
            (
                Route::Settings,
                "settings-scrollbar-viewport",
                "settings-page",
            ),
        ] {
            view.update(cx, |root, cx| root.navigate(route, cx));
            cx.run_until_parked();
            at_right_edge(cx, track, page);
        }
    }
}

#[gpui::test]
fn document_scrollbar_tracks_reach_each_panel_edge(cx: &mut TestAppContext) {
    let files = DocumentsRoot::new();
    files.put(processed("first", "First"), "# First\n");
    std::fs::create_dir_all(files.path().join("broken")).unwrap();
    std::fs::write(files.path().join("broken/state.json"), b"{invalid").unwrap();
    let (view, cx, _changes) = open_on(cx, &files);
    select_row(cx, &view, "first");
    for dimensions in [size(px(900.), px(600.)), size(px(1180.), px(760.))] {
        cx.simulate_resize(dimensions);
        cx.run_until_parked();
        at_right_edge(cx, "project-tree-scrollbar-viewport", "project-sidebar");
        at_right_edge(cx, "meeting-content-scrollbar-viewport", "meetings-page");
        at_right_edge(cx, "document-issues-scrollbar-viewport", "document-issues");
        click_selector(cx, "open-project-detail");
        at_right_edge(cx, "detail-scrollbar-viewport", "detail-panel");
        at_right_edge(cx, "meeting-content-scrollbar-viewport", "meetings-page");
        click_selector(cx, "close-detail");
    }
}

#[gpui::test]
fn the_settings_scrollbar_can_be_dragged_at_the_container_edge(cx: &mut TestAppContext) {
    let (view, cx) = open_desktop_with(
        cx,
        Arc::new(FakeAssistantUiService::default()),
        test_settings(),
    );
    cx.simulate_resize(size(px(900.), px(600.)));
    view.update(cx, |root, cx| root.navigate(Route::Settings, cx));
    cx.run_until_parked();
    let page = rendered(cx, "settings-page");
    let viewport = rendered(cx, "settings-scroll");
    let start = point(page.right() - px(6.), viewport.top() + px(12.));
    let end = start + point(px(0.), px(90.));
    cx.simulate_mouse_move(start, None, Modifiers::none());
    cx.run_until_parked();
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    let offset = view.read_with(cx, |root, _| root.scroll.settings.offset());
    assert!(
        offset.y < px(0.),
        "dragging the visible edge must scroll the actual form: {offset:?}"
    );
    assert_eq!(offset.x, px(0.));
}

#[gpui::test]
fn the_tree_scrollbar_and_sidebar_divider_have_separate_drag_targets(cx: &mut TestAppContext) {
    let files = DocumentsRoot::new();
    for index in 0..30 {
        let id = format!("recording-{index:02}");
        files.put(
            domain::DocumentState::new(domain::ProjectId::new(&id), &id, "recording.wav"),
            "",
        );
    }
    let (view, cx, _changes) = open_on(cx, &files);
    cx.simulate_resize(size(px(900.), px(600.)));
    cx.run_until_parked();
    let sidebar = rendered(cx, "project-sidebar");
    let track = rendered(cx, "project-tree-scrollbar-viewport");
    let start = point(sidebar.right() - px(7.), track.top() + px(16.));
    let end = start + point(px(0.), px(90.));
    cx.simulate_mouse_move(start, None, Modifiers::none());
    cx.run_until_parked();
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    let scroll = view.read_with(cx, |root, _| root.scroll.project_tree.offset());
    assert!(
        scroll.y < px(0.),
        "the visible thumb must scroll rather than resize: {scroll:?}"
    );
    assert_eq!(
        rendered(cx, "project-sidebar").size.width,
        sidebar.size.width
    );
    let divider = rendered(cx, "sidebar-resize-handle").center();
    let end = divider + point(px(20.), px(0.));
    cx.simulate_mouse_down(divider, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert!(rendered(cx, "project-sidebar").size.width > sidebar.size.width);
    assert_eq!(
        view.read_with(cx, |root, _| root.scroll.project_tree.offset()),
        scroll
    );
}
