//! The tooltip of a control shown without its label: the label and, when a
//! shortcut runs the same action, the shortcut in quieter text, on a small
//! frosted tag (`BlurLevel::Menu`), a tooltip for assistive technology.
//! GPUI shows it once the pointer has rested on the control for 500 ms,
//! beside the pointer, and hides it as the pointer leaves or presses.

use crate::theme::{self, BlurLevel};
use gpui::{div, prelude::*, px, AnyView, App, Context, Keystroke, Pixels, SharedString, Window};

/// Where the tag sits from the pointer, which GPUI places it at: below the
/// cursor arrow, so the pointer never covers the label.
const POINTER_GAP: (Pixels, Pixels) = (px(6.), px(18.));

pub(super) struct Tooltip {
    label: SharedString,
    shortcut: Option<SharedString>,
}

impl Tooltip {
    /// Builds the tooltip of `label` and the shortcut `keys` (in gpui's key
    /// binding syntax) whenever GPUI shows it.
    pub(super) fn builder(
        label: SharedString,
        keys: Option<&'static str>,
    ) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
        let shortcut = keys.map(|keys| SharedString::from(shortcut_label(keys)));
        move |_, cx| {
            let (label, shortcut) = (label.clone(), shortcut.clone());
            cx.new(|_| Self { label, shortcut }).into()
        }
    }
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = *theme::tokens(cx);
        let tag = gpui_base::Tooltip::new("tooltip")
            .px(tokens.spacing.sm)
            .py(tokens.spacing.xs)
            .flex()
            .items_center()
            .gap(tokens.spacing.md)
            .text_xs()
            .text_color(tokens.text)
            .whitespace_nowrap()
            .child(self.label.clone())
            .children(
                self.shortcut
                    .clone()
                    .map(|shortcut| div().text_color(tokens.text_muted).child(shortcut)),
            );
        div().pl(POINTER_GAP.0).pt(POINTER_GAP.1).child(
            div()
                .debug_selector(|| "tooltip".into())
                .child(theme::frosted(tokens.radius.sm, BlurLevel::Menu, tag)),
        )
    }
}

/// `keys`, in gpui's key binding syntax, as a Mac menu shows them: the
/// modifiers as symbols in the order Control, Option, Shift, Command, then
/// the key in capitals, such as `⌃⌘S` for `ctrl-cmd-s`.
fn shortcut_label(keys: &str) -> String {
    let Ok(keystroke) = Keystroke::parse(keys) else {
        return keys.to_string();
    };
    let modifiers = keystroke.modifiers;
    [
        (modifiers.control, "⌃"),
        (modifiers.alt, "⌥"),
        (modifiers.shift, "⇧"),
        (modifiers.platform, "⌘"),
    ]
    .into_iter()
    .filter_map(|(held, symbol)| held.then_some(symbol))
    .collect::<String>()
        + &keystroke.key.to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_read_as_a_mac_menu_shows_them() {
        for (keys, label) in [
            ("cmd-1", "⌘1"),
            ("cmd-,", "⌘,"),
            ("cmd-[", "⌘["),
            ("cmd-o", "⌘O"),
            ("ctrl-cmd-s", "⌃⌘S"),
            ("cmd-ctrl-s", "⌃⌘S"),
            ("shift-alt-cmd-z", "⌥⇧⌘Z"),
        ] {
            assert_eq!(shortcut_label(keys), label, "{keys}");
        }
    }
}
