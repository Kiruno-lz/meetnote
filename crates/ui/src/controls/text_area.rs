//! `TextArea`: a labelled multi-line text field. gpui-base's `TextareaState`
//! owns editing (typing, IME composition through `EntityInputHandler`,
//! paste, undo/redo, soft wrap and its own scrolling); this entity owns the
//! label and paints the frame (`InputBase` with the multi-line text-input
//! role and the label for assistive technology).
//!
//! Like `TextField`, a value set without a window takes effect at the next
//! render; `value` reads a pending value at once.

use super::text_field::editor_style;
use crate::theme::{self, Tokens};
use gpui::{
    prelude::*, px, App, Context, Entity, FocusHandle, Focusable, IntoElement, MouseButton, Role,
    SharedString, Window,
};
use gpui_base::input::TextareaState;
use gpui_base::InputBase;

/// The height of one text row at the small font size, and the frame's
/// vertical padding.
const ROW_HEIGHT: f32 = 22.;
const PADDING: f32 = 8.;

pub(crate) struct TextArea {
    label: SharedString,
    input: Entity<TextareaState>,
    rows: usize,
    /// A value set without a window, applied at the next render.
    pending_value: Option<String>,
    /// The tokens the input's colors were last taken from.
    styled_with: Tokens,
}

impl Focusable for TextArea {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

impl TextArea {
    /// `label` names the area for assistive technology; `placeholder` shows
    /// while it is empty; the frame is `rows` text rows tall and the text
    /// scrolls inside it.
    pub(crate) fn new(
        label: impl Into<SharedString>,
        placeholder: impl Into<SharedString>,
        rows: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let label = label.into();
        debug_assert!(!label.is_empty(), "a text area needs a label");
        let tokens = *theme::tokens(cx);
        let placeholder = placeholder.into();
        let input = cx.new(|cx| {
            let mut input = TextareaState::new(window, cx).placeholder(placeholder);
            input.set_editor_style(editor_style(&tokens));
            input
        });
        Self {
            label,
            input,
            rows: rows.max(1),
            pending_value: None,
            styled_with: tokens,
        }
    }

    /// The value, including one set since the last render.
    pub(crate) fn value(&self, cx: &App) -> String {
        match &self.pending_value {
            Some(value) => value.clone(),
            None => self.input.read(cx).value().to_string(),
        }
    }

    /// Replaces the value and clears the undo history.
    pub(crate) fn set_value(&mut self, value: impl Into<String>, cx: &mut Context<Self>) {
        self.pending_value = Some(value.into());
        cx.notify();
    }

    /// Renames the area, e.g. after the locale changed.
    pub(crate) fn set_label(&mut self, label: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.label = label.into();
        debug_assert!(!self.label.is_empty(), "a text area needs a label");
        cx.notify();
    }

    /// The name assistive technology announces for the area.
    #[cfg(test)]
    pub(crate) fn accessibility_label(&self) -> &SharedString {
        &self.label
    }

    /// Applies a value set without a window, and new tokens.
    fn sync_input(&mut self, tokens: Tokens, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.pending_value.take();
        let restyle = self.styled_with != tokens;
        if value.is_none() && !restyle {
            return;
        }
        self.styled_with = tokens;
        self.input.update(cx, |input, cx| {
            if let Some(value) = value {
                input.set_value(value, window, cx);
            }
            if restyle {
                input.set_editor_style(editor_style(&tokens));
            }
        });
    }
}

impl Render for TextArea {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = *theme::tokens(cx);
        self.sync_input(tokens, window, cx);
        let focus = self.focus_handle(cx);
        let input = self.input.clone();
        InputBase::new("text-area")
            .role(Role::MultilineTextInput)
            .accessibility_label(self.label.clone())
            .focused(focus.is_focused(window))
            .styles(|styles| styles.focused(|style| style.border_color(tokens.accent)))
            .w_full()
            .h(px(self.rows as f32 * ROW_HEIGHT + 2. * PADDING))
            .px(tokens.spacing.sm)
            .py(px(PADDING))
            .flex()
            .rounded(tokens.radius.sm)
            .border_1()
            .border_color(tokens.border)
            .bg(tokens.surface)
            .text_size(tokens.font_size.sm)
            .text_color(tokens.text)
            // A press on the padding around the text focuses it too.
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                input.update(cx, |input, cx| input.focus(window, cx));
            })
            .child(self.input.clone())
    }
}

#[cfg(test)]
mod tests {
    /// The constructor takes the label first, so no area is built without
    /// one; the scan in `controls/mod.rs` keeps the rest of the crate on it.
    #[test]
    fn the_constructor_takes_the_label_first() {
        let source = include_str!("text_area.rs");
        let (_, after) = source
            .split_once("pub(crate) fn new(")
            .expect("`new` is missing");
        let first = after.trim_start().lines().next().unwrap_or_default();
        assert_eq!(first, "label: impl Into<SharedString>,");
    }
}
