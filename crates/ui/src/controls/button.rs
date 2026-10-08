//! `Button`: a labelled push button on gpui-base's unstyled `Button`, which
//! gives it the button role, a tab stop and Enter/Space activation. MeetNote
//! paints the variant's token fills, the disabled look and, while keyboard
//! focus is on it, the focus ring. A press activates the button without
//! moving focus, so a field keeps focus when the button beside it is
//! clicked. A choice among exclusive options (`selected`) is a radio button
//! for assistive technology, checked while it is the selected one.

use super::{keyboard_focused, with_focus_ring};
use crate::theme;
use gpui::{
    div, prelude::*, px, relative, App, ClickEvent, ElementId, FocusHandle, Interactivity,
    IntoElement, MouseButton, Pixels, Role, SharedString, StyleRefinement, Toggled, Window,
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
    id: ElementId,
    base: gpui_base::Button,
    label: SharedString,
    glyph: Option<SharedString>,
    icon: Option<SharedString>,
    leading: bool,
    variant: Variant,
    compact: bool,
    disabled: bool,
    /// Whether this choice is the selected one of its group.
    choice: Option<bool>,
    focus: Option<FocusHandle>,
}

impl Button {
    /// `label` is the button's text and names it for assistive technology.
    /// The button is secondary until a variant is chosen.
    pub(crate) fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        let id = id.into();
        Self {
            base: gpui_base::Button::new(id.clone()),
            id,
            label: label.into(),
            glyph: None,
            icon: None,
            leading: false,
            variant: Variant::Secondary,
            compact: false,
            disabled: false,
            choice: None,
            focus: None,
        }
    }

    /// The one primary action of a group.
    pub(crate) fn primary(mut self) -> Self {
        self.variant = Variant::Primary;
        self
    }

    /// A secondary action.
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

    /// One of a group of exclusive choices: filled as primary while
    /// `selected`, secondary otherwise, and a radio button checked while
    /// selected for assistive technology.
    pub(super) fn selected(mut self, selected: bool) -> Self {
        self.variant = if selected {
            Variant::Primary
        } else {
            Variant::Secondary
        };
        self.choice = Some(selected);
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

    pub(crate) fn icon(mut self, path: impl Into<SharedString>) -> Self {
        self.icon = Some(path.into());
        self
    }

    pub(crate) fn leading_icon(mut self, path: impl Into<SharedString>) -> Self {
        self.icon = Some(path.into());
        self.leading = true;
        self
    }

    /// A disabled button ignores presses and keys and is no tab stop.
    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Holds focus through `focus`, which the caller keeps, e.g. to return
    /// focus to the button when what it opened closes.
    pub(crate) fn focus_handle(mut self, focus: &FocusHandle) -> Self {
        self.focus = Some(focus.clone());
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

    /// The role and, for a choice, whether it is checked.
    #[cfg(test)]
    fn accessibility_role(&self) -> (Role, Option<bool>) {
        match self.choice {
            Some(selected) => (Role::RadioButton, Some(selected)),
            None => (Role::Button, None),
        }
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
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
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
        // The same keyed handle gpui-base would create, kept here so the
        // ring knows when the button holds keyboard focus.
        let focus = self.focus.clone().unwrap_or_else(|| {
            window
                .use_keyed_state(self.id.clone(), cx, |_, cx| cx.focus_handle())
                .read(cx)
                .clone()
        });
        let ring = !self.disabled && keyboard_focused(&focus, window);
        // The caller's styles apply over the button's own.
        let caller = std::mem::take(self.base.style());
        let mut button = self
            .base
            .track_focus(&focus)
            .accessibility_label(self.label.clone())
            .disabled(self.disabled)
            .when_some(self.choice, |button, selected| {
                button.role(Role::RadioButton).aria_toggled(if selected {
                    Toggled::True
                } else {
                    Toggled::False
                })
            })
            .relative()
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
            .when(!self.disabled, |button| {
                button
                    .hover(move |style| style.bg(colors.hover))
                    .active(move |style| style.bg(colors.pressed))
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            });
        button.style().refine(&caller);
        let button = button.child(
            div()
                .size_full()
                .flex()
                .items_center()
                .map(|content| {
                    if self.leading {
                        content.justify_start()
                    } else {
                        content.justify_center()
                    }
                })
                .text_size(tokens.font_size.md)
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(tokens.spacing.md)
                        .line_height(relative(1.))
                        .map(|content| match self.icon {
                            Some(path) => content
                                .child(gpui::svg().path(path).size(gpui::px(18.)).text_color(text))
                                .when(self.leading, |content| content.child(self.label)),
                            None => content.child(self.glyph.unwrap_or(self.label)),
                        }),
                ),
        );
        with_focus_ring(button, ring, &focus, &tokens)
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

    #[test]
    fn a_choice_is_a_radio_button_checked_while_selected() {
        assert_eq!(
            Button::new("save", "Save").accessibility_role(),
            (Role::Button, None)
        );
        for selected in [true, false] {
            let choice = Button::new("ask", "Ask").selected(selected);
            assert_eq!(
                choice.accessibility_role(),
                (Role::RadioButton, Some(selected))
            );
            let variant = if selected {
                Variant::Primary
            } else {
                Variant::Secondary
            };
            assert_eq!(choice.variant, variant);
        }
    }

    #[gpui::test]
    fn the_ring_shows_while_the_button_holds_keyboard_focus(cx: &mut TestAppContext) {
        let (cx, _, _) = harness(cx);
        let save = cx.debug_bounds("save").unwrap();
        cx.update(|window, cx| window.focus_next(cx));
        assert!(
            cx.debug_bounds("focus-ring").is_none(),
            "no ring before a key is pressed"
        );
        // A key the button ignores makes the keyboard the last input.
        cx.simulate_keystrokes("escape");
        assert_eq!(
            cx.debug_bounds("focus-ring"),
            Some(save),
            "keyboard focus rings the button"
        );
        cx.simulate_mouse_move(save.center(), None, Modifiers::none());
        assert!(
            cx.debug_bounds("focus-ring").is_none(),
            "the pointer hides the ring"
        );
        assert!(cx.update(|window, cx| window.focused(cx)).is_some());
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
