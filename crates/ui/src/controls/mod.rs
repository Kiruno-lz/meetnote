//! MeetNote's interactive controls and the window root. Each control wrapper
//! requires an accessibility label in its constructor, because GPUI's test
//! windows build no accessibility tree: the source scans below keep pages
//! and the controller from constructing the underlying primitives without
//! one. Every control is rendered from the theme tokens; gpui-base supplies
//! only unstyled behavior, and nothing outside this module names it.
//!
//! - `Button`: a push button on gpui-base's `Button`, in the primary,
//!   secondary, ghost or danger variant.
//! - `Collapsible`: a disclosure whose header is a gpui-base button.
//! - `Dropdown`: a select built on gpui-base's controlled `Select` root with
//!   a frosted menu.
//! - `ProgressBar`: a linear progress bar on gpui-base's `Progress` root.
//! - `ConfirmDialog`: a confirm/cancel dialog on gpui-base's `AlertDialog`
//!   host with a frosted sheet.
//! - `TextField`: a single-line text field on gpui-base's `InputState`, with
//!   a secret mode that masks the value and keeps it off the clipboard.
//! - `TextArea`: a multi-line text field on gpui-base's `TextareaState`.
//! - `Separator` and `Scrollbar`: a hairline divider and a scroll area's
//!   scrollbar; neither is focusable, so neither takes a label.
//! - `window`: the window's first layer, which cycles Tab inside focus
//!   traps.

mod button;
mod collapsible;
mod dialog;
mod progress;
mod scrollbar;
mod select;
mod separator;
mod text_area;
mod text_field;
pub(crate) mod window;

pub(crate) use button::Button;
pub(crate) use collapsible::Collapsible;
pub(crate) use dialog::ConfirmDialog;
pub(crate) use progress::ProgressBar;
pub(crate) use scrollbar::{follow_tokens, Scrollbar};
pub(crate) use select::{Dropdown, DropdownEvent, SelectOption};
pub(crate) use separator::Separator;
pub(crate) use text_area::TextArea;
#[cfg(test)]
pub(crate) use text_field::FieldInput;
pub(crate) use text_field::{TextField, TextFieldEvent};

use crate::theme::Tokens;
use gpui::{prelude::*, px, Animation, AnimationExt as _, AnyElement, BoxShadow, Div, Pixels};
use std::time::Duration;

/// How far the focus ring reaches outside a control's edge.
const FOCUS_RING_WIDTH: Pixels = px(2.);

/// The ring around a control with keyboard focus: a solid band outside its
/// edge, drawn as a shadow so it takes no layout space.
fn focus_ring(tokens: &Tokens) -> BoxShadow {
    BoxShadow::new(px(0.), px(0.), tokens.focus_ring).spread_radius(FOCUS_RING_WIDTH)
}

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

    /// What only the wrappers in `controls/` may name: gpui-base itself,
    /// whose unstyled roots (button, collapsible, scrollbar, select,
    /// progress, dialog, input) carry no label of their own; the retired
    /// styled API (button variants, disabling, scrollbar extensions and
    /// the styled select, progress, dialog and input); and the retired
    /// `TextInput`.
    const UNWRAPPED: [&str; 30] = [
        "gpui_base",
        "button::Button",
        "ButtonVariants",
        "Disableable",
        "collapsible::Collapsible",
        "separator::Separator",
        "ScrollableElement",
        "vertical_scrollbar(",
        "overflow_y_scrollbar(",
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
        "gpui_base::input",
        "gpui_base::Input",
        "InputState",
        "InputBase",
        "Input::new(",
        "EditorState",
        "TextareaState",
        "TextInput::new(",
        "TextInput::secret(",
    ];

    /// The styled crate's path, spelled so the scan does not find itself.
    const STYLED_CRATE: &str = concat!("gpui_component", "::");

    /// The crate's files that `SOURCES` does not list: its manifest, the
    /// workspace manifest and the integration tests.
    const OTHER_FILES: [(&str, &str); 7] = [
        ("Cargo.toml", include_str!("../../Cargo.toml")),
        ("../../Cargo.toml", include_str!("../../../../Cargo.toml")),
        ("tests/pixels.rs", include_str!("../../tests/pixels.rs")),
        ("tests/routes.rs", include_str!("../../tests/routes.rs")),
        ("tests/visual.rs", include_str!("../../tests/visual.rs")),
        (
            "tests/support/mod.rs",
            include_str!("../../tests/support/mod.rs"),
        ),
        (
            "tests/support/pixels.rs",
            include_str!("../../tests/support/pixels.rs"),
        ),
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
        assert!(!names("NumberInput::new(", "Input::new("));
        assert!(names(
            "gpui_base::input::InputState::new(window, cx)",
            "InputState"
        ));
        assert!(names("use gpui_base::{Button, Collapsible};", "gpui_base"));
        assert!(!names("my_gpui_base", "gpui_base"));
    }

    /// Whether a manifest `line` declares the styled `gpui-component`
    /// crate; its `gpui-component-assets` icons stay.
    fn declares_styled_crate(line: &str) -> bool {
        line.trim_start()
            .strip_prefix("gpui-component")
            .is_some_and(|rest| rest.starts_with([' ', '=', '.']))
    }

    #[test]
    fn nothing_in_the_crate_uses_the_styled_component_crate() {
        assert!(declares_styled_crate("gpui-component.workspace = true"));
        assert!(declares_styled_crate("gpui-component = { git = \"\" }"));
        assert!(!declares_styled_crate(
            "gpui-component-assets.workspace = true"
        ));
        assert!(!declares_styled_crate("# gpui-component fork"));

        let found = SOURCES
            .iter()
            .chain(OTHER_FILES.iter())
            .flat_map(|(path, source)| {
                let manifest = path.ends_with("Cargo.toml");
                source
                    .lines()
                    .enumerate()
                    .filter(move |(_, line)| {
                        line.contains(STYLED_CRATE) || (manifest && declares_styled_crate(line))
                    })
                    .map(move |(index, line)| format!("{path}:{}: {}", index + 1, line.trim()))
            })
            .collect::<Vec<_>>();
        assert!(
            found.is_empty(),
            "{} uses of the styled gpui-component crate: {found:#?}",
            found.len()
        );
    }

    #[test]
    fn only_the_wrappers_construct_controls() {
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

    /// The constructors of the wrappers. Separators and scrollbars are
    /// neither focusable nor named on their own, so they have none.
    const CONSTRUCTORS: [&str; 2] = ["pub(crate) fn new(", "pub(crate) fn secret("];

    /// The parameters that name a control for assistive technology.
    const NAMES: [&str; 2] = [
        "label: impl Into<SharedString>",
        "title: impl Into<SharedString>",
    ];

    #[test]
    fn every_wrapper_constructor_takes_the_accessible_name() {
        let mut constructors = 0;
        let mut unnamed = Vec::new();
        // This file names the constructors it looks for.
        let wrappers = SOURCES
            .iter()
            .filter(|(path, _)| path.starts_with("controls/") && *path != "controls/mod.rs");
        for (path, source) in wrappers {
            for constructor in CONSTRUCTORS {
                // Declarations only: a line that starts with the signature.
                let declarations = source.match_indices(constructor).filter(|(start, _)| {
                    source[..*start]
                        .rsplit('\n')
                        .next()
                        .is_some_and(|indent| indent.trim().is_empty())
                });
                for (start, _) in declarations {
                    let parameters = &source[start + constructor.len()..];
                    let parameters = &parameters[..parameters.find(") -> Self").unwrap_or(0)];
                    constructors += 1;
                    if !NAMES.iter().any(|name| parameters.contains(name)) {
                        unnamed.push(format!("{path}: {constructor}{}", parameters.trim()));
                    }
                }
            }
        }
        // Button, Collapsible, ConfirmDialog, Dropdown, ProgressBar,
        // TextArea and TextField's plain and secret constructors.
        assert_eq!(constructors, 8, "a wrapper constructor was added or lost");
        assert!(
            unnamed.is_empty(),
            "constructors without a name: {unnamed:#?}"
        );
    }
}
