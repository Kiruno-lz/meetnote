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
//! - `NavList` and `NavEntry`: the sidebar's route list and its entries.
//! - `Tree` and `TreeItem`: a tree that is one tab stop, moves among its
//!   items with the arrow keys and moves the selected item with Option and
//!   the arrows.
//! - `Region`: a labelled group that holds focus.
//! - `Separator` and `Scrollbar`: a hairline divider and a scroll area's
//!   scrollbar; neither is focusable, so neither takes a label.
//! - `window`: the window's first layer, which moves focus on Tab, cycles
//!   it inside focus traps and scrolls the control it moves to into view,
//!   and `DragStrip`, the titlebar strip that drags the window.
//!
//! Every focusable control shows the focus ring (`tokens.focus_ring`) only
//! while it holds keyboard focus: after a press it shows none.

mod button;
mod collapsible;
mod dialog;
mod nav;
mod progress;
mod radio_group;
mod region;
mod scrollbar;
mod select;
mod separator;
pub(crate) mod sidebar;
mod text_area;
mod text_field;
pub(crate) mod tree;
pub(crate) mod window;

pub(crate) use button::Button;
pub(crate) use collapsible::Collapsible;
pub(crate) use dialog::ConfirmDialog;
pub(crate) use nav::{NavEntry, NavList};
pub(crate) use progress::ProgressBar;
pub(crate) use radio_group::RadioGroup;
pub(crate) use region::Region;
pub(crate) use scrollbar::{follow_tokens, Scrollbar};
pub(crate) use select::{Dropdown, DropdownEvent, SelectOption};
pub(crate) use separator::Separator;
pub(crate) use text_area::TextArea;
#[cfg(test)]
pub(crate) use text_field::FieldInput;
pub(crate) use text_field::{TextField, TextFieldEvent};
pub(crate) use tree::{Tree, TreeItem, TreeShift, TreeStep};

use crate::theme::Tokens;
use gpui::{
    canvas, div, prelude::*, px, Animation, AnimationExt as _, AnyElement, BoxShadow, Div,
    FocusHandle, Pixels, Window,
};
use std::time::Duration;

/// How far the focus ring reaches outside a control's edge.
const FOCUS_RING_WIDTH: Pixels = px(2.);

/// The ring around a control with keyboard focus: a solid band outside its
/// edge, drawn as a shadow so it takes no layout space.
fn focus_ring(tokens: &Tokens) -> BoxShadow {
    BoxShadow::new(px(0.), px(0.), tokens.focus_ring).spread_radius(FOCUS_RING_WIDTH)
}

/// Whether the control that holds focus through `focus` shows its ring:
/// focus is on it and the last input was a key, so a press never shows one.
fn keyboard_focused(focus: &FocusHandle, window: &Window) -> bool {
    focus.is_focused(window) && window.last_input_was_keyboard()
}

/// `control`, which holds focus through `focus`, with its focus ring when
/// `shown`: the ring's band, and inside the control a layer that marks it
/// `focus-ring` for tests and, once keyboard focus has moved to it, scrolls
/// it into view (`window::reveal_painted`). `control` must be `relative`.
fn with_focus_ring<E: Styled + ParentElement>(
    control: E,
    shown: bool,
    focus: &FocusHandle,
    tokens: &Tokens,
) -> E {
    if !shown {
        return control;
    }
    let focus = focus.clone();
    control.shadow(vec![focus_ring(tokens)]).child(
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .debug_selector(|| "focus-ring".into())
            .child(
                canvas(
                    move |bounds, window, cx| window::reveal_painted(&focus, bounds, window, cx),
                    |_, _, _, _| {},
                )
                .size_full(),
            ),
    )
}

/// How focus arrived at a text field, which decides its ring: typing into
/// a field focused by a press shows none, and one reached from the keyboard
/// keeps its ring while it is typed in.
#[derive(Debug, Default)]
struct KeyboardArrival {
    focused: bool,
    from_keyboard: bool,
}

impl KeyboardArrival {
    /// Whether the field that holds focus through `focus` shows its ring;
    /// notes how focus arrived when it just did.
    fn ring(&mut self, focus: &FocusHandle, window: &Window) -> bool {
        let focused = focus.is_focused(window);
        if focused && !self.focused {
            self.from_keyboard = window.last_input_was_keyboard();
        }
        self.focused = focused;
        focused && self.from_keyboard
    }
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

    /// The methods that make an element respond to the pointer or the
    /// keyboard, take focus, or name it for assistive technology. Outside
    /// `controls/` only a chain that starts at a wrapper may call them, so
    /// every interactive element carries the role and label its wrapper
    /// gives it. Dragging and dropping stay pointer affordances of the page.
    const INTERACTIVE: [&str; 16] = [
        ".on_click(",
        ".on_mouse_down(",
        ".on_mouse_up(",
        ".on_mouse_down_out(",
        ".on_mouse_move(",
        ".on_key_down(",
        ".on_key_up(",
        ".on_action(",
        ".key_context(",
        ".track_focus(",
        ".tab_stop(",
        ".tab_index(",
        ".focus_visible(",
        ".focus(|",
        ".role(",
        ".aria_",
    ];

    /// The constructors a chain may start from to call `INTERACTIVE`.
    const WRAPPER_ROOTS: [&str; 10] = [
        "Button::new(",
        "Collapsible::new(",
        "NavEntry::new(",
        "NavList::new(",
        "SidebarPanel::new(",
        "Tree::new(",
        "TreeItem::new(",
        "Region::new(",
        "RadioGroup::new(",
        "DragStrip::titlebar(",
    ];

    /// The files that render: tests and the scans themselves build no UI.
    fn renders(path: &str) -> bool {
        !(path.starts_with("controls/")
            || path.ends_with("_tests.rs")
            || matches!(path, "lib.rs" | "test_support.rs" | "source_files.rs"))
    }

    /// The index of the bracket that `close`, a `)` or `]` in `source`,
    /// closes.
    fn opening(source: &[u8], close: usize) -> usize {
        let mut depth = 0usize;
        let mut index = close + 1;
        while index > 0 {
            index -= 1;
            match source[index] {
                b')' | b']' => depth += 1,
                b'(' | b'[' => {
                    depth -= 1;
                    if depth == 0 {
                        return index;
                    }
                }
                _ => {}
            }
        }
        0
    }

    /// Where the method chain that calls at `dot`, a `.` in `source`,
    /// starts: past every earlier link and its arguments, at a constructor
    /// such as `Button::new(`, a function such as `div(`, or a variable.
    fn chain_start(source: &str, dot: usize) -> usize {
        let bytes = source.as_bytes();
        // Skips whitespace and the line comments between two links.
        let skip_space = |mut index: usize| loop {
            while index > 0 && bytes[index - 1].is_ascii_whitespace() {
                index -= 1;
            }
            let line_start = source[..index].rfind('\n').map_or(0, |at| at + 1);
            match source[line_start..index].find("//") {
                Some(comment) => index = line_start + comment,
                None => return index,
            }
        };
        let mut end = dot;
        loop {
            let mut index = skip_space(end);
            while index > 0 && matches!(bytes[index - 1], b')' | b']') {
                index = opening(bytes, index - 1);
            }
            let name_end = index;
            while index > 0
                && (bytes[index - 1].is_ascii_alphanumeric()
                    || matches!(bytes[index - 1], b'_' | b':' | b'!'))
            {
                index -= 1;
            }
            if index == name_end {
                return index;
            }
            let before = skip_space(index);
            if before > 0 && bytes[before - 1] == b'.' {
                end = before - 1;
                continue;
            }
            return index;
        }
    }

    /// Whether the chain that starts at `start` in `source` starts at a
    /// wrapper, directly or through the `let` binding of a variable.
    fn starts_at_wrapper(source: &str, start: usize) -> bool {
        let root = &source[start..];
        if WRAPPER_ROOTS
            .iter()
            .any(|wrapper| root.starts_with(wrapper))
        {
            return true;
        }
        let name = root
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .next()
            .unwrap_or_default();
        let is_variable = !name.is_empty()
            && root[name.len()..].trim_start().starts_with('.')
            && name.starts_with(|c: char| c.is_lowercase() || c == '_');
        if !is_variable {
            return false;
        }
        // The bindings of `name` before the chain, nearest first. A binding
        // that branches (`let button = if … { button.primary() } …`)
        // shadows one of the same type, so the one before it decides.
        let mut bindings = ["let mut ", "let "]
            .iter()
            .flat_map(|binding| {
                let pattern = format!("{binding}{name} =");
                source[..start]
                    .match_indices(&pattern)
                    .map(|(at, _)| at + pattern.len())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        bindings.sort_unstable_by(|a, b| b.cmp(a));
        for initializer in bindings {
            let initializer = source[initializer..].trim_start();
            if WRAPPER_ROOTS
                .iter()
                .any(|wrapper| initializer.starts_with(wrapper))
            {
                return true;
            }
            if !(initializer.starts_with("if ") || initializer.starts_with("match ")) {
                return false;
            }
        }
        false
    }

    #[test]
    fn a_chain_starts_past_every_link_and_argument() {
        let start = |source: &str| {
            let dot = source.rfind(".on_click(").unwrap();
            source[chain_start(source, dot)..dot].trim_end().to_string()
        };
        assert_eq!(
            start("div()\n    .h(px(2.)) // a note.\n    // More (of it).\n    .on_click(f)"),
            "div()\n    .h(px(2.)) // a note.\n    // More (of it).",
            "comments between links are skipped"
        );
        assert_eq!(
            start("x.child(Button::new(\"a\", tr(l, \"a\")).primary().on_click(f))"),
            "Button::new(\"a\", tr(l, \"a\")).primary()"
        );
        assert_eq!(
            start("div()\n    .id(\"x\")\n    .child(Button::new(\"b\", \"B\"))\n    .on_click(f)"),
            "div()\n    .id(\"x\")\n    .child(Button::new(\"b\", \"B\"))"
        );
        assert_eq!(start("    button\n        .on_click(f)"), "button");
        assert_eq!(
            start("format!(\"{x}\").when(a, |this| this.b()).on_click(f)"),
            "format!(\"{x}\").when(a, |this| this.b())"
        );

        let wrapped = |source: &str| {
            let dot = source.rfind(".on_click(").unwrap();
            starts_at_wrapper(source, chain_start(source, dot))
        };
        assert!(wrapped("Button::new(\"a\", \"A\").on_click(f)"));
        assert!(wrapped(
            "let mut button =\n    Button::new(\"a\", \"A\");\nbutton = button.primary();\nbutton.on_click(f)"
        ));
        assert!(wrapped(
            "let button = Button::new(\"a\", \"A\");\nlet button = if a { button.primary() } else { button };\nbutton.on_click(f)"
        ));
        assert!(!wrapped("div().id(\"a\").on_click(f)"));
        assert!(!wrapped("let row = div().id(\"a\");\nrow.on_click(f)"));
        assert!(!wrapped(".when(a, |this| this.on_click(f))"));
        assert!(!wrapped("ConfirmButton::new(\"a\").on_click(f)"));
    }

    #[test]
    fn only_wrappers_are_interactive_outside_the_controls() {
        let found = SOURCES
            .iter()
            .filter(|(path, _)| renders(path))
            .flat_map(|(path, source)| {
                INTERACTIVE.iter().flat_map(move |method| {
                    source
                        .match_indices(method)
                        .filter(move |(dot, _)| {
                            !starts_at_wrapper(source, chain_start(source, *dot))
                        })
                        .map(move |(dot, _)| {
                            let line = source[..dot].lines().count();
                            let start = chain_start(source, dot);
                            let root = source[start..].lines().next().unwrap_or_default();
                            format!("{path}:{line}: `{method}` on `{}`", root.trim())
                        })
                })
            })
            .collect::<Vec<_>>();
        assert!(
            found.is_empty(),
            "{} interactive elements outside the labelled wrappers: {found:#?}",
            found.len()
        );
    }

    #[test]
    fn radio_choices_are_only_created_by_labelled_groups() {
        let found = SOURCES
            .iter()
            .filter(|(path, _)| renders(path))
            .flat_map(|(path, source)| {
                source
                    .match_indices(".selected(")
                    .filter_map(move |(dot, _)| {
                        let start = chain_start(source, dot);
                        let root = source[start..].trim_start();
                        (!root.starts_with("TreeItem::new(") && !root.starts_with("NavEntry::new("))
                            .then(|| format!("{path}:{}", source[..dot].lines().count()))
                    })
            })
            .collect::<Vec<_>>();
        assert!(
            found.is_empty(),
            "radio choices must be options of a labelled RadioGroup: {found:#?}"
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
        // Button, Collapsible, ConfirmDialog, Dropdown, NavEntry, NavList,
        // ProgressBar, Region, TextArea, TextField's plain and secret
        // constructors, Tree and TreeItem.
        assert_eq!(constructors, 15, "a wrapper constructor was added or lost");
        assert!(
            unnamed.is_empty(),
            "constructors without a name: {unnamed:#?}"
        );
    }
}
