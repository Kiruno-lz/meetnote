// Portions adapted from Zeron (https://github.com/zeronsh/zeron), Copyright (c) 2026 Wing, MIT License.

//! `Dropdown`: a labelled select. gpui-base's controlled `Select` root owns
//! the combobox semantics, keyboard opening and dismissal (Up/Down open it,
//! Enter or Space opens it or confirms, Esc dismisses it) and the focus transfer
//! between the trigger and the menu. This entity owns the options, the
//! selection, the open state and the keyboard highlight, and paints the
//! trigger and a frosted menu (`BlurLevel::Menu`) below it.
//!
//! The menu closes on a confirmation, on Esc and on a press outside it, and
//! focus returns to the trigger; a press on another focusable element then
//! takes focus as usual. The trigger shows the focus ring while it holds
//! keyboard focus.

use super::{entrance, keyboard_focused, with_focus_ring};
use crate::theme::{self, BlurLevel, Tokens};
use gpui::{
    canvas, deferred, div, prelude::*, px, Bounds, Context, EventEmitter, FocusHandle, Focusable,
    IntoElement, MouseButton, Pixels, Role, ScrollHandle, SharedString, Window,
};
use gpui_base::actions::{SelectDown, SelectUp};
use gpui_base::{Align, Placement, Positioner};
use std::cell::Cell;
use std::rc::Rc;

/// The tallest menu; longer option lists scroll.
const MENU_MAX_HEIGHT: Pixels = px(320.);
/// The gap between the trigger and its menu.
const MENU_OFFSET: Pixels = px(4.);
/// The least distance between the menu and the window edge.
const MENU_MARGIN: Pixels = px(8.);
/// How far the menu rises into place as it fades in.
const MENU_RISE: Pixels = px(4.);

/// One choice of a `Dropdown`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SelectOption {
    value: String,
    label: SharedString,
    detail: Option<SharedString>,
    disabled: bool,
}

impl SelectOption {
    /// An option shown as its value.
    pub(crate) fn text(value: impl Into<String>) -> Self {
        let value = value.into();
        Self {
            label: value.clone().into(),
            value,
            detail: None,
            disabled: false,
        }
    }

    /// An option whose value is not what the user reads, such as an id.
    pub(crate) fn labelled(value: impl Into<String>, label: impl Into<SharedString>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            detail: None,
            disabled: false,
        }
    }

    /// A quiet note after the label, in the menu only.
    pub(crate) fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// A disabled option is shown but can be neither highlighted nor chosen.
    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub(crate) fn value(&self) -> &str {
        &self.value
    }

    /// The position of the option whose value is `value`.
    pub(crate) fn position(options: &[SelectOption], value: Option<&str>) -> Option<usize> {
        let value = value?;
        options.iter().position(|option| option.value == value)
    }
}

/// What a `Dropdown` reports to its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DropdownEvent {
    /// The user chose the option with this value.
    Confirm(String),
}

/// The highlight `delta` enabled options on from `from`, stopping at the
/// first and last enabled option. From no highlight, a step forward lands on
/// the first enabled option and a step back on the last.
fn step_highlight(options: &[SelectOption], from: Option<usize>, delta: isize) -> Option<usize> {
    let enabled = options
        .iter()
        .enumerate()
        .filter(|(_, option)| !option.disabled)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let last = enabled.len().checked_sub(1)?;
    let target = match from.and_then(|from| enabled.iter().position(|index| *index == from)) {
        Some(current) => current.saturating_add_signed(delta).min(last),
        None if delta < 0 => last,
        None => 0,
    };
    Some(enabled[target])
}

/// The highlight a menu opens with: the selection when it is enabled, else
/// the first enabled option.
fn opening_highlight(options: &[SelectOption], selected: Option<usize>) -> Option<usize> {
    selected
        .filter(|index| options.get(*index).is_some_and(|option| !option.disabled))
        .or_else(|| step_highlight(options, None, 1))
}

pub(crate) struct Dropdown {
    label: SharedString,
    placeholder: SharedString,
    options: Vec<SelectOption>,
    selected: Option<usize>,
    open: bool,
    highlighted: Option<usize>,
    trigger_focus: FocusHandle,
    menu_focus: FocusHandle,
    menu_scroll: ScrollHandle,
    /// Where the trigger painted last, in window coordinates; the menu opens
    /// below it, or above when it does not fit.
    trigger_bounds: Rc<Cell<Bounds<Pixels>>>,
}

impl EventEmitter<DropdownEvent> for Dropdown {}

impl Focusable for Dropdown {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.trigger_focus.clone()
    }
}

impl Dropdown {
    /// `label` names the control for assistive technology; `placeholder`
    /// shows while no option is selected.
    pub(crate) fn new(
        label: impl Into<SharedString>,
        placeholder: impl Into<SharedString>,
        options: Vec<SelectOption>,
        selected: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            label: label.into(),
            placeholder: placeholder.into(),
            selected: SelectOption::position(&options, selected),
            options,
            open: false,
            highlighted: None,
            trigger_focus: cx.focus_handle(),
            menu_focus: cx.focus_handle(),
            menu_scroll: ScrollHandle::new(),
            trigger_bounds: Rc::default(),
        }
    }

    /// Renames the select and its placeholder, e.g. after the locale
    /// changed.
    pub(crate) fn set_label(
        &mut self,
        label: impl Into<SharedString>,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.label = label.into();
        self.placeholder = placeholder.into();
        cx.notify();
    }

    /// The name assistive technology announces for the select and its menu.
    #[cfg(test)]
    pub(crate) fn accessibility_label(&self) -> &SharedString {
        &self.label
    }

    #[cfg(test)]
    pub(crate) fn selected_value(&self) -> Option<&str> {
        let option = self.options.get(self.selected?)?;
        Some(option.value.as_str())
    }

    #[cfg(test)]
    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    #[cfg(test)]
    pub(crate) fn highlighted_value(&self) -> Option<&str> {
        let option = self.options.get(self.highlighted?)?;
        Some(option.value.as_str())
    }

    /// Replaces the options and the selection, keeping an open menu open.
    pub(crate) fn set_options(
        &mut self,
        options: Vec<SelectOption>,
        selected: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let selected = SelectOption::position(&options, selected);
        if self.options == options && self.selected == selected {
            return;
        }
        let highlighted = self
            .highlighted
            .and_then(|index| self.options.get(index))
            .and_then(|previous| SelectOption::position(&options, Some(&previous.value)))
            .filter(|index| !options[*index].disabled);
        self.selected = selected;
        self.options = options;
        if self.open {
            let highlighted =
                highlighted.or_else(|| opening_highlight(&self.options, self.selected));
            if highlighted != self.highlighted {
                if let Some(index) = highlighted {
                    self.menu_scroll.scroll_to_item(index);
                }
            }
            self.highlighted = highlighted;
        }
        cx.notify();
    }

    /// Opens the menu with the selection highlighted. A select without
    /// options stays closed.
    pub(crate) fn open_menu(&mut self, cx: &mut Context<Self>) {
        if self.open || self.options.is_empty() {
            return;
        }
        self.open = true;
        self.highlighted = opening_highlight(&self.options, self.selected);
        if let Some(index) = self.highlighted {
            self.menu_scroll.scroll_to_item(index);
        }
        cx.notify();
    }

    pub(crate) fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.open = false;
            self.highlighted = None;
            cx.notify();
        }
    }

    fn move_highlight(&mut self, delta: isize, cx: &mut Context<Self>) {
        let highlighted = step_highlight(&self.options, self.highlighted, delta);
        if highlighted != self.highlighted {
            self.highlighted = highlighted;
            if let Some(index) = highlighted {
                self.menu_scroll.scroll_to_item(index);
            }
            cx.notify();
        }
    }

    fn hover(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.highlighted != Some(index) {
            self.highlighted = Some(index);
            cx.notify();
        }
    }

    /// Chooses the option at `index` when it is enabled and closes the menu.
    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(option) = self.options.get(index).filter(|option| !option.disabled) else {
            return;
        };
        let value = option.value.clone();
        self.selected = Some(index);
        self.close_menu(cx);
        cx.emit(DropdownEvent::Confirm(value));
        cx.notify();
    }

    /// Enter on an open menu: chooses the highlighted option, or only closes
    /// the menu when nothing is highlighted.
    fn confirm_highlighted(&mut self, cx: &mut Context<Self>) {
        match self.highlighted {
            Some(index) => self.choose(index, cx),
            None => self.close_menu(cx),
        }
    }

    fn render_trigger(&self, tokens: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.open;
        let entity = cx.entity();
        let menu_focus = self.menu_focus.clone();
        let bounds = self.trigger_bounds.clone();
        let (text, color) = match self.selected.and_then(|index| self.options.get(index)) {
            Some(option) => (option.label.clone(), tokens.text),
            None => (self.placeholder.clone(), tokens.text_muted),
        };
        div()
            .id("select-trigger")
            .size_full()
            .px(tokens.spacing.md)
            .flex()
            .items_center()
            .gap(tokens.spacing.sm)
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                // The root's own focus transfer would take focus back from
                // the menu.
                window.prevent_default();
                if open {
                    // The press outside the menu has already closed it.
                    return;
                }
                entity.update(cx, |this, cx| this.open_menu(cx));
                menu_focus.focus(window, cx);
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(color)
                    .child(theme::tail_faded_label(
                        "select-value",
                        text,
                        tokens.surface,
                    )),
            )
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(if open { tokens.text } else { tokens.text_muted })
                    .child(if open { "▴" } else { "▾" }),
            )
            // The trigger row fills the root inside its 1px border; the menu
            // aligns with the border's outer edge.
            .child(
                canvas(
                    move |painted, _, _| bounds.set(painted.dilate(px(1.))),
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
    }

    fn render_menu(&self, tokens: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let trigger = self.trigger_bounds.get();
        let rows = self
            .options
            .iter()
            .enumerate()
            .map(|(index, option)| self.render_row(index, option, tokens, cx))
            .collect::<Vec<_>>();
        let list = div()
            .id("select-menu")
            .debug_selector(|| "select-menu".into())
            .track_focus(&self.menu_focus)
            .role(Role::ListBox)
            .aria_label(self.label.clone())
            // The card's border sits outside the list.
            .w((trigger.size.width - px(2.)).max(px(0.)))
            .max_h(MENU_MAX_HEIGHT)
            .overflow_y_scroll()
            .track_scroll(&self.menu_scroll)
            .p(tokens.spacing.xs)
            .flex()
            .flex_col()
            .gap(px(2.))
            .text_sm()
            .text_color(tokens.text)
            .on_action(cx.listener(|this, _: &SelectUp, _, cx| this.move_highlight(-1, cx)))
            .on_action(cx.listener(|this, _: &SelectDown, _, cx| this.move_highlight(1, cx)))
            .children(rows);
        let entity = cx.entity();
        let trigger_focus = self.trigger_focus.clone();
        let menu = div()
            // Hitboxes are paint-order only; without occluding, a press on a
            // row would also reach what lies under the menu.
            .occlude()
            .on_mouse_down_out(move |_, window, cx| {
                entity.update(cx, |this, cx| this.close_menu(cx));
                trigger_focus.focus(window, cx);
            })
            .child(theme::frosted(
                tokens.radius.md,
                BlurLevel::Menu,
                theme::scroll_faded(&self.menu_scroll, tokens.float, list),
            ));
        deferred(
            Positioner::side(trigger)
                .placement(Placement::Bottom)
                .align(Align::Start)
                .offset(MENU_OFFSET)
                .margin(MENU_MARGIN)
                .child(entrance(
                    "select-menu-in",
                    menu,
                    tokens.motion.menu,
                    MENU_RISE,
                )),
        )
        .with_priority(1)
    }

    fn render_row(
        &self,
        index: usize,
        option: &SelectOption,
        tokens: &Tokens,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let selected = self.selected == Some(index);
        let highlighted = self.highlighted == Some(index);
        let mut row = div()
            .id(("select-option", index))
            .debug_selector(move || format!("select-option-{index}"))
            .role(Role::ListBoxOption)
            .aria_label(option.label.clone())
            .aria_selected(selected)
            .flex_none()
            .h(tokens.control_height.md)
            .px(tokens.spacing.sm)
            .rounded(tokens.radius.sm)
            .flex()
            .items_center()
            .gap(tokens.spacing.sm)
            .when(highlighted, |row| {
                row.bg(tokens.selection).aria_active_descendant()
            })
            .child(
                div()
                    .w(px(14.))
                    .flex_none()
                    .text_color(tokens.accent_text)
                    .child(if selected { "✓" } else { "" }),
            )
            .child(div().flex_1().min_w_0().child(theme::tail_faded_label(
                ("select-option-label", index),
                option.label.clone(),
                if highlighted {
                    tokens.selection
                } else {
                    tokens.float
                },
            )))
            .children(option.detail.clone().map(|detail| {
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(tokens.text_muted)
                    .child(detail)
            }));
        if option.disabled {
            row = row.text_color(tokens.text_subtle);
        } else {
            row = row
                .cursor_pointer()
                .on_mouse_move(cx.listener(move |this, _, _, cx| this.hover(index, cx)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.choose(index, cx);
                    this.trigger_focus.focus(window, cx);
                }));
        }
        row.into_any_element()
    }
}

impl Render for Dropdown {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = *theme::tokens(cx);
        let entity = cx.entity();
        let trigger_focus = self.trigger_focus.clone();
        let focused = self.trigger_focus.is_focused(window);
        let ring = keyboard_focused(&self.trigger_focus, window);
        let trigger = self.render_trigger(&tokens, cx);
        let menu = self.open.then(|| self.render_menu(&tokens, cx));
        let select = gpui_base::Select::new("select")
            .open(self.open)
            .focus_handle(&self.trigger_focus)
            .content_focus_handle(&self.menu_focus)
            .accessibility_label(self.label.clone())
            .on_open_change({
                let entity = entity.clone();
                move |open, _, cx| {
                    entity.update(cx, |this, cx| {
                        if open {
                            this.open_menu(cx)
                        } else {
                            this.close_menu(cx)
                        }
                    })
                }
            })
            .on_confirm(move |window, cx| {
                entity.update(cx, |this, cx| this.confirm_highlighted(cx));
                // The root focuses the menu after this handler returns; hand
                // focus back to the trigger once it has.
                let trigger_focus = trigger_focus.clone();
                window.defer(cx, move |window, cx| trigger_focus.focus(window, cx));
            })
            .relative()
            .w_full()
            .h(tokens.control_height.md)
            .rounded(tokens.radius.sm)
            .border_1()
            .border_color(if focused {
                tokens.accent
            } else {
                tokens.border
            })
            .bg(tokens.surface)
            .text_sm()
            .child(trigger)
            .children(menu);
        with_focus_ring(select, ring, &self.trigger_focus, &tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(disabled: &[bool]) -> Vec<SelectOption> {
        disabled
            .iter()
            .enumerate()
            .map(|(index, disabled)| SelectOption::text(format!("{index}")).disabled(*disabled))
            .collect()
    }

    #[test]
    fn the_highlight_steps_over_disabled_options_and_stops_at_the_ends() {
        let all_disabled = options(&[true, true]);
        let options = options(&[false, true, false, false, true]);
        assert_eq!(step_highlight(&options, Some(0), 1), Some(2));
        assert_eq!(step_highlight(&options, Some(2), 1), Some(3));
        assert_eq!(
            step_highlight(&options, Some(3), 1),
            Some(3),
            "stops at the last"
        );
        assert_eq!(step_highlight(&options, Some(2), -1), Some(0));
        assert_eq!(
            step_highlight(&options, Some(0), -1),
            Some(0),
            "stops at the first"
        );
        assert_eq!(step_highlight(&options, None, 1), Some(0));
        assert_eq!(step_highlight(&options, None, -1), Some(3));
        assert_eq!(step_highlight(&all_disabled, None, 1), None);
        assert_eq!(step_highlight(&[], Some(0), 1), None);
    }

    #[test]
    fn a_menu_opens_on_the_selection_or_the_first_enabled_option() {
        let options = options(&[true, false, false]);
        assert_eq!(opening_highlight(&options, Some(2)), Some(2));
        assert_eq!(
            opening_highlight(&options, Some(0)),
            Some(1),
            "disabled selection"
        );
        assert_eq!(opening_highlight(&options, None), Some(1));
        assert_eq!(opening_highlight(&options, Some(9)), Some(1));
    }

    #[gpui::test]
    fn refreshed_options_keep_the_keyboard_choice_and_update_install_details(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let menu = cx.new(|cx| {
                Dropdown::new(
                    "Model",
                    "Choose",
                    options(&[false, false, true]),
                    Some("0"),
                    cx,
                )
            });
            menu.update(cx, |menu, cx| {
                menu.open_menu(cx);
                menu.move_highlight(1, cx);
                assert_eq!(menu.highlighted_value(), Some("1"));
                let mut updated = options(&[false, false, false]);
                updated[2] = updated[2].clone().detail("Downloaded");
                updated.rotate_right(1);
                menu.set_options(updated.clone(), Some("0"), cx);
                assert!(menu.is_open());
                assert_eq!(menu.options[0].detail.as_deref(), Some("Downloaded"));
                assert_eq!(menu.selected_value(), Some("0"));
                assert_eq!(
                    menu.highlighted_value(),
                    Some("1"),
                    "keep the choice by value"
                );
                menu.set_options(updated, Some("0"), cx);
                assert_eq!(
                    menu.highlighted_value(),
                    Some("1"),
                    "an unchanged refresh is inert"
                );
                menu.set_options(options(&[false, true, false]), Some("0"), cx);
                assert_eq!(
                    menu.highlighted_value(),
                    Some("0"),
                    "skip a newly disabled choice"
                );
                menu.set_options(Vec::new(), None, cx);
                assert_eq!(menu.highlighted_value(), None);
                menu.confirm_highlighted(cx);
                assert!(!menu.is_open());
            });
        });
    }

    #[test]
    fn options_are_found_by_value() {
        let options = vec![SelectOption::text("自动"), SelectOption::text("CPU")];
        assert_eq!(SelectOption::position(&options, Some("CPU")), Some(1));
        assert_eq!(SelectOption::position(&options, Some("Metal")), None);
        assert_eq!(SelectOption::position(&options, None), None);
    }
}
