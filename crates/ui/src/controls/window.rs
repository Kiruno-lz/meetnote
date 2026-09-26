//! The window's first layer. `root` paints the window's base fill in the UI
//! font and text color, and moves keyboard focus on Tab and Shift-Tab. Focus
//! inside a gpui-base focus trap, such as an open dialog's, cycles through
//! the trap's tab stops and never reaches the page behind it.

use crate::theme;
use gpui::{actions, div, prelude::*, App, Div, IntoElement, KeyBinding, Window};

actions!(meetnote_window, [FocusNext, FocusPrevious]);

const KEY_CONTEXT: &str = "MeetNoteWindow";

/// Installs gpui-base's key bindings (inputs, selects, dialogs) and the
/// window's Tab bindings. Inputs bind Tab themselves, closer to the focus.
pub(crate) fn init(cx: &mut App) {
    gpui_base::init(cx);
    cx.bind_keys([
        KeyBinding::new("tab", FocusNext, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-tab", FocusPrevious, Some(KEY_CONTEXT)),
    ]);
}

/// Prepares a new window for `root`: rem-based sizes follow the body font
/// size, and on macOS AppKit's accessibility hit test reaches GPUI's
/// elements (it asks the window, which forwards to its content view).
pub(crate) fn attach(window: &mut Window, cx: &App) {
    window.set_rem_size(theme::tokens(cx).font_size.md);
    #[cfg(target_os = "macos")]
    gpui_base::install_window_hit_test_forwarder(window);
}

/// `content` as the window's root element.
pub(crate) fn root(content: impl IntoElement, cx: &App) -> Div {
    let tokens = theme::tokens(cx);
    div()
        .key_context(KEY_CONTEXT)
        .on_action(|_: &FocusNext, window, cx| move_focus(true, window, cx))
        .on_action(|_: &FocusPrevious, window, cx| move_focus(false, window, cx))
        .relative()
        .size_full()
        .font_family(tokens.font_family)
        .bg(tokens.root_background())
        .text_color(tokens.text)
        .child(content)
}

/// Moves focus to the next (or previous) tab stop. Tab stops wrap around the
/// window; inside a focus trap, focus steps on past the stops outside it,
/// and gives up after one lap if the trap has none.
fn move_focus(forward: bool, window: &mut Window, cx: &mut App) {
    let step = |window: &mut Window, cx: &mut App| {
        if forward {
            window.focus_next(cx);
        } else {
            window.focus_prev(cx);
        }
    };
    let trap = gpui_base::active_focus_trap(window, cx);
    step(window, cx);
    let Some(trap) = trap else {
        return;
    };
    let first = window.focused(cx);
    while !trap.contains_focused(window, cx) {
        step(window, cx);
        if window.focused(cx) == first {
            return;
        }
    }
}
