//! L2: the window shell's feedback: the toast that shows the latest notice
//! at the top of the content plane. A fading toast leaves after its
//! lifetime on GPUI's test clock; one that stays leaves when it is closed
//! or replaced.

use crate::tests::{click_selector, open_desktop_with, test_settings, FakeAssistantUiService};
use crate::{
    controller::RootView,
    i18n::{self, Locale},
    state::{NoticeTone, TOAST_LIFETIME},
    theme,
};
use gpui::{px, Bounds, Pixels, TestAppContext, VisualTestContext};
use std::{sync::Arc, time::Duration};

type View = gpui::Entity<RootView>;

fn open(cx: &mut TestAppContext) -> (View, &mut VisualTestContext) {
    open_desktop_with(
        cx,
        Arc::new(FakeAssistantUiService::default()),
        test_settings(),
    )
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear());
}

/// Sets `notice` as the controller does, then draws.
fn set_notice(cx: &mut VisualTestContext, view: &View, notice: impl Into<String>) {
    let notice = notice.into();
    view.update(cx, |root, cx| {
        root.ui.notice = notice;
        cx.notify();
    });
    cx.run_until_parked();
    draw(cx);
}

/// Runs the timers due within `duration` on the test clock, then draws.
fn advance(cx: &mut VisualTestContext, duration: Duration) {
    cx.executor().advance_clock(duration);
    cx.run_until_parked();
    draw(cx);
}

fn toast(cx: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
    cx.debug_bounds("notice-toast")
}

fn notice(cx: &mut VisualTestContext, view: &View) -> String {
    view.read_with(cx, |root, _| root.ui.notice.clone())
}

/// The exit animation's length.
fn fade(cx: &mut VisualTestContext) -> Duration {
    cx.update(|_, cx| theme::tokens(cx).motion.menu)
}

#[gpui::test]
fn no_toast_opens_with_the_window(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    draw(cx);
    assert!(toast(cx).is_none(), "the window opens without a toast");
    assert_eq!(notice(cx, &view), "");
}

#[gpui::test]
fn a_success_toast_drops_in_at_the_top_of_the_content_and_fades(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    set_notice(
        cx,
        &view,
        i18n::message(Locale::ZhCn, "document_saved", "周会"),
    );
    let shown = toast(cx).expect("the notice opens a toast");
    let plane = cx.debug_bounds("content-plane").unwrap();
    assert!(
        shown.top() > plane.top() && shown.top() < plane.top() + px(32.),
        "the toast sits at the top of the content: {shown:?} in {plane:?}"
    );
    assert!(
        (shown.center().x - plane.center().x).abs() < px(1.),
        "the toast is centered over the content"
    );
    assert!(
        cx.debug_bounds("notice-toast-close").is_none(),
        "a fading toast has no close button"
    );
    let input = cx.debug_bounds("assistant-input").unwrap();
    assert!(shown.bottom() < input.top(), "clear of the input bar");

    advance(cx, TOAST_LIFETIME - Duration::from_millis(1));
    assert!(toast(cx).is_some(), "it stays for its lifetime");
    let fade = fade(cx);
    advance(cx, Duration::from_millis(1) + fade);
    assert!(toast(cx).is_none(), "it fades out after its lifetime");
    assert_eq!(notice(cx, &view), "", "the faded notice is cleared");
}

#[gpui::test]
fn an_error_toast_stays_until_it_is_closed(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    set_notice(
        cx,
        &view,
        i18n::message(Locale::ZhCn, "import_failed", "不支持的格式"),
    );
    assert!(toast(cx).is_some());
    advance(cx, Duration::from_secs(60));
    assert!(toast(cx).is_some(), "an error does not fade on its own");

    click_selector(cx, "notice-toast-close");
    let fade = fade(cx);
    advance(cx, fade);
    assert!(toast(cx).is_none(), "closing it removes it");
    assert_eq!(notice(cx, &view), "");
}

#[gpui::test]
fn a_new_notice_replaces_the_toast_and_a_repeated_one_opens_it_again(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    let warning = i18n::text(Locale::ZhCn, "select_project_first");
    set_notice(cx, &view, warning);
    let tone = |cx: &mut VisualTestContext| {
        view.read_with(cx, |root, _| root.ui.toast().map(|toast| toast.tone))
    };
    assert_eq!(tone(cx), Some(NoticeTone::Warning));

    set_notice(cx, &view, i18n::message(Locale::ZhCn, "imported", "周会"));
    assert_eq!(
        tone(cx),
        Some(NoticeTone::Success),
        "the new notice replaces it"
    );
    assert!(cx.debug_bounds("notice-toast-close").is_none());
    let fade = fade(cx);
    advance(cx, TOAST_LIFETIME + fade);
    assert!(toast(cx).is_none());

    // The same warning twice: closed, then raised again.
    set_notice(cx, &view, warning);
    click_selector(cx, "notice-toast-close");
    advance(cx, fade);
    assert!(toast(cx).is_none());
    set_notice(cx, &view, warning);
    assert!(toast(cx).is_some(), "the repeated warning shows again");

    // Progress the page already shows leaves the toast as it is.
    set_notice(
        cx,
        &view,
        i18n::message(Locale::ZhCn, "job_progress_total", "ASR|3|10"),
    );
    assert_eq!(tone(cx), Some(NoticeTone::Warning));

    // A cleared notice closes the toast.
    set_notice(cx, &view, "");
    advance(cx, fade);
    assert!(toast(cx).is_none(), "a cleared notice closes the toast");
}
