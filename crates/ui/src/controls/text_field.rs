//! `TextField`: a labelled single-line text field. gpui-base's `InputState`
//! owns editing: typing, IME composition through `EntityInputHandler`, paste,
//! undo/redo, select-all, word and line movement, and the `Input` key
//! bindings that `gpui_base::init` installs. This entity owns the label and
//! paints the frame (`InputBase`, which carries the text-input role and the
//! label for assistive technology).
//!
//! A secret field masks its value and keeps it off the clipboard: the frame
//! intercepts Copy and Cut before the input sees them. Cut does nothing, so
//! the value and the selection stay; removing the key without putting it on
//! the clipboard would lose it silently.
//!
//! The editing state needs a window for every change, while the controller
//! also loads values where it has none (a settings save completes on a
//! worker). `set_value` and `set_placeholder` therefore take effect at the
//! next render; `value` reads a pending value at once.

use crate::theme::{self, Tokens};
use gpui::{
    prelude::*, App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    MouseButton, Role, SharedString, Subscription, Window,
};
use gpui_base::input::{
    Copy as CopyAction, Cut as CutAction, InputEditorStyle, InputEvent, InputState,
};
use gpui_base::InputBase;

/// The editing state behind a field, for tests that drive its
/// `EntityInputHandler` as the platform does.
#[cfg(test)]
pub(crate) type FieldInput = InputState;

/// What a `TextField` reports to its owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextFieldEvent {
    /// Enter was pressed in the field.
    Submit,
}

pub(crate) struct TextField {
    label: SharedString,
    input: Entity<InputState>,
    secret: bool,
    large: bool,
    /// A value set without a window, applied at the next render.
    pending_value: Option<String>,
    /// A placeholder set without a window, applied at the next render.
    pending_placeholder: Option<SharedString>,
    /// The tokens the input's colors were last taken from.
    styled_with: Tokens,
    _subscription: Subscription,
}

impl EventEmitter<TextFieldEvent> for TextField {}

impl Focusable for TextField {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

/// The colors the input paints its text, placeholder, selection and caret
/// with; the frame paints the fill and the border.
fn editor_style(tokens: &Tokens) -> InputEditorStyle {
    InputEditorStyle {
        foreground: tokens.text,
        muted_foreground: tokens.text_muted,
        background: tokens.surface,
        border: tokens.border,
        selection: tokens.selection,
        caret: tokens.caret,
        ..InputEditorStyle::default()
    }
}

impl TextField {
    /// `label` names the field for assistive technology; `placeholder`
    /// shows while it is empty.
    pub(crate) fn new(
        label: impl Into<SharedString>,
        placeholder: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(label.into(), placeholder.into(), false, window, cx)
    }

    /// A field for a credential: the value is masked and never reaches the
    /// clipboard.
    pub(crate) fn secret(
        label: impl Into<SharedString>,
        placeholder: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(label.into(), placeholder.into(), true, window, cx)
    }

    fn build(
        label: SharedString,
        placeholder: SharedString,
        secret: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        debug_assert!(!label.is_empty(), "a text field needs a label");
        let tokens = *theme::tokens(cx);
        let input = cx.new(|cx| {
            let mut input = InputState::new(window, cx)
                .placeholder(placeholder)
                .masked(secret);
            input.set_editor_style(editor_style(&tokens));
            input
        });
        let subscription = cx.subscribe(&input, |_, _, event: &InputEvent, cx| {
            if let InputEvent::PressEnter { .. } = event {
                cx.emit(TextFieldEvent::Submit);
            }
        });
        Self {
            label,
            input,
            secret,
            large: false,
            pending_value: None,
            pending_placeholder: None,
            styled_with: tokens,
            _subscription: subscription,
        }
    }

    /// Makes this the taller field of the Assistant input bar.
    pub(crate) fn set_large(&mut self, cx: &mut Context<Self>) {
        self.large = true;
        cx.notify();
    }

    /// The value, including one set since the last render.
    pub(crate) fn value(&self, cx: &App) -> String {
        match &self.pending_value {
            Some(value) => value.clone(),
            None => self.input.read(cx).value().to_string(),
        }
    }

    /// Replaces the value and clears the undo history; the caret moves to
    /// the end.
    pub(crate) fn set_value(&mut self, value: impl Into<String>, cx: &mut Context<Self>) {
        self.pending_value = Some(value.into());
        cx.notify();
    }

    pub(crate) fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.pending_placeholder = Some(placeholder.into());
        cx.notify();
    }

    /// Renames the field, e.g. after the locale changed.
    pub(crate) fn set_label(&mut self, label: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.label = label.into();
        debug_assert!(!self.label.is_empty(), "a text field needs a label");
        cx.notify();
    }

    /// The name assistive technology announces for the field.
    #[cfg(test)]
    pub(crate) fn accessibility_label(&self) -> &SharedString {
        &self.label
    }

    /// The editing state, whose `EntityInputHandler` the platform drives.
    #[cfg(test)]
    pub(crate) fn input(&self) -> &Entity<InputState> {
        &self.input
    }

    /// Applies what was set without a window, and new tokens.
    fn sync_input(&mut self, tokens: Tokens, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.pending_value.take();
        let placeholder = self.pending_placeholder.take();
        let restyle = self.styled_with != tokens;
        if value.is_none() && placeholder.is_none() && !restyle {
            return;
        }
        self.styled_with = tokens;
        self.input.update(cx, |input, cx| {
            if let Some(value) = value {
                input.set_value(value, window, cx);
            }
            if let Some(placeholder) = placeholder {
                input.set_placeholder(placeholder, window, cx);
            }
            if restyle {
                input.set_editor_style(editor_style(&tokens));
            }
        });
    }
}

impl Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = *theme::tokens(cx);
        self.sync_input(tokens, window, cx);
        let focus = self.focus_handle(cx);
        let input = self.input.clone();
        let (height, font_size) = if self.large {
            (tokens.control_height.lg, tokens.font_size.md)
        } else {
            (tokens.control_height.md, tokens.font_size.sm)
        };
        let mut frame = InputBase::new("text-field")
            .role(if self.secret {
                Role::PasswordInput
            } else {
                Role::TextInput
            })
            .accessibility_label(self.label.clone())
            .focused(focus.is_focused(window))
            .styles(|styles| styles.focused(|style| style.border_color(tokens.accent)))
            .w_full()
            .h(height)
            .px(tokens.spacing.sm)
            .flex()
            .items_center()
            .rounded(tokens.radius.sm)
            .border_1()
            .border_color(tokens.border)
            .bg(tokens.surface)
            .text_size(font_size)
            .text_color(tokens.text)
            // A press on the padding around the text line focuses it too.
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                input.update(cx, |input, cx| input.focus(window, cx));
            });
        if self.secret {
            // The capture phase runs before the input's own handlers;
            // stopping it there keeps the key off the clipboard.
            frame = frame
                .capture_action(|_: &CopyAction, _, cx| cx.stop_propagation())
                .capture_action(|_: &CutAction, _, cx| cx.stop_propagation());
        }
        frame.child(self.input.clone())
    }
}

#[cfg(test)]
mod tests {
    /// Every constructor takes the label first, so no field is built
    /// without one; the scan in `controls/mod.rs` keeps the rest of the
    /// crate on these constructors.
    #[test]
    fn every_constructor_takes_the_label_first() {
        let source = include_str!("text_field.rs");
        let constructors = ["pub(crate) fn new(", "pub(crate) fn secret("];
        for constructor in constructors {
            let (_, after) = source
                .split_once(constructor)
                .unwrap_or_else(|| panic!("`{constructor}` is missing"));
            let first = after.trim_start().lines().next().unwrap_or_default();
            assert_eq!(
                first, "label: impl Into<SharedString>,",
                "`{constructor}` must take the label first"
            );
        }
    }
}
