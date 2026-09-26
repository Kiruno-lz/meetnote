//! `Button`: a labelled push button on gpui-base's unstyled `Button`, which
//! gives it the button role, a tab stop and Enter/Space activation. MeetNote
//! paints the variant's token fills, the disabled look and, while keyboard
//! focus is on it, the focus ring. A press activates the button without
//! moving focus, so a field keeps focus when the button beside it is
//! clicked.

use super::focus_ring;
use crate::theme;
use gpui::{
    div, prelude::*, px, relative, App, ClickEvent, ElementId, Interactivity, IntoElement,
    MouseButton, Pixels, SharedString, StyleRefinement, Window,
};

/// Side padding of a regular button; a compact one uses `spacing.sm`.
const PADDING: Pixels = px(10.);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Variant {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

#[derive(IntoElement)]
pub(crate) struct Button {
    base: gpui_base::Button,
    label: SharedString,
    glyph: Option<SharedString>,
    variant: Variant,
    compact: bool,
    disabled: bool,
}

impl Button {
    /// `label` is the button's text and names it for assistive technology.
    /// The button is secondary until a variant is chosen.
    pub(crate) fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            base: gpui_base::Button::new(id),
            label: label.into(),
            glyph: None,
            variant: Variant::Secondary,
            compact: false,
            disabled: false,
        }
    }

    /// The one primary action of a group, or the selected choice.
    pub(crate) fn primary(mut self) -> Self {
        self.variant = Variant::Primary;
        self
    }

    /// A secondary action, or an unselected choice.
    pub(crate) fn secondary(mut self) -> Self {
        self.variant = Variant::Secondary;
        self
    }

    /// A borderless action on a panel.
    pub(crate) fn ghost(mut self) -> Self {
        self.variant = Variant::Ghost;
        self
    }

    /// A destructive action.
    pub(crate) fn danger(mut self) -> Self {
        self.variant = Variant::Danger;
        self
    }

    /// Narrower side padding, for buttons in rows of controls.
    pub(crate) fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    /// Shows `glyph` in place of the label, which still names the button.
    pub(crate) fn glyph(mut self, glyph: impl Into<SharedString>) -> Self {
        self.glyph = Some(glyph.into());
        self
    }

    /// A disabled button ignores presses and keys and is no tab stop.
    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// What a press, Enter or Space on the focused button runs.
    pub(crate) fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.base = self.base.on_click(handler);
        self
    }

    #[cfg(test)]
    pub(crate) fn accessibility_label(&self) -> &SharedString {
        &self.label
    }
}

/// Sizing styles (a width, `flex_1`) refine the button's own.
impl Styled for Button {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for Button {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl RenderOnce for Button {
    fn render(mut self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = *theme::tokens(cx);
        let colors = match self.variant {
            Variant::Primary => tokens.button.primary,
            Variant::Secondary => tokens.button.secondary,
            Variant::Ghost => tokens.button.ghost,
            Variant::Danger => tokens.button.danger,
        };
        let (fill, text) = if self.disabled {
            (colors.disabled, tokens.button.disabled_text)
        } else {
            (colors.fill, colors.text)
        };
        // The caller's styles apply over the button's own.
        let caller = std::mem::take(self.base.style());
        let ring = focus_ring(&tokens);
        let mut button = self
            .base
            .accessibility_label(self.label.clone())
            .disabled(self.disabled)
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .h(tokens.control_height.md)
            .px(if self.compact {
                tokens.spacing.sm
            } else {
                PADDING
            })
            .when(self.compact, |button| {
                button.min_w(tokens.control_height.md)
            })
            .rounded(tokens.radius.sm)
            .cursor_default()
            .bg(fill)
            .text_color(text)
            .focus_visible(move |style| style.shadow(vec![ring]))
            .when(!self.disabled, |button| {
                button
                    .hover(move |style| style.bg(colors.hover))
                    .active(move |style| style.bg(colors.pressed))
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            });
        button.style().refine(&caller);
        button.child(
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(tokens.font_size.md)
                .child(
                    div()
                        .flex_none()
                        .line_height(relative(1.))
                        .child(self.glyph.unwrap_or(self.label)),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Material, Mode, Tokens};
    use gpui::{
        Context, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, Render, TestAppContext,
        VisualTestContext,
    };
    use std::{cell::Cell, rc::Rc};

    struct Harness {
        saves: Rc<Cell<usize>>,
        disabled_presses: Rc<Cell<usize>>,
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let (saves, disabled_presses) = (self.saves.clone(), self.disabled_presses.clone());
            div()
                .size_full()
                .flex()
                .gap_2()
                .child(
                    Button::new("save", "Save")
                        .primary()
                        .debug_selector(|| "save".into())
                        .on_click(move |_, _, _| saves.set(saves.get() + 1)),
                )
                .child(
                    Button::new("delete", "Delete")
                        .danger()
                        .disabled(true)
                        .debug_selector(|| "delete".into())
                        .on_click(move |_, _, _| disabled_presses.set(disabled_presses.get() + 1)),
                )
        }
    }

    fn harness(
        cx: &mut TestAppContext,
    ) -> (&mut VisualTestContext, Rc<Cell<usize>>, Rc<Cell<usize>>) {
        cx.update(|cx| {
            theme::install_for_test(Tokens::new(Mode::Light, Material::Opaque, false), cx)
        });
        let (saves, disabled_presses) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
        let (_, cx) = cx.add_window_view({
            let (saves, disabled_presses) = (saves.clone(), disabled_presses.clone());
            move |_, _| Harness {
                saves,
                disabled_presses,
            }
        });
        cx.update(|window, cx| window.draw(cx).clear());
        (cx, saves, disabled_presses)
    }

    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        let bounds = cx.debug_bounds(selector).expect("the button is rendered");
        cx.simulate_click(bounds.center(), Modifiers::none());
    }

    /// Presses and releases `key`: a keyboard click fires on the release.
    fn press(cx: &mut VisualTestContext, key: &str) {
        let keystroke = Keystroke::parse(key).unwrap();
        cx.simulate_event(KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(KeyUpEvent { keystroke });
    }

    #[test]
    fn the_label_names_the_button_even_behind_a_glyph() {
        assert_eq!(Button::new("save", "Save").accessibility_label(), "Save");
        assert_eq!(
            Button::new("close", "Close detail")
                .glyph("×")
                .accessibility_label(),
            "Close detail"
        );
    }

    #[gpui::test]
    fn a_press_activates_the_button_without_taking_focus(cx: &mut TestAppContext) {
        let (cx, saves, disabled_presses) = harness(cx);
        let bounds = cx.debug_bounds("save").unwrap();
        assert_eq!(
            bounds.size.height,
            Tokens::new(Mode::Light, Material::Opaque, false)
                .control_height
                .md
        );

        click(cx, "save");
        click(cx, "delete");

        assert_eq!(saves.get(), 1);
        assert_eq!(disabled_presses.get(), 0, "a disabled button is inert");
        assert!(
            cx.update(|window, cx| window.focused(cx)).is_none(),
            "a press must not move focus"
        );
    }

    #[gpui::test]
    fn enter_and_space_activate_the_focused_button_and_skip_a_disabled_one(
        cx: &mut TestAppContext,
    ) {
        let (cx, saves, disabled_presses) = harness(cx);
        cx.update(|window, cx| window.focus_next(cx));
        let first = cx.update(|window, cx| window.focused(cx));
        assert!(first.is_some(), "the enabled button is a tab stop");
        cx.update(|window, cx| window.focus_next(cx));
        assert_eq!(
            cx.update(|window, cx| window.focused(cx)),
            first,
            "the disabled button is no tab stop"
        );

        press(cx, "enter");
        press(cx, "space");

        assert_eq!(saves.get(), 2);
        assert_eq!(disabled_presses.get(), 0);
    }
}
