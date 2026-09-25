//! MeetNote's interactive controls. Each wrapper requires an accessibility
//! label in its constructor, because GPUI's test windows build no
//! accessibility tree: the source scan below keeps pages and the controller
//! from constructing the underlying controls without one.
//!
//! - `Dropdown`: a select built on gpui-base's controlled `Select` root with
//!   a frosted menu.
//! - `ProgressBar`: a linear progress bar on gpui-base's `Progress` root.
//! - `ConfirmDialog`: a confirm/cancel dialog on gpui-base's `AlertDialog`
//!   host with a frosted sheet.

mod dialog;
mod progress;
mod select;

pub(crate) use dialog::ConfirmDialog;
pub(crate) use progress::ProgressBar;
pub(crate) use select::{Dropdown, DropdownEvent, SelectOption};

use gpui::{prelude::*, Animation, AnimationExt as _, AnyElement, Div, Pixels};
use std::time::Duration;

/// Fades `element` in while it rises `rise` into place over `duration`. An
/// overlay's entrance changes only its opacity and position, never its size
/// or blur radius, so no blur kernel or texture is rebuilt per frame. The
/// motion tokens are zero under reduced motion, which shows it at once.
fn entrance(id: &'static str, element: Div, duration: Duration, rise: Pixels) -> AnyElement {
    if duration.is_zero() {
        return element.into_any_element();
    }
    element
        .relative()
        .with_animation(
            id,
            Animation::new(duration).with_easing(gpui::ease_out_quint()),
            move |element, delta| element.opacity(delta).top(rise * (1. - delta)),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use crate::source_files::SOURCES;

    /// Constructors of unlabelled controls: gpui-component's styled select,
    /// progress and dialog, and gpui-base's unstyled roots. Only the wrappers
    /// in `controls/` may build them.
    const UNWRAPPED: [&str; 12] = [
        "select::Select",
        "SelectState",
        "Select::new(",
        "Progress::new(",
        "progress::Progress",
        "open_alert_dialog",
        "open_dialog",
        "Dialog::new(",
        "AlertDialog",
        "gpui_base::Select",
        "gpui_base::Dialog",
        "gpui_base::Progress",
    ];

    /// Whether `line` names `pattern` as a whole path: `ConfirmDialog::new(`
    /// does not name `Dialog::new(`.
    fn names(line: &str, pattern: &str) -> bool {
        line.match_indices(pattern).any(|(start, _)| {
            !line[..start]
                .chars()
                .next_back()
                .is_some_and(|before| before.is_alphanumeric() || before == '_')
        })
    }

    #[test]
    fn a_path_is_named_only_whole() {
        assert!(names("gpui_base::Select::new(\"s\")", "Select::new("));
        assert!(names("Dialog::new(cx)", "Dialog::new("));
        assert!(!names("ConfirmDialog::new(title, &focus)", "Dialog::new("));
        assert!(!names("AssistantProgress::new(", "Progress::new("));
    }

    #[test]
    fn only_the_wrappers_construct_selects_progress_bars_and_dialogs() {
        let found = SOURCES
            .iter()
            .filter(|(path, _)| !path.starts_with("controls/"))
            .flat_map(|(path, source)| {
                source.lines().enumerate().flat_map(move |(index, line)| {
                    UNWRAPPED
                        .iter()
                        .filter(move |pattern| names(line, pattern))
                        .map(move |pattern| format!("{path}:{}: {pattern}", index + 1))
                })
            })
            .collect::<Vec<_>>();
        assert!(
            found.is_empty(),
            "{} unlabelled control constructions outside controls/: {found:#?}",
            found.len()
        );
    }
}
